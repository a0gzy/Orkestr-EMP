use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use chrono::Utc;
use orkestr_domain::*;
use orkestr_protocol::*;
use orkestr_server::{
    api::{self, Api},
    hash_token, new_token,
};
use orkestr_storage::Store;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use tokio::sync::{mpsc, Semaphore};
use tower::ServiceExt;
use uuid::Uuid;

struct Fixture {
    _dir: tempfile::TempDir,
    api: Api,
    app: Router,
    admin: IssuedToken,
    operator: IssuedToken,
    viewer: IssuedToken,
    _receiver: mpsc::Receiver<Uuid>,
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(&dir.path().join("test.sqlite")).unwrap();
        let admin = new_token("A".into(), Role::Admin, 1).unwrap();
        let operator = new_token("O".into(), Role::Operator, 1).unwrap();
        let viewer = new_token("V".into(), Role::Viewer, 1).unwrap();
        store
            .bootstrap_server(
                "synthetic-owner",
                b"test-blob",
                &admin.actor,
                &hash_token(&admin.token),
            )
            .unwrap();
        for token in [&operator, &viewer] {
            store
                .issue_token(&admin.actor, &token.actor, &hash_token(&token.token))
                .unwrap();
        }
        let (jobs, receiver) = mpsc::channel(8);
        let api = Api {
            store: Arc::new(Mutex::new(store)),
            jobs,
            sid: "synthetic-owner".into(),
            requests: Arc::new(Semaphore::new(16)),
            rate: Arc::new(Mutex::new((std::time::Instant::now(), 0))),
            instance_guard: None,
            artifact_directory: Some(dir.path().join("artifacts")),
            stopping: Arc::default(),
        };
        Self {
            _dir: dir,
            app: api::router(api.clone()),
            api,
            admin,
            operator,
            viewer,
            _receiver: receiver,
        }
    }
    fn seed_connection(&self) -> ConnectionProfile {
        self.seed_connection_with_kind(ProfileKind::Execution)
    }
    fn seed_connection_with_kind(&self, kind: ProfileKind) -> ConnectionProfile {
        let mut store = self.api.store.lock().unwrap();
        let id = Uuid::new_v4();
        store
            .save_credential(
                &self.admin.actor,
                &CredentialProfile {
                    id,
                    revision: 0,
                    label: "Fixture".into(),
                    username: "FIXTURE\\user".into(),
                    kind,
                    target: "fixture.invalid".into(),
                    owner_sid: "synthetic-owner".into(),
                    updated_at: Utc::now(),
                },
                None,
                b"ciphertext-fixture",
            )
            .unwrap();
        store
            .save_connection(
                &self.admin.actor,
                &ConnectionProfile {
                    id: Uuid::new_v4(),
                    revision: 0,
                    label: "Fixture".into(),
                    host: "fixture.invalid".into(),
                    kind,
                    port: if kind == ProfileKind::AdRead {
                        636
                    } else {
                        5986
                    },
                    credential_id: id,
                    winrm_authentication: None,
                },
                None,
            )
            .unwrap()
    }

    fn seed_device(&self, host: &str) -> DeviceId {
        let mut store = self.api.store.lock().unwrap();
        let preview = store
            .preview_discovery(&orkestr_inventory::DiscoveryBatch::manual(host).unwrap())
            .unwrap();
        let id = preview.writes()[0].id;
        store.apply_discovery(&preview).unwrap();
        id
    }
}
async fn call(
    app: &Router,
    method: &str,
    path: &str,
    token: &str,
    body: Value,
    key: Option<&str>,
) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("Authorization", format!("Bearer {token}"))
        .header("Content-Type", "application/json");
    if let Some(key) = key {
        request = request.header("Idempotency-Key", key);
    }
    let response = app
        .clone()
        .oneshot(request.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 10 * 1024 * 1024)
        .await
        .unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

