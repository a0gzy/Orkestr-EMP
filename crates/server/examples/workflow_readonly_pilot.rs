//! Opt-in WinRM workflow pilot. Reads hostname/CIM, sleeps and tests exit 3010;
//! never installs software, reboots, logs off users or changes endpoint settings.
use orkestr_client::Client;
use orkestr_domain::DeviceRecord;
use orkestr_protocol::*;
use orkestr_server::runtime;
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
use uuid::Uuid;
#[tokio::main]
async fn main() -> Result<(), String> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let [data, target, consent] = args.as_slice() else {
        return Err("<DATA> <AUTHORIZED-TARGET> --read-only-workflow-pilot required".into());
    };
    if consent != "--read-only-workflow-pilot" {
        return Err("EXPLICIT_PILOT_ARGUMENTS_REQUIRED".into());
    }
    let data = std::env::current_dir()
        .map_err(|_| "WORKSPACE_UNAVAILABLE")?
        .join(data);
    let target = target.clone();
    let handle = axum_server::Handle::new();
    let service_handle = handle.clone();
    let service_data = data.clone();
    let server = tokio::spawn(async move { runtime::serve(&service_data, service_handle).await });
    let address = handle.listening().await.ok_or("LISTENER_FAILED")?;
    let result = tokio::task::spawn_blocking(move || {
        run(
            data,
            target,
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
fn run(data: PathBuf, target: String, origin: String) -> Result<(), String> {
    let connect = || -> Result<Client, String> {
        let token = orkestr_client::load_token(&data.join("bootstrap-admin.dpapi"))?;
        let ca = std::fs::read(data.join("server.crt")).map_err(|_| "CA_UNAVAILABLE")?;
        Client::new(&origin, token, Some(&ca))
    };
    let client = connect()?;
    let connections: Vec<ConnectionProfile> = client.get("/v1/connections")?;
    let profile = connections
        .into_iter()
        .find(|p| p.host.eq_ignore_ascii_case(&target) && p.kind == ProfileKind::Execution)
        .ok_or("SAVED_EXECUTION_PROFILE_REQUIRED")?;
    let workspace: orkestr_application::Workspace = client.get("/v1/workspace")?;
    let device: &DeviceRecord = workspace
        .inventory
        .devices
        .iter()
        .find(|d| d.device.host.eq_ignore_ascii_case(&target))
        .ok_or("SAVED_DEVICE_REQUIRED")?;
    let package: orkestr_domain::PackageRevision = client.post(
        "/v1/workflows/revisions",
        &WorkflowDefinition {
            name: "Read-only workflow pilot".into(),
            version: "1".into(),
            steps: vec![
                WorkflowStep::Command {
                    executable: r"C:\Windows\System32\hostname.exe".into(),
                    arguments: String::new(),
                    success_codes: vec![0],
                    timeout_seconds: 30,
                },
                WorkflowStep::PowerShell {
                    script:
                        "$null=Get-CimInstance Win32_OperatingSystem -ErrorAction Stop;exit 3010"
                            .into(),
                    timeout_seconds: 30,
                },
                WorkflowStep::Sleep { seconds: 1 },
                WorkflowStep::SystemScan {},
            ],
        },
        None,
    )?;
    let run: WorkflowRun = client.post(
        "/v1/workflows",
        &PlanDeployment {
            device_id: device.device.id,
            package_revision_id: package.id,
            connection_id: profile.id,
            connection_revision: profile.revision,
        },
        Some(&format!("readonly-{}", Uuid::new_v4())),
    )?;
    let approved: WorkflowRun = client.post(
        &format!("/v1/workflows/{}/approve", run.id),
        &DeploymentChange {
            expected_revision: run.revision,
        },
        None,
    )?;
    drop(client);
    std::thread::sleep(Duration::from_secs(3));
    let client = connect()?;
    let start = Instant::now();
    let result = loop {
        let run: WorkflowRun = client.get(&format!("/v1/workflows/{}", approved.id))?;
        if !matches!(
            run.state,
            DeploymentState::Queued | DeploymentState::Running
        ) {
            break run;
        }
        if start.elapsed() > Duration::from_secs(150) {
            return Err("PILOT_TIMEOUT".into());
        }
        std::thread::sleep(Duration::from_millis(500));
    };
    std::fs::write(
        data.join("readonly-workflow-pilot-result.json"),
        serde_json::to_vec_pretty(&result).map_err(|_| "REPORT_FAILED")?,
    )
    .map_err(|_| "REPORT_FAILED")?;
    println!(
        "WORKFLOW {:?} {}\nCONFIRMED_STEPS {}\nACTUAL_REBOOT false",
        result.state, result.code, result.next_step
    );
    if result.state != DeploymentState::RebootRequired || result.next_step != 4 {
        return Err("WORKFLOW_NOT_CONFIRMED".into());
    }
    Ok(())
}
