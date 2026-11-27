#![cfg(windows)]
use orkestr_client::Client;
use orkestr_protocol::*;
use orkestr_server::runtime;
use orkestr_storage::Store;
use uuid::Uuid;
use zeroize::Zeroizing;

#[test]
fn setup_saves_both_profiles_and_protects_the_password_under_the_server_identity() {
    let parent = tempfile::tempdir().unwrap();
    let data = parent.path().join("setup-server");
    runtime::init(&data, "127.0.0.1:0".parse().unwrap(), "localhost".into()).unwrap();
    let token = orkestr_client::load_token(&data.join("bootstrap-admin.dpapi")).unwrap();
    let ca = std::fs::read(data.join("server.crt")).unwrap();
    let (process, origin) = launch(&data);
    let client = Client::new(&origin, token, Some(&ca)).unwrap();
    let input = ConfigureConnection {
        id: Uuid::new_v4(),
        host: "Fixture.Example.test.".into(),
        locate_domain_controller: false,
        kind: ProfileKind::Execution,
        winrm_authentication: Some(WinRmAuthentication::Kerberos),
        account: ConnectionAccount::Password {
            username: "fixture@example.test".into(),
            password: Zeroizing::new("SYNTHETIC_SETUP_PASSWORD".into()),
        },
    };
    let response: ConfiguredConnection =
        client.post("/v1/connections/setup", &input, None).unwrap();
    assert_eq!(response.connection.host, "fixture.example.test");
    assert_eq!(response.connection.label, response.connection.host);
    assert_eq!(response.connection.credential_id, response.credential.id);
    let profiles: Vec<CredentialProfile> = client.get("/v1/credentials").unwrap();
    let connections: Vec<ConnectionProfile> = client.get("/v1/connections").unwrap();
    assert_eq!(profiles.len(), 1);
    assert_eq!(connections.len(), 1);
    assert_eq!(profiles[0].id, connections[0].credential_id);
    drop(client);
    drop(process);
    let store = Store::open_read_only(&data.join("orkestr.sqlite")).unwrap();
    let (profile, blob) = store.credential_secret(response.credential.id).unwrap();
    assert_eq!(
        orkestr_credentials::unprotect(&profile.owner_sid, &blob)
            .unwrap()
            .as_slice(),
        b"SYNTHETIC_SETUP_PASSWORD"
    );
}