#[tokio::test]
async fn inventory_jobs_are_scoped_to_the_saved_profile_and_exact_device() {
    let f = Fixture::new();
    let profile = f.seed_connection();
    let device = f.seed_device(&profile.host);
    let other = f.seed_device("other.invalid");
    let path = "/v1/inventory/jobs";
    let mut request = json!({"connection_id":profile.id,"expected_revision":profile.revision,
        "task":{"ScanDevice":{"device_id":device}}});
    assert_eq!(
        call(
            &f.app,
            "POST",
            path,
            &f.operator.token,
            request.clone(),
            Some("scan")
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let (status, queued) = call(
        &f.app,
        "POST",
        path,
        &f.admin.token,
        request.clone(),
        Some("scan"),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(queued["state"], "Queued");
    let (_, replay) = call(
        &f.app,
        "POST",
        path,
        &f.admin.token,
        request.clone(),
        Some("scan"),
    )
    .await;
    assert_eq!(replay["id"], queued["id"]);
    request["task"]["ScanDevice"]["device_id"] = json!(other);
    assert_eq!(
        call(
            &f.app,
            "POST",
            path,
            &f.admin.token,
            request.clone(),
            Some("scan-other")
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    request["task"] = json!({"DiscoverOu":{"base_dn":"OU=Lab,DC=example,DC=test"}});
    assert_eq!(
        call(
            &f.app,
            "POST",
            path,
            &f.admin.token,
            request,
            Some("ad-with-execution")
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
}

#[tokio::test]
async fn setup_reuses_domain_credentials_for_explicit_registered_devices_in_the_same_domain() {
    let f = Fixture::new();
    let existing = f.seed_connection_with_kind(ProfileKind::AdRead);
    f.seed_device("manual.invalid");
    f.seed_device("other.example");
    {
        let mut store = f.api.store.lock().unwrap();
        let (mut profile, ciphertext) = store.credential_secret(existing.credential_id).unwrap();
        profile.username = "inventory@invalid".into();
        store
            .save_credential(
                &f.admin.actor,
                &profile,
                Some(profile.revision),
                &ciphertext,
            )
            .unwrap();
        let batch = orkestr_inventory::DiscoveryBatch::directory(vec![DirectoryComputer {
            guid: Uuid::new_v4(),
            host: "imported.invalid".into(),
            name: "Imported".into(),
            ou: "OU=Lab,DC=invalid".into(),
            os: String::new(),
            disabled: false,
        }])
        .unwrap();
        let preview = store.preview_discovery(&batch).unwrap();
        store.apply_discovery(&preview).unwrap();
    }
    let endpoint = "/v1/connections/setup";
    let input = json!({"id":Uuid::new_v4(),"host":"imported.invalid","kind":"Execution","winrm_authentication":"Kerberos",
        "account":{"source":"DomainAccount","credential_id":existing.credential_id,"expected_revision":2}});
    assert_eq!(
        call(
            &f.app,
            "POST",
            endpoint,
            &f.operator.token,
            input.clone(),
            None
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let mut manual = input.clone();
    manual["id"] = json!(Uuid::new_v4());
    manual["host"] = json!("manual.invalid");
    let (status, saved_manual) = call(&f.app, "POST", endpoint, &f.admin.token, manual, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(saved_manual["connection"]["host"], "manual.invalid");
    for host in ["other.example", "unregistered.invalid"] {
        let mut outside = input.clone();
        outside["host"] = json!(host);
        let (status, denied) = call(&f.app, "POST", endpoint, &f.admin.token, outside, None).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(denied["code"], "DOMAIN_ACCOUNT_SCOPE_MISMATCH");
    }
    let mut stale = input.clone();
    stale["account"]["expected_revision"] = json!(1);
    assert_eq!(
        call(&f.app, "POST", endpoint, &f.admin.token, stale, None)
            .await
            .0,
        StatusCode::CONFLICT
    );
    let (status, saved) = call(
        &f.app,
        "POST",
        endpoint,
        &f.admin.token,
        input.clone(),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(saved["connection"]["host"], "imported.invalid");
    assert_eq!(saved["connection"]["winrm_authentication"], "Kerberos");
    assert_eq!(
        saved["credential"]["id"],
        saved["connection"]["credential_id"]
    );
    assert_ne!(saved["credential"]["id"], json!(existing.credential_id));
    assert!(!saved.to_string().contains("ciphertext-fixture"));
    assert_eq!(
        call(&f.app, "POST", endpoint, &f.admin.token, input, None)
            .await
            .0,
        StatusCode::CONFLICT
    );
    let store = f.api.store.lock().unwrap();
    assert_eq!(store.connections().unwrap().len(), 3);
    assert_eq!(store.credentials().unwrap().len(), 3);
}

#[tokio::test]
async fn setup_uses_the_existing_credential_and_matching_connection_for_the_same_target() {
    let f = Fixture::new();
    let existing = f.seed_connection();
    let endpoint = "/v1/connections/setup";
    let mut input = json!({"id":Uuid::new_v4(),"host":existing.host,"kind":"Execution","winrm_authentication":null,
        "account":{"source":"SavedTarget","credential_id":existing.credential_id,"expected_revision":1}});
    let (status, configured) = call(
        &f.app,
        "POST",
        endpoint,
        &f.admin.token,
        input.clone(),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(configured["connection"]["id"], json!(existing.id));
    assert_eq!(
        configured["credential"]["id"],
        json!(existing.credential_id)
    );
    input["winrm_authentication"] = json!("Kerberos");
    let (status, configured) = call(
        &f.app,
        "POST",
        endpoint,
        &f.admin.token,
        input.clone(),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        configured["connection"]["credential_id"],
        json!(existing.credential_id)
    );
    assert_ne!(configured["connection"]["id"], json!(existing.id));
    input["host"] = json!("other.invalid");
    let (status, denied) = call(&f.app, "POST", endpoint, &f.admin.token, input, None).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(denied["code"], "CREDENTIAL_SCOPE_MISMATCH");
    let store = f.api.store.lock().unwrap();
    assert_eq!(store.credentials().unwrap().len(), 1);
    assert_eq!(store.connections().unwrap().len(), 2);
}

#[tokio::test]
async fn deployment_requires_approval_and_keeps_the_device_locked_after_unknown_restart() {
    for execution in [
        None,
        Some(json!({"kind":"Exe", "arguments":"/S /norestart"})),
    ] {
        let f = Fixture::new();
        let profile = f.seed_connection();
        let device_id = f.seed_device(&profile.host);
        let hash = "a".repeat(64);
        let artifacts = f.api.artifact_directory.as_ref().unwrap();
        std::fs::create_dir(artifacts).unwrap();
        std::fs::write(
            artifacts.join(format!(
                "{hash}.{}",
                if execution.is_some() { "exe" } else { "msi" }
            )),
            b"synthetic, never executed",
        )
        .unwrap();
        let mut manifest =
            json!({"name":"Lab fixture","version":"1","sha256":hash,"product_code":Uuid::new_v4()});
        if let Some(execution) = &execution {
            manifest["execution"] = execution.clone();
        }
        let (status, package) = call(
            &f.app,
            "POST",
            "/v1/deploy/revisions",
            &f.admin.token,
            manifest,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let plan = json!({"device_id":device_id,"package_revision_id":package["id"],"connection_id":profile.id,"connection_revision":profile.revision});
        let mut records = Vec::new();
        for key in ["first", "second"] {
            let (status, record) = call(
                &f.app,
                "POST",
                "/v1/deployments",
                &f.admin.token,
                plan.clone(),
                Some(key),
            )
            .await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(record["state"], "WaitingApproval");
            records.push(record);
        }
        let (_, replay) = call(
            &f.app,
            "POST",
            "/v1/deployments",
            &f.admin.token,
            plan,
            Some("first"),
        )
        .await;
        assert_eq!(replay["id"], records[0]["id"]);
        let first_id = records[0]["id"].as_str().unwrap();
        let second_id = records[1]["id"].as_str().unwrap();
        let approve_first = format!("/v1/deployments/{first_id}/approve");
        let approve_second = format!("/v1/deployments/{second_id}/approve");
        let expected = json!({"expected_revision":1});
        assert_eq!(
            call(
                &f.app,
                "POST",
                &approve_first,
                &f.operator.token,
                expected.clone(),
                None
            )
            .await
            .0,
            StatusCode::FORBIDDEN
        );
        let (status, approved) = call(
            &f.app,
            "POST",
            &approve_first,
            &f.admin.token,
            expected.clone(),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(approved["state"], "Queued");
        let (status, denied) = call(
            &f.app,
            "POST",
            &approve_second,
            &f.admin.token,
            expected.clone(),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(denied["code"], "DEPLOYMENT_DEVICE_BUSY");
        let recovery_revision = {
            let mut store = f.api.store.lock().unwrap();
            let id = Uuid::parse_str(first_id).unwrap();
            let mut record = store.deployment(id).unwrap().unwrap();
            record.state = DeploymentState::Running;
            store
                .update_deployment(&mut record, f.admin.actor.id)
                .unwrap();
            store.recover_deployments().unwrap();
            let recovered = store.deployment(id).unwrap().unwrap();
            assert_eq!(recovered.state, DeploymentState::NeedsReconciliation);
            recovered.revision
        };
        let (status, denied) = call(
            &f.app,
            "POST",
            &approve_second,
            &f.admin.token,
            expected,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(denied["code"], "DEPLOYMENT_DEVICE_BUSY");
        let (_, reconciled) = call(
            &f.app,
            "POST",
            &format!("/v1/deployments/{first_id}/reconcile"),
            &f.admin.token,
            json!({"expected_revision":recovery_revision}),
            None,
        )
        .await;
        assert_eq!(reconciled["state"], "ReconcileQueued");
    }
}

#[tokio::test]
async fn exe_registration_requires_explicit_arguments_exact_artifact_type_and_admin() {
    let f = Fixture::new();
    let artifacts = f.api.artifact_directory.as_ref().unwrap();
    std::fs::create_dir(artifacts).unwrap();
    let hash = "b".repeat(64);
    std::fs::write(artifacts.join(format!("{hash}.msi")), b"fixture").unwrap();
    let mut manifest = json!({"name":"EXE fixture", "version":"1", "sha256":hash,
        "product_code":Uuid::new_v4(), "execution":{"kind":"Exe", "arguments":"/S"}});
    for token in [&f.viewer.token, &f.operator.token] {
        assert_eq!(
            call(
                &f.app,
                "POST",
                "/v1/deploy/revisions",
                token,
                manifest.clone(),
                None
            )
            .await
            .0,
            StatusCode::FORBIDDEN
        );
    }
    let (status, error) = call(
        &f.app,
        "POST",
        "/v1/deploy/revisions",
        &f.admin.token,
        manifest.clone(),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(error["code"], "ARTIFACT_UNAVAILABLE");
    std::fs::write(artifacts.join(format!("{hash}.exe")), b"fixture").unwrap();
    for arguments in [
        "".into(),
        "\t".into(),
        "/S\r\ncommand".into(),
        "x".repeat(2049),
    ] {
        manifest["execution"]["arguments"] = json!(arguments);
        let (status, error) = call(
            &f.app,
            "POST",
            "/v1/deploy/revisions",
            &f.admin.token,
            manifest.clone(),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(error["code"], "INVALID_INSTALLER_MANIFEST");
    }
    manifest["execution"]["arguments"] = json!("/S");
    let (status, first) = call(
        &f.app,
        "POST",
        "/v1/deploy/revisions",
        &f.admin.token,
        manifest.clone(),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    manifest["execution"]["arguments"] = json!("/quiet /norestart");
    let (status, second) = call(
        &f.app,
        "POST",
        "/v1/deploy/revisions",
        &f.admin.token,
        manifest,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_ne!(first["id"], second["id"]);
    let first_id: PackageRevisionId = serde_json::from_value(first["id"].clone()).unwrap();
    let store = f.api.store.lock().unwrap();
    assert_eq!(
        store
            .managed_installer(first_id)
            .unwrap()
            .unwrap()
            .execution,
        InstallerExecution::Exe {
            arguments: "/S".into()
        }
    );
}
fn collection() -> Collection {
    Collection {
        id: CollectionId::new(),
        parent_id: None,
        name: "Shared".into(),
        revision: 0,
        kind: CollectionKind::Folder,
    }
}
#[tokio::test]
async fn viewer_denied_before_body_and_no_unknown_command_surface() {
    let f = Fixture::new();
    for path in [
        "/v1/jobs",
        "/v1/credentials",
        "/v1/connections",
        "/v1/connections/winrm",
        "/v1/operators",
        "/v1/collections",
        "/v1/collections/delete",
        "/v1/discovery/preview",
        "/v1/discovery/apply",
        "/v1/discovery/discard",
    ] {
        let response = f
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(path)
                    .header("Authorization", format!("Bearer {}", *f.viewer.token))
                    .body(Body::from("x".repeat(API_BODY_LIMIT + 1)))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "{path}");
    }
    assert_eq!(
        call(
            &f.app,
            "POST",
            "/v1/command",
            &f.admin.token,
            json!({"command":"anything"}),
            None
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(
            &f.app,
            "GET",
            "/v1/workspace",
            &"0".repeat(64),
            Value::Null,
            None
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(
            &f.app,
            "GET",
            "/v1/workspace",
            &f.viewer.token,
            Value::Null,
            None
        )
        .await
        .0,
        StatusCode::OK
    );
}
#[tokio::test]
async fn winrm_authentication_survives_legacy_edits_and_requires_explicit_changes() {
    let f = Fixture::new();
    let legacy = f.seed_connection();
    let legacy_path = "/v1/connections";
    let winrm_path = "/v1/connections/winrm";
    let mut input = json!({"profile":legacy,"expected":legacy.revision});
    assert!(input["profile"].get("winrm_authentication").is_none());

    input["profile"]["winrm_authentication"] = json!("Kerberos");
    let (status, error) = call(
        &f.app,
        "POST",
        legacy_path,
        &f.admin.token,
        input.clone(),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(error["code"], "INVALID_WINRM_PROFILE");
    let (status, saved) = call(
        &f.app,
        "POST",
        winrm_path,
        &f.admin.token,
        input.clone(),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(saved["winrm_authentication"], "Kerberos");
    assert_eq!(saved["revision"], 2);

    let mut legacy_edit = legacy.clone();
    legacy_edit.label = "Edited by an M2 console".into();
    let (status, saved) = call(
        &f.app,
        "POST",
        legacy_path,
        &f.admin.token,
        json!({"profile":legacy_edit,"expected":2}),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(saved["winrm_authentication"], "Kerberos");
    assert_eq!(saved["label"], "Edited by an M2 console");

    let job = json!({"connection_id":legacy.id,"expected_revision":3});
    let (status, queued) = call(
        &f.app,
        "POST",
        "/v1/jobs",
        &f.operator.token,
        job.clone(),
        Some("winrm-test"),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);

    input["expected"] = json!(2);
    input["profile"]["winrm_authentication"] = json!("Negotiate");
    assert_eq!(
        call(
            &f.app,
            "POST",
            winrm_path,
            &f.admin.token,
            input.clone(),
            None
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    input["expected"] = json!(3);
    let (status, saved) = call(&f.app, "POST", winrm_path, &f.admin.token, input, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(saved["winrm_authentication"], "Negotiate");

    let (status, retried) = call(
        &f.app,
        "POST",
        "/v1/jobs",
        &f.operator.token,
        job.clone(),
        Some("winrm-test"),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(retried["id"], queued["id"]);
    let (status, error) = call(
        &f.app,
        "POST",
        "/v1/jobs",
        &f.operator.token,
        job,
        Some("stale-winrm-test"),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(error["code"], "CONFLICT");
    let (status, error) = call(
        &f.app,
        "POST",
        "/v1/jobs",
        &f.operator.token,
        json!({"connection_id":legacy.id,"expected_revision":4}),
        Some("winrm-test"),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(error["code"], "IDEMPOTENCY_CONFLICT");
}

#[tokio::test]
async fn winrm_profiles_reject_invalid_modes_and_still_require_https_dns_and_scope() {
    let f = Fixture::new();
    let legacy = f.seed_connection();
    let path = "/v1/connections/winrm";
    let mut input = json!({"profile":legacy,"expected":legacy.revision});
    let (status, _) = call(&f.app, "POST", path, &f.admin.token, input.clone(), None).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    input["profile"]["winrm_authentication"] = json!("Basic");
    let (status, _) = call(&f.app, "POST", path, &f.admin.token, input.clone(), None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    input["profile"]["winrm_authentication"] = json!("Kerberos");
    for (field, value) in [
        ("kind", json!("AdRead")),
        ("port", json!(5985)),
        ("host", json!("192.0.2.1")),
        ("host", json!("other.invalid")),
    ] {
        let mut invalid = input.clone();
        invalid["profile"][field] = value;
        let (status, _) = call(&f.app, "POST", path, &f.admin.token, invalid, None).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    }
    {
        let mut store = f.api.store.lock().unwrap();
        let (mut credential, blob) = store.credential_secret(legacy.credential_id).unwrap();
        credential.target = "fixture".into();
        store
            .save_credential(
                &f.admin.actor,
                &credential,
                Some(credential.revision),
                &blob,
            )
            .unwrap();
    }
    input["profile"]["host"] = json!("fixture");
    let (status, _) = call(&f.app, "POST", path, &f.admin.token, input, None).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn discovery_requires_admin_before_body_and_never_accepts_server_paths() {
    let f = Fixture::new();
    for path in [
        "/v1/discovery/preview",
        "/v1/discovery/apply",
        "/v1/discovery/discard",
    ] {
        let response = f
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(path)
                    .header("Authorization", format!("Bearer {}", *f.operator.token))
                    .body(Body::from("x".repeat(API_BODY_LIMIT + 1)))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
    assert_eq!(
        call(
            &f.app,
            "POST",
            "/v1/discovery/preview",
            &f.admin.token,
            json!({"format":"Csv","path":"C:/private/secret.csv"}),
            None
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        call(
            &f.app,
            "POST",
            "/v1/discovery/preview",
            &f.admin.token,
            json!({"format":"Manual","text":"x".repeat(DISCOVERY_TEXT_LIMIT+1)}),
            None
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        call(
            &f.app,
            "POST",
            "/v1/discovery/preview",
            &f.admin.token,
            json!({"format":"Manual","text":"https://SYNTHETIC_SECRET"}),
            None
        )
        .await
        .1["code"],
        "INVALID_DATA"
    );
    assert!(f
        .api
        .store
        .lock()
        .unwrap()
        .snapshot()
        .unwrap()
        .devices
        .is_empty());
}

#[tokio::test]
async fn discovery_is_actor_bound_confirmed_idempotent_and_visible_to_other_clients() {
    let mut f = Fixture::new();
    let second = new_token("Second Admin".into(), Role::Admin, 1).unwrap();
    f.api
        .store
        .lock()
        .unwrap()
        .issue_token(&f.admin.actor, &second.actor, &hash_token(&second.token))
        .unwrap();
    let (code, p) = call(
        &f.app,
        "POST",
        "/v1/discovery/preview",
        &f.admin.token,
        json!({"format":"Csv","text":"host,name,site\npc-api,Приёмная,Офис\nPC-API,Приёмная,Офис"}),
        None,
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(p["changes"], 1);
    assert_eq!(p["rows"][1]["action"], "Duplicate");
    assert_eq!(p["rows"][0]["name"], "Приёмная");
    let request = json!({"preview_id":p["id"],"confirmed":true});
    assert_eq!(
        call(
            &f.app,
            "POST",
            "/v1/discovery/apply",
            &second.token,
            request.clone(),
            None
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        call(
            &f.app,
            "POST",
            "/v1/discovery/apply",
            &f.admin.token,
            json!({"preview_id":p["id"],"confirmed":false}),
            None
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        call(
            &f.app,
            "POST",
            "/v1/discovery/apply",
            &f.admin.token,
            json!({"preview_id":p["id"],"confirmed":true,"backup":"C:/anything.sqlite"}),
            None
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert!(f
        .api
        .store
        .lock()
        .unwrap()
        .snapshot()
        .unwrap()
        .devices
        .is_empty());
    let (code, applied) = call(
        &f.app,
        "POST",
        "/v1/discovery/apply",
        &f.admin.token,
        request.clone(),
        None,
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(applied["changes"], 1);
    assert!(applied["backup"]
        .as_str()
        .unwrap()
        .starts_with("before-discovery-"));
    assert_eq!(
        call(
            &f.app,
            "POST",
            "/v1/discovery/apply",
            &f.admin.token,
            request,
            None
        )
        .await
        .1,
        applied
    );
    let (_, w) = call(
        &f.app,
        "GET",
        "/v1/workspace",
        &f.viewer.token,
        Value::Null,
        None,
    )
    .await;
    assert_eq!(w["inventory"]["devices"].as_array().unwrap().len(), 1);
    assert!(w["inventory"]["devices"][0]["scan"].is_null());
    assert!(f._receiver.try_recv().is_err());
    let (_, stale) = call(
        &f.app,
        "POST",
        "/v1/discovery/preview",
        &f.admin.token,
        json!({"format":"Manual","text":"new-stale"}),
        None,
    )
    .await;
    f.api
        .store
        .lock()
        .unwrap()
        .save_collection(&collection(), None)
        .unwrap();
    assert_eq!(
        call(
            &f.app,
            "POST",
            "/v1/discovery/apply",
            &f.admin.token,
            json!({"preview_id":stale["id"],"confirmed":true}),
            None
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        f.api
            .store
            .lock()
            .unwrap()
            .snapshot()
            .unwrap()
            .devices
            .len(),
        1
    );
}

#[tokio::test]
async fn inventory_status_is_readable_to_all_roles_but_never_returns_raw_rows() {
    use orkestr_inventory::{DiscoveryBatch, InventoryReport, ProviderReport};
    let f = Fixture::new();
    let at = Utc::now();
    let (device, good, failed) = {
        let mut store = f.api.store.lock().unwrap();
        let plan = store
            .preview_discovery(&DiscoveryBatch::manual("status-fixture").unwrap())
            .unwrap();
        let device = plan.writes()[0].clone();
        store.apply_discovery(&plan).unwrap();
        let good = InventoryReport {
            id: Uuid::new_v4(),
            device_id: device.id,
            started_at: at,
            finished_at: at,
            providers: vec![ProviderReport {
                provider: InventoryProvider::Environment,
                state: ProviderState::Complete,
                failure: None,
                rows: vec![json!({"value":"SYNTHETIC_PRIVATE_REPORT"})],
            }],
        };
        store.record_inventory(&good.normalize().unwrap()).unwrap();
        let failed = InventoryReport {
            id: Uuid::new_v4(),
            device_id: device.id,
            started_at: at + chrono::Duration::seconds(1),
            finished_at: at + chrono::Duration::seconds(2),
            providers: vec![ProviderReport {
                provider: InventoryProvider::OperatingSystem,
                state: ProviderState::Failed,
                failure: Some(ProviderFailure::Timeout),
                rows: vec![],
            }],
        };
        store
            .record_inventory(&failed.normalize().unwrap())
            .unwrap();
        (device, good, failed)
    };
    for token in [&f.admin.token, &f.operator.token, &f.viewer.token] {
        let (code, status) = call(
            &f.app,
            "GET",
            &format!("/v1/devices/{}/inventory-status", device.id),
            token,
            Value::Null,
            None,
        )
        .await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(status["latest"]["id"], failed.id.to_string());
        assert_eq!(status["latest"]["state"], "Failed");
        assert_eq!(status["head"]["id"], good.id.to_string());
        assert!(!status.to_string().contains("SYNTHETIC_PRIVATE_REPORT"));
        assert!(!status.to_string().contains("VariableValue"));
    }
    assert_eq!(
        call(
            &f.app,
            "GET",
            &format!("/v1/devices/{}/inventory-status", Uuid::new_v4()),
            &f.viewer.token,
            Value::Null,
            None
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
}
#[tokio::test]
async fn two_clients_conflict_and_personal_revocation() {
    let f = Fixture::new();
    let c = collection();
    let body = json!({"collection":c,"expected":null});
    let (status, saved) = call(
        &f.app,
        "POST",
        "/v1/collections",
        &f.operator.token,
        body.clone(),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(saved["revision"], 1);
    assert_eq!(
        call(
            &f.app,
            "POST",
            "/v1/collections",
            &f.admin.token,
            body,
            None
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let (_, workspace) = call(
        &f.app,
        "GET",
        "/v1/workspace",
        &f.viewer.token,
        Value::Null,
        None,
    )
    .await;
    assert_eq!(workspace["inventory"]["collections"][0]["name"], "Shared");
    assert_eq!(
        call(
            &f.app,
            "POST",
            &format!("/v1/operators/{}/revoke", f.operator.actor.id),
            &f.admin.token,
            Value::Null,
            None
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            &f.app,
            "GET",
            "/v1/workspace",
            &f.operator.token,
            Value::Null,
            None
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(
            &f.app,
            "GET",
            "/v1/workspace",
            &f.admin.token,
            Value::Null,
            None
        )
        .await
        .0,
        StatusCode::OK
    );
    let (_, events) = call(
        &f.app,
        "GET",
        "/v1/events?after=0",
        &f.admin.token,
        Value::Null,
        None,
    )
    .await;
    assert!(events.to_string().contains("token.revoke"));
    assert!(!events.to_string().contains(&*f.admin.token));
}
#[tokio::test]
async fn jobs_are_bounded_idempotent_cancelable_and_interrupted_not_retried() {
    let f = Fixture::new();
    let c = f.seed_connection();
    let body = json!({"connection_id":c.id});
    assert_eq!(
        call(
            &f.app,
            "POST",
            "/v1/jobs",
            &f.operator.token,
            body.clone(),
            None
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let (_, first) = call(
        &f.app,
        "POST",
        "/v1/jobs",
        &f.operator.token,
        body.clone(),
        Some("same-key"),
    )
    .await;
    let (_, again) = call(
        &f.app,
        "POST",
        "/v1/jobs",
        &f.operator.token,
        body.clone(),
        Some("same-key"),
    )
    .await;
    assert_eq!(first["id"], again["id"]);
    let mut ids = vec![first["id"].as_str().unwrap().to_string()];
    for i in 1..8 {
        let (status, job) = call(
            &f.app,
            "POST",
            "/v1/jobs",
            &f.operator.token,
            body.clone(),
            Some(&format!("key-{i}")),
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED);
        ids.push(job["id"].as_str().unwrap().into());
    }
    assert_eq!(
        call(
            &f.app,
            "POST",
            "/v1/jobs",
            &f.operator.token,
            body.clone(),
            Some("overflow")
        )
        .await
        .0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        call(
            &f.app,
            "POST",
            "/v1/jobs",
            &f.operator.token,
            body,
            Some("same-key")
        )
        .await
        .0,
        StatusCode::ACCEPTED
    );
    assert_eq!(
        call(
            &f.app,
            "POST",
            &format!("/v1/jobs/{}/cancel", ids[0]),
            &f.operator.token,
            Value::Null,
            None
        )
        .await
        .0,
        StatusCode::OK
    );
    f.api.store.lock().unwrap().interrupt_jobs().unwrap();
    for id in &ids[1..] {
        let (_, job) = call(
            &f.app,
            "GET",
            &format!("/v1/jobs/{id}"),
            &f.operator.token,
            Value::Null,
            None,
        )
        .await;
        assert_eq!(job["state"], "Interrupted");
    }
    let (_, job) = call(
        &f.app,
        "GET",
        &format!("/v1/jobs/{}", ids[0]),
        &f.operator.token,
        Value::Null,
        None,
    )
    .await;
    assert_eq!(job["state"], "Cancelled");
}
#[tokio::test]
async fn body_limits_profile_scope_metadata_and_cursor_resync() {
    let f = Fixture::new();
    let c = f.seed_connection();
    let mut wrong = c.clone();
    wrong.id = Uuid::new_v4();
    wrong.host = "other.invalid".into();
    assert_eq!(
        call(
            &f.app,
            "POST",
            "/v1/connections",
            &f.admin.token,
            json!({"profile":wrong,"expected":null}),
            None
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    wrong = c.clone();
    wrong.id = Uuid::new_v4();
    wrong.port = 5985;
    assert_eq!(
        call(
            &f.app,
            "POST",
            "/v1/connections",
            &f.admin.token,
            json!({"profile":wrong,"expected":null}),
            None
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let (_, metadata) = call(
        &f.app,
        "GET",
        "/v1/credentials",
        &f.admin.token,
        Value::Null,
        None,
    )
    .await;
    assert!(!metadata.to_string().contains("ciphertext-fixture"));
    assert!(!metadata.to_string().contains("password"));
    assert_eq!(
        call(
            &f.app,
            "POST",
            "/v1/operators",
            &f.admin.token,
            json!({"label":"x".repeat(API_BODY_LIMIT),"role":"Viewer","days":1}),
            None
        )
        .await
        .0,
        StatusCode::PAYLOAD_TOO_LARGE
    );
    for _ in 0..150 {
        f.api
            .store
            .lock()
            .unwrap()
            .audit_denied(&f.viewer.actor)
            .unwrap();
    }
    let (_, events) = call(
        &f.app,
        "GET",
        "/v1/events?after=0",
        &f.admin.token,
        Value::Null,
        None,
    )
    .await;
    assert_eq!(events["resync"], true);
    assert_eq!(events["events"].as_array().unwrap().len(), EVENT_LIMIT);
}
#[tokio::test]
async fn non_admin_workspace_and_preview_strip_private_legacy_and_m3_payloads() {
    let f = Fixture::new();
    let device = Device {
        id: DeviceId::new(),
        host: "fixture.invalid".into(),
        aliases: vec![],
        ad_guid: None,
        name: "Fixture".into(),
        domain: String::new(),
        ou: String::new(),
        os: String::new(),
        source: "Fixture".into(),
        site: String::new(),
        disabled: false,
        inventory_metadata: json!({"private":"SYNTHETIC_PRIVATE"}),
    };
    f.api.store.lock().unwrap().upsert_device(&device).unwrap();
    f.api.store.lock().unwrap().append_scan(&DeviceScan {
        id: ScanId::new(), device_id: device.id, scanned_at: Utc::now(),
        installed_ram_bytes: None, software_state: ProviderState::Partial,
        software: vec![InstalledSoftware { application_id: None, name: "Public software fixture".into(),
            publisher: "Fixture".into(), version: "1".into(), architecture: None, channel: None }],
        details: json!({"Environment":[{"VariableValue":"SYNTHETIC_M3_PRIVATE"}],
            "OrkestrInventory":{"schema":1,"software":[{"scope":{"User":{"sid":"SYNTHETIC_M3_PRIVATE"}}}]}}),
    }).unwrap();
    let (_, viewer) = call(
        &f.app,
        "GET",
        "/v1/workspace",
        &f.viewer.token,
        Value::Null,
        None,
    )
    .await;
    let (_, admin) = call(
        &f.app,
        "GET",
        "/v1/workspace",
        &f.admin.token,
        Value::Null,
        None,
    )
    .await;
    assert!(!viewer.to_string().contains("SYNTHETIC_PRIVATE"));
    assert!(admin.to_string().contains("SYNTHETIC_PRIVATE"));
    assert!(!viewer.to_string().contains("SYNTHETIC_M3_PRIVATE"));
    assert!(admin.to_string().contains("SYNTHETIC_M3_PRIVATE"));
    assert!(viewer.to_string().contains("Public software fixture"));
    let (_, operator) = call(
        &f.app,
        "GET",
        "/v1/workspace",
        &f.operator.token,
        Value::Null,
        None,
    )
    .await;
    assert!(!operator.to_string().contains("SYNTHETIC_M3_PRIVATE"));
    assert!(!operator.to_string().contains("SYNTHETIC_PRIVATE"));
    let (_, p) = call(
        &f.app,
        "POST",
        "/v1/preview",
        &f.viewer.token,
        json!("All"),
        None,
    )
    .await;
    assert!(!p.to_string().contains("SYNTHETIC_PRIVATE"));
    assert!(!p.to_string().contains("SYNTHETIC_M3_PRIVATE"));
}

#[tokio::test]
async fn expired_tokens_and_revoked_queued_work_fail_closed_without_probe() {
    let mut f = Fixture::new();
    let mut expired = new_token("Expired fixture".into(), Role::Admin, 1).unwrap();
    expired.actor.expires_at = Utc::now() - chrono::Duration::seconds(1);
    f.api
        .store
        .lock()
        .unwrap()
        .issue_token(&f.admin.actor, &expired.actor, &hash_token(&expired.token))
        .unwrap();
    assert_eq!(
        call(&f.app, "GET", "/v1/me", &expired.token, Value::Null, None)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    let connection = f.seed_connection();
    let (_, job) = call(
        &f.app,
        "POST",
        "/v1/jobs",
        &f.operator.token,
        json!({"connection_id":connection.id}),
        Some("revoked-before-start"),
    )
    .await;
    let id = Uuid::parse_str(job["id"].as_str().unwrap()).unwrap();
    f.api
        .store
        .lock()
        .unwrap()
        .revoke_token(&f.admin.actor, f.operator.actor.id)
        .unwrap();
    let (_dummy, receiver) = mpsc::channel(8);
    let receiver = std::mem::replace(&mut f._receiver, receiver);
    let (shutdown, stopping) = tokio::sync::watch::channel(false);
    let workers = api::workers(f.api.clone(), receiver, stopping).await;
    let start = std::time::Instant::now();
    loop {
        let job = f.api.store.lock().unwrap().job(id).unwrap();
        if job.state == JobState::Failed {
            assert_eq!(job.code, "CREDENTIAL_OR_AUTHORIZATION_UNAVAILABLE");
            break;
        }
        assert!(start.elapsed() < std::time::Duration::from_secs(5));
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    shutdown.send(true).unwrap();
    for worker in workers {
        worker.await.unwrap();
    }
}

#[tokio::test]
async fn library_jobs_are_admin_scoped_idempotent_and_download_never_implies_approval() {
    let f = Fixture::new();
    let application = json!({"id":ApplicationId::new(),"revision":0,"name":"Fixture","channel":"STABLE","architecture":"AMD64","source":{"kind":"Manual","version":"2.0"},"download":{"kind":"Manual","url":"https://repo.example.test/file.exe"},"network":{"allowed_hosts":["repo.example.test"],"max_bytes":1048576},"verification":{"kind":"Hash","expected_sha256":"a".repeat(64)},"software_match":{"name":"Fixture","publisher":"Publisher","product_code":null}});
    for token in [&f.viewer.token, &f.operator.token] {
        assert_eq!(
            call(
                &f.app,
                "POST",
                "/v1/library/applications",
                token,
                json!({"application":application,"expected":null}),
                None
            )
            .await
            .0,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            call(&f.app, "GET", "/v1/policies", token, Value::Null, None)
                .await
                .0,
            StatusCode::FORBIDDEN
        );
    }
    let (status, app) = call(
        &f.app,
        "POST",
        "/v1/library/applications",
        &f.admin.token,
        json!({"application":application,"expected":null}),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(app["architecture"], "x64");
    assert_eq!(app["channel"], "stable");
    assert_eq!(
        call(
            &f.app,
            "POST",
            "/v1/library/applications",
            &f.admin.token,
            json!({"application":app,"expected":null}),
            None
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let task = json!({"kind":"Check","application_id":app["id"],"revision":app["revision"]});
    let (status, job) = call(
        &f.app,
        "POST",
        "/v1/library/jobs",
        &f.admin.token,
        task.clone(),
        Some("one-check"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, replay) = call(
        &f.app,
        "POST",
        "/v1/library/jobs",
        &f.admin.token,
        task,
        Some("one-check"),
    )
    .await;
    assert_eq!(job["id"], replay["id"]);
    let path = format!("/v1/library/jobs/{}", job["id"].as_str().unwrap());
    assert_eq!(
        call(&f.app, "GET", &path, &f.viewer.token, Value::Null, None)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(&f.app, "GET", &path, &f.admin.token, Value::Null, None)
            .await
            .1["state"],
        "Queued"
    );
    assert_eq!(
        call(
            &f.app,
            "POST",
            &format!("{path}/cancel"),
            &f.admin.token,
            Value::Null,
            None
        )
        .await
        .1["state"],
        "Cancelled"
    );
    let release = {
        let mut store = f.api.store.lock().unwrap();
        let app: LibraryApplication = serde_json::from_value(app).unwrap();
        let preview = orkestr_library::preview_source(&app, &[], None, vec![], None).unwrap();
        store.add_release(f.admin.actor.id, &app, preview).unwrap()
    };
    let change = json!({"id":release.id,"application_revision":release.application_revision});
    assert_eq!(
        call(
            &f.app,
            "POST",
            "/v1/library/approve",
            &f.admin.token,
            change.clone(),
            None
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        call(
            &f.app,
            "POST",
            "/v1/library/publish",
            &f.admin.token,
            json!({"release":change,"product_code":Uuid::new_v4(),"execution":{"kind":"Msi"}}),
            None
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert!(f
        .api
        .store
        .lock()
        .unwrap()
        .deployments()
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn workflow_recovery_keeps_device_locked_until_explicit_evidence_resolution() {
    let f = Fixture::new();
    let connection = f.seed_connection();
    let device = f.seed_device(&connection.host);
    let definition =
        json!({"name":"Checkpoint fixture","version":"1","steps":[{"kind":"Sleep","seconds":0}]});
    assert_eq!(
        call(
            &f.app,
            "POST",
            "/v1/workflows/revisions",
            &f.operator.token,
            definition.clone(),
            None
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let (_, package) = call(
        &f.app,
        "POST",
        "/v1/workflows/revisions",
        &f.admin.token,
        definition,
        None,
    )
    .await;
    let plan = json!({"device_id":device,"package_revision_id":package["id"],"connection_id":connection.id,"connection_revision":connection.revision});
    let (status, first) = call(
        &f.app,
        "POST",
        "/v1/workflows",
        &f.admin.token,
        plan.clone(),
        Some("checkpoint-first"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, second) = call(
        &f.app,
        "POST",
        "/v1/workflows",
        &f.admin.token,
        plan,
        Some("checkpoint-second"),
    )
    .await;
    let first_id = Uuid::parse_str(first["id"].as_str().unwrap()).unwrap();
    let first_path = format!("/v1/workflows/{first_id}");
    let second_path = format!("/v1/workflows/{}", second["id"].as_str().unwrap());
    let (_, queued) = call(
        &f.app,
        "POST",
        &format!("{first_path}/approve"),
        &f.admin.token,
        json!({"expected_revision":1}),
        None,
    )
    .await;
    assert_eq!(queued["state"], "Queued");
    assert_eq!(
        call(
            &f.app,
            "POST",
            &format!("{second_path}/approve"),
            &f.admin.token,
            json!({"expected_revision":1}),
            None
        )
        .await
        .1["code"],
        "DEPLOYMENT_DEVICE_BUSY"
    );
    {
        let mut store = f.api.store.lock().unwrap();
        let mut run = store.workflow_run(first_id).unwrap().unwrap();
        run.state = DeploymentState::Running;
        run.checkpoints.push(StepCheckpoint {
            index: 0,
            started_at: Utc::now(),
            finished_at: None,
            code: "STEP_STARTED".into(),
            exit_code: None,
            evidence: None,
            transfer: None,
        });
        store.update_workflow(f.admin.actor.id, &mut run).unwrap();
        store.recover_workflows().unwrap();
    }
    let (_, unknown) = call(
        &f.app,
        "GET",
        &first_path,
        &f.admin.token,
        Value::Null,
        None,
    )
    .await;
    assert_eq!(unknown["state"], "NeedsReconciliation");
    assert_eq!(
        call(
            &f.app,
            "POST",
            &format!("{first_path}/retry"),
            &f.admin.token,
            json!({"expected_revision":unknown["revision"]}),
            None
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(call(&f.app,"POST",&format!("{first_path}/resolve-step"),&f.operator.token,json!({"expected_revision":unknown["revision"],"completed":false,"evidence":"Fixture did not run"}),None).await.0,StatusCode::FORBIDDEN);
    let (_,resolved)=call(&f.app,"POST",&format!("{first_path}/resolve-step"),&f.admin.token,json!({"expected_revision":unknown["revision"],"completed":false,"evidence":"Synthetic checkpoint; no process was launched"}),None).await;
    assert_eq!(resolved["state"], "Failed");
    assert_eq!(resolved["next_step"], 0);
    assert_eq!(
        call(
            &f.app,
            "POST",
            &format!("{second_path}/approve"),
            &f.admin.token,
            json!({"expected_revision":1}),
            None
        )
        .await
        .1["state"],
        "Queued"
    );
}
