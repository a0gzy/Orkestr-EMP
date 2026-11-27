//! Opt-in two-device quota/transport pilot. Copies an 8 MiB synthetic file into
//! unique marked folders, verifies/removes only those files, and scans inventory.
//! No installer, reboot, logoff, firewall or WinRM configuration changes.
use orkestr_client::Client;
use orkestr_domain::{DeviceId, PackageRevision};
use orkestr_protocol::*;
use orkestr_server::runtime;
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use uuid::Uuid;
const BYTES: u64 = 8 * 1024 * 1024;
#[tokio::main]
async fn main() -> Result<(), String> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let [data, first, second, consent] = args.as_slice() else {
        return Err("<DATA> <AUTHORIZED-TARGET-1> <AUTHORIZED-TARGET-2> --two-device-transfer-pilot required".into());
    };
    if consent != "--two-device-transfer-pilot" || first.eq_ignore_ascii_case(second) {
        return Err("TWO_DISTINCT_AUTHORIZED_TARGETS_REQUIRED".into());
    }
    let data = std::env::current_dir()
        .map_err(|_| "WORKSPACE_UNAVAILABLE")?
        .join(data);
    let targets = [first.clone(), second.clone()];
    let handle = axum_server::Handle::new();
    let service_handle = handle.clone();
    let service_data = data.clone();
    let server = tokio::spawn(async move { runtime::serve(&service_data, service_handle).await });
    let address = handle.listening().await.ok_or("LISTENER_FAILED")?;
    let result = tokio::task::spawn_blocking(move || {
        run(
            data,
            targets,
            format!("https://localhost:{}", address.port()),
        )
    })
    .await
    .map_err(|_| "PILOT_WORKER_FAILED")?;
    handle.shutdown();
    let stopped = server.await.map_err(|_| "SERVER_JOIN_FAILED")?;
    result?;
    stopped
}
fn connect(data: &Path, origin: &str) -> Result<Client, String> {
    let token = orkestr_client::load_token(&data.join("bootstrap-admin.dpapi"))?;
    let ca = std::fs::read(data.join("server.crt")).map_err(|_| "CA_UNAVAILABLE")?;
    Client::new(origin, token, Some(&ca))
}
fn definition(nonce: Uuid, phase: &str, hash: &str) -> WorkflowDefinition {
    let folder = format!(r"C:\OrkestrEMP-pilot-{nonce}-{phase}");
    let setup=r#"
$folder='{FOLDER}';$marker=Join-Path $folder 'owner.txt'
if(Test-Path -LiteralPath $folder){throw 'Pilot directory already exists'}
$null=New-Item -ItemType Directory -Path $folder
$acl=Get-Acl -LiteralPath $folder;$acl.SetAccessRuleProtection($true,$false)
foreach($sid in @([Security.Principal.WindowsIdentity]::GetCurrent().User,[Security.Principal.SecurityIdentifier]::new('S-1-5-18'))){$acl.AddAccessRule([Security.AccessControl.FileSystemAccessRule]::new($sid,'FullControl','ContainerInherit,ObjectInherit','None','Allow'))}
Set-Acl -LiteralPath $folder -AclObject $acl
[IO.File]::WriteAllText($marker,'{ID}')
"#.replace("{FOLDER}",&folder).replace("{ID}",&nonce.to_string());
    let cleanup=r#"
$folder='{FOLDER}';$expected=[IO.Path]::GetFullPath('{FOLDER}')
$item=Get-Item -LiteralPath $folder -Force
if($item.FullName -cne $expected -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0){throw 'Pilot root mismatch'}
$marker=Join-Path $expected 'owner.txt';$file=Join-Path $expected 'payload.bin'
if([IO.File]::ReadAllText($marker) -cne '{ID}' -or (Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash -ine '{HASH}'){throw 'Pilot evidence mismatch'}
# Only these two freshly-created, verified files; never recursive cleanup.
[IO.File]::Delete($file);[IO.File]::Delete($marker);[IO.Directory]::Delete($expected,$false)
"#.replace("{FOLDER}",&folder).replace("{ID}",&nonce.to_string()).replace("{HASH}",hash);
    WorkflowDefinition {
        name: format!("Two-device {phase} transfer {nonce}"),
        version: "1".into(),
        steps: vec![
            WorkflowStep::PowerShell {
                script: setup,
                timeout_seconds: 30,
            },
            WorkflowStep::FileCopy {
                sha256: hash.into(),
                destination: format!(r"{folder}\payload.bin"),
                overwrite: false,
            },
            WorkflowStep::Sleep { seconds: 3 },
            WorkflowStep::PowerShell {
                script: cleanup,
                timeout_seconds: 30,
            },
            WorkflowStep::SystemScan {},
        ],
    }
}
fn settings(client: &Client) -> Result<(u64, Quotas), String> {
    let value: serde_json::Value = client.get("/v1/scheduler")?;
    serde_json::from_value(value["settings"].clone())
        .map_err(|_| "INVALID_SCHEDULER_RESPONSE".into())
}
fn run(data: PathBuf, targets: [String; 2], origin: String) -> Result<(), String> {
    let client = connect(&data, &origin)?;
    let preview: PreparedDiscovery = client.post(
        "/v1/discovery/preview",
        &PrepareDiscovery {
            format: DiscoverySource::Manual,
            text: targets.join("\n"),
        },
        None,
    )?;
    let _: AppliedDiscovery = client.post(
        "/v1/discovery/apply",
        &ApplyDiscovery {
            preview_id: preview.id,
            confirmed: true,
        },
        None,
    )?;
    let mut profiles: Vec<ConnectionProfile> = client.get("/v1/connections")?;
    for host in &targets {
        if !profiles
            .iter()
            .any(|p| p.host.eq_ignore_ascii_case(host) && p.kind == ProfileKind::Execution)
        {
            let saved: ConfiguredConnection = client.post(
                "/v1/connections/setup",
                &ConfigureConnection {
                    id: Uuid::new_v4(),
                    host: host.clone(),
                    locate_domain_controller: false,
                    kind: ProfileKind::Execution,
                    winrm_authentication: Some(WinRmAuthentication::Kerberos),
                    account: ConnectionAccount::ServerIdentity {},
                },
                None,
            )?;
            profiles.push(saved.connection);
        }
    }
    let workspace: orkestr_application::Workspace = client.get("/v1/workspace")?;
    let selected: Vec<(DeviceId, ConnectionProfile)> = targets
        .iter()
        .map(|host| {
            let device = workspace
                .inventory
                .devices
                .iter()
                .find(|d| d.device.host.eq_ignore_ascii_case(host))
                .ok_or("DEVICE_UNAVAILABLE")?;
            let profile = profiles
                .iter()
                .find(|p| p.host.eq_ignore_ascii_case(host) && p.kind == ProfileKind::Execution)
                .ok_or("PROFILE_UNAVAILABLE")?;
            Ok((device.device.id, profile.clone()))
        })
        .collect::<Result<_, String>>()?;
    let nonce = Uuid::new_v4();
    let source = data.join(format!("matrix-payload-{nonce}.bin"));
    std::fs::write(&source, vec![0x5au8; BYTES as usize]).map_err(|_| "PAYLOAD_WRITE_FAILED")?;
    let hash = orkestr_server::deployment::stage_file(&data, &source).map_err(str::to_string)?;
    let (_, original) = settings(&client)?;
    let result: Result<(), String> = (|| {
        let mut reports = vec![];
        for (phase, sequential, expected_max) in [("parallel", false, 2), ("sequential", true, 1)] {
            let (revision, _) = settings(&client)?;
            let mut quotas = original.clone();
            quotas.global = 2;
            quotas.per_site = 2;
            quotas.transfers = 2;
            quotas.installs = 2;
            quotas.strict_sequential = sequential;
            quotas.endpoint_bytes_per_second = 1024 * 1024;
            let _saved: (u64, Quotas) = client.post(
                "/v1/scheduler",
                &serde_json::json!({"expected_revision":revision,"quotas":quotas}),
                None,
            )?;
            let package: PackageRevision = client.post(
                "/v1/workflows/revisions",
                &definition(nonce, phase, &hash),
                None,
            )?;
            let mut ids = vec![];
            for (device, profile) in &selected {
                let run: WorkflowRun = client.post(
                    "/v1/workflows",
                    &PlanDeployment {
                        device_id: *device,
                        package_revision_id: package.id,
                        connection_id: profile.id,
                        connection_revision: profile.revision,
                    },
                    Some(&format!("matrix-{}", Uuid::new_v4())),
                )?;
                let run: WorkflowRun = client.post(
                    &format!("/v1/workflows/{}/approve", run.id),
                    &DeploymentChange {
                        expected_revision: run.revision,
                    },
                    None,
                )?;
                ids.push(run.id);
            }
            // A fresh client is used for observation; workers have no UI dependency.
            let observer = connect(&data, &origin)?;
            let started = Instant::now();
            let mut maximum = 0usize;
            let runs = loop {
                let state: serde_json::Value = observer.get("/v1/scheduler")?;
                let leases: Vec<Lease> = serde_json::from_value(state["leases"].clone())
                    .map_err(|_| "LEASE_RESPONSE_FAILED")?;
                let active = leases.iter().filter(|l| ids.contains(&l.id)).count();
                maximum = maximum.max(active);
                if active > expected_max {
                    return Err("QUOTA_EXCEEDED".into());
                }
                let runs: Vec<WorkflowRun> = ids
                    .iter()
                    .map(|id| observer.get(&format!("/v1/workflows/{id}")))
                    .collect::<Result<_, _>>()?;
                if runs
                    .iter()
                    .all(|r| !matches!(r.state, DeploymentState::Queued | DeploymentState::Running))
                {
                    break runs;
                }
                if started.elapsed() > Duration::from_secs(180) {
                    return Err("MATRIX_TIMEOUT".into());
                }
                std::thread::sleep(Duration::from_millis(200));
            };
            let transfers: Vec<_> = runs
                .iter()
                .flat_map(|r| r.checkpoints.iter().filter_map(|c| c.transfer.as_ref()))
                .collect();
            let per_target = if sequential { 1024 * 1024 } else { 512 * 1024 };
            let valid = runs
                .iter()
                .all(|r| r.state == DeploymentState::Succeeded && r.next_step == 5)
                && maximum == expected_max
                && transfers.len() == 2
                && transfers
                    .iter()
                    .all(|t| t.bytes == BYTES && t.elapsed_ms >= BYTES * 1000 / per_target);
            println!("MATRIX {phase} MAX_ACTIVE {maximum} CAP {expected_max} VALID {valid}");
            reports.push(serde_json::json!({"phase":phase,"max_active":maximum,"expected_max":expected_max,"endpoint_bytes_per_second":1024*1024,"valid":valid,"runs":runs}));
            std::fs::write(
                data.join("two-device-transfer-pilot-result.json"),
                serde_json::to_vec_pretty(&reports).map_err(|_| "REPORT_FAILED")?,
            )
            .map_err(|_| "REPORT_FAILED")?;
            if !valid {
                return Err("MATRIX_NOT_CONFIRMED".into());
            }
        }
        Ok(())
    })();
    let restore = (|| {
        let (revision, _) = settings(&client)?;
        let _saved: (u64, Quotas) = client.post(
            "/v1/scheduler",
            &serde_json::json!({"expected_revision":revision,"quotas":original}),
            None,
        )?;
        Ok::<_, String>(())
    })();
    result?;
    restore
}
