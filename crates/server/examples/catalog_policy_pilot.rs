//! Explicit live pipeline check using the already-installed synthetic fixture.
use orkestr_client::Client;
use orkestr_domain::*;
use orkestr_protocol::*;
use orkestr_server::runtime;
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
use uuid::Uuid;
fn wait_job(client: &Client, id: Uuid) -> Result<LibraryJob, String> {
    let start = Instant::now();
    loop {
        let jobs: Vec<LibraryJob> = client.get("/v1/library/jobs")?;
        let job = jobs
            .into_iter()
            .find(|j| j.id == id)
            .ok_or("JOB_UNAVAILABLE")?;
        if !matches!(
            job.state,
            LibraryJobState::Queued | LibraryJobState::Running
        ) {
            return Ok(job);
        }
        if start.elapsed() > Duration::from_secs(90) {
            return Err("LIBRARY_JOB_TIMEOUT".into());
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}
#[tokio::main]
async fn main() -> Result<(), String> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let [data, file, target, consent] = args.as_slice() else {
        return Err(
            "<DATA> <SYNTHETIC-EXE> <AUTHORIZED-TARGET> --catalog-policy-pilot required".into(),
        );
    };
    if consent != "--catalog-policy-pilot" {
        return Err("EXPLICIT_PILOT_ARGUMENTS_REQUIRED".into());
    }
    let root = std::env::current_dir().map_err(|_| "WORKSPACE_UNAVAILABLE")?;
    let data = root.join(data);
    let file = root.join(file);
    let target = target.clone();
    let handle = axum_server::Handle::new();
    let server_handle = handle.clone();
    let server_data = data.clone();
    let server = tokio::spawn(async move { runtime::serve(&server_data, server_handle).await });
    let address = handle.listening().await.ok_or("LISTENER_FAILED")?;
    let result = tokio::task::spawn_blocking(move || {
        run(
            data,
            file,
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
fn run(data: PathBuf, file: PathBuf, target: String, origin: String) -> Result<(), String> {
    let token = orkestr_client::load_token(&data.join("bootstrap-admin.dpapi"))?;
    let ca = std::fs::read(data.join("server.crt")).map_err(|_| "CA_UNAVAILABLE")?;
    let client = Client::new(&origin, token, Some(&ca))?;
    let hash = orkestr_server::deployment::stage_exe(&data, &file).map_err(str::to_string)?;
    let application:LibraryApplication=serde_json::from_value(serde_json::json!({"id":ApplicationId::new(),"revision":0,"name":"Orkestr synthetic fixture catalog","channel":"Stable","architecture":"AMD64","source":{"kind":"Manual","version":"1.2.3"},"download":{"kind":"LocalArtifact","sha256":hash,"extension":"exe"},"network":{"allowed_hosts":[],"max_bytes":1048576},"verification":{"kind":"Hash","expected_sha256":hash},"software_match":{"name":"Orkestr synthetic deployment fixture 0","publisher":"Orkestr pilot (synthetic)","product_code":null}})).map_err(|_|"CONFIG_FAILED")?;
    let app: LibraryApplication = client.post(
        "/v1/library/applications",
        &SaveApplication {
            application,
            expected: None,
        },
        None,
    )?;
    let checked: LibraryJob = client.post(
        "/v1/library/jobs",
        &LibraryTask::Check {
            application_id: app.id,
            revision: app.revision,
        },
        Some("synthetic-catalog-check"),
    )?;
    let checked = wait_job(&client, checked.id)?;
    if checked.state != LibraryJobState::Succeeded {
        return Err(checked.code);
    }
    let release = checked.release_id.ok_or("RELEASE_UNAVAILABLE")?;
    let downloaded: LibraryJob = client.post(
        "/v1/library/jobs",
        &LibraryTask::Download {
            release_id: release,
        },
        Some("synthetic-catalog-quarantine"),
    )?;
    let downloaded = wait_job(&client, downloaded.id)?;
    if downloaded.state != LibraryJobState::Succeeded {
        return Err(downloaded.code);
    }
    let publish = PublishRelease {
        allow_upgrade: false,
        release: ChangeRelease {
            id: release,
            application_revision: app.revision,
        },
        product_code: Uuid::parse_str("5D6444C6-3D63-4E1F-AD2D-1D5000050000").unwrap(),
        execution: InstallerExecution::Exe {
            arguments: "/pilot-0".into(),
        },
        registry_key: None,
        installed_version: None,
    };
    if client
        .post::<_, PackageRevision>("/v1/library/publish", &publish, None)
        .is_ok()
    {
        return Err("DOWNLOAD_BYPASSED_APPROVAL".into());
    }
    let _: ReleaseCandidate = client.post("/v1/library/approve", &publish.release, None)?;
    let package: PackageRevision = client.post("/v1/library/publish", &publish, None)?;
    let workspace: orkestr_application::Workspace = client.get("/v1/workspace")?;
    let device = workspace
        .inventory
        .devices
        .iter()
        .find(|r| r.device.host.eq_ignore_ascii_case(&target))
        .ok_or("TARGET_UNAVAILABLE")?
        .device
        .id;
    let collection = Collection {
        id: CollectionId::new(),
        parent_id: None,
        name: "Owner-authorized synthetic pilot".into(),
        revision: 0,
        kind: CollectionKind::Static([device].into_iter().collect()),
    };
    let collection: Collection = client.post(
        "/v1/collections",
        &SaveCollection {
            collection,
            expected: None,
        },
        None,
    )?;
    let policy = Policy {
        id: Uuid::new_v4(),
        revision: 0,
        name: "Synthetic fixture opt-in pilot".into(),
        application_id: app.id,
        collection_id: collection.id,
        package_revision_id: package.id,
        automation: Automation {
            auto_check: false,
            auto_download: false,
            auto_deploy: true,
            unattended: true,
        },
        interval_seconds: 3600,
        jitter_seconds: 0,
        window: None,
        pilot_count: 1,
        soak_seconds: 2,
        max_failure_percent: 0,
        max_attempts: 1,
    };
    let mut policy: Policy = client.post(
        "/v1/policies",
        &SavePolicy {
            policy,
            expected: None,
        },
        None,
    )?;
    drop(client);
    std::thread::sleep(Duration::from_secs(3)); // the server continues without its client
    let token = orkestr_client::load_token(&data.join("bootstrap-admin.dpapi"))?;
    let client = Client::new(&origin, token, Some(&ca))?;
    let start = Instant::now();
    let run = loop {
        let runs: Vec<Rollout> = client.get("/v1/rollouts")?;
        if let Some(run) = runs.into_iter().find(|r| r.policy.id == policy.id) {
            if matches!(
                run.state,
                RolloutState::Succeeded | RolloutState::Paused | RolloutState::Cancelled
            ) {
                break run;
            }
        }
        if start.elapsed() > Duration::from_secs(60) {
            return Err("ROLLOUT_TIMEOUT".into());
        }
        std::thread::sleep(Duration::from_millis(250));
    };
    std::fs::write(
        data.join("catalog-policy-pilot-result.json"),
        serde_json::to_vec_pretty(&run).map_err(|_| "REPORT_FAILED")?,
    )
    .map_err(|_| "REPORT_FAILED")?;
    println!(
        "CATALOG_CHECK {:?}\nQUARANTINE {:?}\nROLLOUT {:?} {}\nTARGETS {}",
        checked.state,
        downloaded.state,
        run.state,
        run.code,
        run.targets.len()
    );
    let revision = policy.revision;
    policy.automation.auto_deploy = false;
    let _: Policy = client.post(
        "/v1/policies",
        &SavePolicy {
            policy,
            expected: Some(revision),
        },
        None,
    )?;
    if run.state != RolloutState::Succeeded {
        return Err("ROLLOUT_NOT_SUCCESSFUL".into());
    }
    Ok(())
}