#[test]
fn protected_artifact_stage_is_hash_addressed_and_idempotent() {
    let parent = tempfile::tempdir().unwrap();
    let data = parent.path().join("artifact-server");
    runtime::init(&data, "127.0.0.1:0".parse().unwrap(), "localhost".into()).unwrap();
    let source = parent.path().join("synthetic.msi");
    let data_bytes = b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1not-an-installable-fixture";
    std::fs::write(&source, data_bytes).unwrap();
    let hash = orkestr_server::deployment::stage_msi(&data, &source).unwrap();
    assert_eq!(hash.len(), 64);
    assert_eq!(
        std::fs::read(data.join("artifacts").join(format!("{hash}.msi"))).unwrap(),
        data_bytes
    );
    assert_eq!(
        orkestr_server::deployment::stage_msi(&data, &source).unwrap(),
        hash
    );
    assert_eq!(
        std::fs::read_dir(data.join("artifacts")).unwrap().count(),
        1
    );
    let exe_source = parent.path().join("synthetic.exe");
    let mut pe = vec![0u8; 90];
    pe[..2].copy_from_slice(b"MZ");
    pe[60..64].copy_from_slice(&64u32.to_le_bytes());
    pe[64..68].copy_from_slice(b"PE\0\0");
    pe[84..86].copy_from_slice(&2u16.to_le_bytes());
    pe[86..88].copy_from_slice(&2u16.to_le_bytes());
    pe[88..90].copy_from_slice(&0x10bu16.to_le_bytes());
    std::fs::write(&exe_source, &pe).unwrap();
    let exe_hash = orkestr_server::deployment::stage_exe(&data, &exe_source).unwrap();
    let staged = data.join("artifacts").join(format!("{exe_hash}.exe"));
    assert_eq!(std::fs::read(&staged).unwrap(), pe);
    assert_eq!(
        orkestr_server::deployment::stage_exe(&data, &exe_source).unwrap(),
        exe_hash
    );
    // A pre-existing file with a hash-addressed name is verified, never silently reused.
    pe.push(1);
    std::fs::write(&staged, &pe).unwrap();
    assert_eq!(
        orkestr_server::deployment::stage_exe(&data, &exe_source),
        Err("ARTIFACT_HASH_MISMATCH")
    );
    assert_eq!(
        orkestr_server::deployment::stage_exe(&data, &source),
        Err("LOCAL_EXE_REQUIRED")
    );
    let (_, _, _) = runtime::startup(&data).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn actual_https_trust_name_two_clients_and_dpapi_restart() {
    let parent = tempfile::tempdir().unwrap();
    let data = parent.path().join("protected-server");
    let copy = data.clone();
    tokio::task::spawn_blocking(move || {
        runtime::init(&copy, "127.0.0.1:0".parse().unwrap(), "localhost".into())
    })
    .await
    .unwrap()
    .unwrap();
    let data_copy = data.clone();
    let handle = axum_server::Handle::new();
    let server_handle = handle.clone();
    let task = tokio::spawn(async move { runtime::serve(&data_copy, server_handle).await });
    let address = tokio::time::timeout(std::time::Duration::from_secs(25), handle.listening())
        .await
        .unwrap()
        .unwrap();
    let data_copy = data.clone();
    let credential_id = tokio::task::spawn_blocking(move || {
        let token = orkestr_client::load_token(&data_copy.join("bootstrap-admin.dpapi")).unwrap();
        let ca = std::fs::read(data_copy.join("server.crt")).unwrap();
        let origin = format!("https://localhost:{}", address.port());
        let no_ca = Client::new(&origin, Zeroizing::new(token.to_string()), None).unwrap();
        assert!(no_ca.get::<Health>("/v1/health").is_err());
        let wrong_name = Client::new(
            &format!("https://127.0.0.1:{}", address.port()),
            Zeroizing::new(token.to_string()),
            Some(&ca),
        )
        .unwrap();
        assert!(wrong_name.get::<Health>("/v1/health").is_err());
        let a = Client::new(&origin, Zeroizing::new(token.to_string()), Some(&ca)).unwrap();
        let second: IssuedToken = a
            .post(
                "/v1/operators",
                &IssueToken {
                    label: "Second console fixture".into(),
                    role: Role::Operator,
                    days: 1,
                },
                None,
            )
            .unwrap();
        let b = Client::new(&origin, second.token, Some(&ca)).unwrap();
        assert_eq!(a.get::<Health>("/v1/health").unwrap().status, "READY");
        let id = Uuid::new_v4();
        let credential: CredentialProfile = a
            .post(
                "/v1/credentials",
                &SaveCredential {
                    id,
                    expected: None,
                    label: "Synthetic only".into(),
                    username: "FIXTURE\\user".into(),
                    kind: ProfileKind::AdRead,
                    target: "fixture.invalid".into(),
                    password: Zeroizing::new("SYNTHETIC_SECRET_ONLY".into()),
                },
                None,
            )
            .unwrap();
        assert_eq!(credential.revision, 1);
        let metadata: Vec<CredentialProfile> = a.get("/v1/credentials").unwrap();
        assert_eq!(metadata[0].id, id);
        // Exercise the very same bounded Command/Event transport used by native consoles.
        let service_a = a.service(std::sync::Arc::new(|| {}));
        let service_b = b.service(std::sync::Arc::new(|| {}));
        let wait = |service: &orkestr_application::Service| {
            let start = std::time::Instant::now();
            loop {
                if let Some(event) = service.try_event() {
                    break event;
                }
                assert!(start.elapsed() < std::time::Duration::from_secs(15));
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        };
        assert!(matches!(
            wait(&service_a),
            orkestr_application::Event::Loaded(_)
        ));
        assert!(matches!(
            wait(&service_b),
            orkestr_application::Event::Loaded(_)
        ));
        let collection = orkestr_domain::Collection {
            id: orkestr_domain::CollectionId::new(),
            parent_id: None,
            name: "Shared over HTTPS".into(),
            revision: 0,
            kind: orkestr_domain::CollectionKind::Folder,
        };
        service_a
            .submit(orkestr_application::Command::Save {
                collection: Box::new(collection.clone()),
                expected: None,
            })
            .unwrap();
        assert!(matches!(
            wait(&service_a),
            orkestr_application::Event::Saved { .. }
        ));
        service_b
            .submit(orkestr_application::Command::Refresh)
            .unwrap();
        match wait(&service_b) {
            orkestr_application::Event::Loaded(w) => {
                assert_eq!(w.inventory.collections[0].id, collection.id)
            }
            _ => panic!("missing shared workspace"),
        }
        service_b
            .submit(orkestr_application::Command::Save {
                collection: Box::new(collection),
                expected: None,
            })
            .unwrap();
        match wait(&service_b) {
            orkestr_application::Event::Error { message, .. } => assert_eq!(message, "CONFLICT"),
            _ => panic!("stale edit accepted"),
        }
        let input_file=data_copy.join("synthetic-discovery.csv");
        std::fs::write(&input_file,"host,name,site\nhttps-fixture,Учебный ПК,lab").unwrap();
        service_a.submit(orkestr_application::Command::ReadDiscoveryFile{token:40,path:input_file}).unwrap();
        let text=match wait(&service_a) {orkestr_application::Event::DiscoveryText{token:40,text}=>text,_=>panic!("client file load failed")};
        service_a.submit(orkestr_application::Command::PrepareDiscovery{token:41,input:PrepareDiscovery{format:DiscoverySource::Csv,text}}).unwrap();
        let prepared=match wait(&service_a) {orkestr_application::Event::DiscoveryPrepared{token:41,preview}=>preview,_=>panic!("HTTPS discovery prepare failed")};
        let apply=ApplyDiscovery{preview_id:prepared.id,confirmed:true};
        service_a.submit(orkestr_application::Command::ApplyDiscovery{token:41,input:apply.clone()}).unwrap();
        let receipt=match wait(&service_a) {orkestr_application::Event::DiscoveryApplied{token:41,result}=>result,_=>panic!("HTTPS discovery apply failed")};
        service_a.submit(orkestr_application::Command::ApplyDiscovery{token:41,input:apply}).unwrap();
        match wait(&service_a) {orkestr_application::Event::DiscoveryApplied{token:41,result}=>assert_eq!(result,receipt),_=>panic!("HTTPS discovery replay failed")};
        service_b.submit(orkestr_application::Command::Refresh).unwrap();
        let device=match wait(&service_b) {orkestr_application::Event::Loaded(w)=> {assert_eq!(w.inventory.devices.len(),1);w.inventory.devices[0].device.id},_=>panic!("second client did not refresh")};
        service_b.submit(orkestr_application::Command::InventoryStatus{token:42,device_id:device}).unwrap();
        assert!(matches!(wait(&service_b),orkestr_application::Event::InventoryStatus{token:42,status} if status.latest.is_none()));
        service_b.submit(orkestr_application::Command::PrepareDiscovery{token:43,input:PrepareDiscovery{format:DiscoverySource::Manual,text:"denied-fixture".into()}}).unwrap();
        assert!(matches!(wait(&service_b),orkestr_application::Event::Error{token:Some(43),message,..} if message=="FORBIDDEN"));
        // Use a separate management transport; a native console has the same separation.
        let token = orkestr_client::load_token(&data_copy.join("bootstrap-admin.dpapi")).unwrap();
        let a = Client::new(&origin, token, Some(&ca)).unwrap();
        let issued: IssuedToken = a
            .post(
                "/v1/operators",
                &IssueToken {
                    label: "Fixture Viewer".into(),
                    role: Role::Viewer,
                    days: 1,
                },
                None,
            )
            .unwrap();
        let viewer = Client::new(&origin, issued.token, Some(&ca)).unwrap();
        assert_eq!(
            viewer
                .post::<_, Job>(
                    "/v1/jobs",
                    &TestConnection {
                        connection_id: Uuid::new_v4(),
                        expected_revision: None,
                    },
                    Some("denied")
                )
                .unwrap_err(),
            "FORBIDDEN"
        );
        id
    })
    .await
    .unwrap();
    handle.shutdown();
    task.await.unwrap().unwrap();
    let data_copy = data.clone();
    tokio::task::spawn_blocking(move || {
        let (_, api, _receiver) = runtime::startup(&data_copy).unwrap();
        let store = api.store.lock().unwrap();
        let (p, blob) = store.credential_secret(credential_id).unwrap();
        assert_eq!(
            orkestr_credentials::unprotect(&p.owner_sid, &blob)
                .unwrap()
                .as_slice(),
            b"SYNTHETIC_SECRET_ONLY"
        );
        assert!(!serde_json::to_string(&store.events(0).unwrap())
            .unwrap()
            .contains("SYNTHETIC_SECRET_ONLY"));
    })
    .await
    .unwrap();
    // A persisted wrong owner must block startup, not clear the vault or start an empty DB.
    let data_copy = data.clone();
    tokio::task::spawn_blocking(move || {
        assert_persisted_owner_mismatch_blocks(&data_copy);
    })
    .await
    .unwrap();
}

fn assert_persisted_owner_mismatch_blocks(data: &std::path::Path) {
    let mut store = Store::open(&data.join("orkestr.sqlite")).unwrap();
    let existing = store.credentials().unwrap();
    let mut p = existing[0].clone();
    let (sid, _) = store.server_identity().unwrap().unwrap();
    let admin = store
        .operators()
        .unwrap()
        .into_iter()
        .find(|a| a.role == Role::Admin)
        .unwrap();
    let (_, blob) = store.credential_secret(p.id).unwrap();
    p.owner_sid = "S-1-wrong-identity".into();
    store
        .save_credential(&admin, &p, Some(p.revision), &blob)
        .unwrap();
    drop(store);
    let error = match runtime::startup(data) {
        Ok(_) => panic!("wrong identity accepted"),
        Err(e) => e,
    };
    assert!(error.contains("CREDENTIAL_IDENTITY_MISMATCH"));
    let store = Store::open(&data.join("orkestr.sqlite")).unwrap();
    assert_eq!(store.server_identity().unwrap().unwrap().0, sid);
}

struct ServerProcess(std::process::Child);
impl Drop for ServerProcess {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn launch(data: &std::path::Path) -> (ServerProcess, String) {
    use std::io::BufRead;
    let mut child = ServerProcess(
        std::process::Command::new(env!("CARGO_BIN_EXE_orkestr-server"))
            .args(["run", "--data"])
            .arg(data)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap(),
    );
    let stdout = child.0.stdout.take().unwrap();
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let mut line = String::new();
        let _ = std::io::BufReader::new(stdout).read_line(&mut line);
        let _ = sender.send(line);
    });
    let line = receiver
        .recv_timeout(std::time::Duration::from_secs(25))
        .unwrap();
    let address = line
        .trim()
        .strip_prefix("READY https://")
        .unwrap()
        .parse::<std::net::SocketAddr>()
        .unwrap();
    (child, format!("https://localhost:{}", address.port()))
}
#[test]
fn actual_process_restart_retains_dpapi_and_single_instance_lock() {
    let parent = tempfile::tempdir().unwrap();
    let data = parent.path().join("process-server");
    runtime::init(&data, "127.0.0.1:0".parse().unwrap(), "localhost".into()).unwrap();
    let token = orkestr_client::load_token(&data.join("bootstrap-admin.dpapi")).unwrap();
    let ca = std::fs::read(data.join("server.crt")).unwrap();
    let (process, origin) = launch(&data);
    let client = Client::new(&origin, Zeroizing::new(token.to_string()), Some(&ca)).unwrap();
    let id = Uuid::new_v4();
    let _: CredentialProfile = client
        .post(
            "/v1/credentials",
            &SaveCredential {
                id,
                expected: None,
                label: "Process restart fixture".into(),
                username: "FIXTURE\\read".into(),
                kind: ProfileKind::AdRead,
                target: "fixture.invalid".into(),
                password: Zeroizing::new("SYNTHETIC_PROCESS_SECRET".into()),
            },
            None,
        )
        .unwrap();
    assert!(runtime::startup(&data).is_err());
    let prepared: PreparedDiscovery = client
        .post(
            "/v1/discovery/preview",
            &PrepareDiscovery {
                format: DiscoverySource::Manual,
                text: "process-discovery-fixture".into(),
            },
            None,
        )
        .unwrap();
    let apply = ApplyDiscovery {
        preview_id: prepared.id,
        confirmed: true,
    };
    let receipt: AppliedDiscovery = client.post("/v1/discovery/apply", &apply, None).unwrap();
    drop(client);
    drop(process); // abrupt termination, including WAL recovery on next start
    let (process, origin) = launch(&data);
    let client = Client::new(&origin, token, Some(&ca)).unwrap();
    assert_eq!(client.get::<Health>("/v1/health").unwrap().status, "READY");
    let metadata: Vec<CredentialProfile> = client.get("/v1/credentials").unwrap();
    assert_eq!(metadata[0].id, id);
    // Simulate a lost apply reply: replay the old preview ID against a fresh process.
    assert_eq!(
        client
            .post::<_, AppliedDiscovery>("/v1/discovery/apply", &apply, None)
            .unwrap(),
        receipt
    );
    let w: orkestr_application::Workspace = client.get("/v1/workspace").unwrap();
    assert_eq!(w.inventory.devices.len(), 1);
    assert_eq!(
        w.inventory.devices[0].device.host,
        "process-discovery-fixture"
    );
    drop(client);
    drop(process);
    let store = Store::open(&data.join("orkestr.sqlite")).unwrap();
    let (p, blob) = store.credential_secret(id).unwrap();
    assert_eq!(
        orkestr_credentials::unprotect(&p.owner_sid, &blob)
            .unwrap()
            .as_slice(),
        b"SYNTHETIC_PROCESS_SECRET"
    );
}
