//! Explicit source/download-only pilot. Never approves, publishes or installs.
use orkestr_client::Client;
use orkestr_domain::ApplicationId;
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
    let [data, config, consent] = args.as_slice() else {
        return Err("<DATA> <SOURCE-JSON> --download-only-library-pilot required".into());
    };
    if consent != "--download-only-library-pilot" {
        return Err("EXPLICIT_PILOT_ARGUMENTS_REQUIRED".into());
    }
    let cwd = std::env::current_dir().map_err(|_| "WORKSPACE_UNAVAILABLE")?;
    let data = cwd.join(data);
    let config = cwd.join(config);
    let handle = axum_server::Handle::new();
    let service_handle = handle.clone();
    let service_data = data.clone();
    let server = tokio::spawn(async move { runtime::serve(&service_data, service_handle).await });
    let address = handle.listening().await.ok_or("LISTENER_FAILED")?;
    let result = tokio::task::spawn_blocking(move || {
        run(
            data,
            config,
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
fn wait(client: &Client, id: Uuid) -> Result<LibraryJob, String> {
    let started = Instant::now();
    loop {
        let job: LibraryJob = client.get(&format!("/v1/library/jobs/{id}"))?;
        if !matches!(
            job.state,
            LibraryJobState::Queued | LibraryJobState::Running
        ) {
            return Ok(job);
        }
        if started.elapsed() > Duration::from_secs(180) {
            return Err("LIBRARY_PILOT_TIMEOUT".into());
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}
fn run(data: PathBuf, config: PathBuf, origin: String) -> Result<(), String> {
    let token = orkestr_client::load_token(&data.join("bootstrap-admin.dpapi"))?;
    let ca = std::fs::read(data.join("server.crt")).map_err(|_| "CA_UNAVAILABLE")?;
    let client = Client::new(&origin, token, Some(&ca))?;
    let mut input: SaveApplication =
        serde_json::from_slice(&std::fs::read(config).map_err(|_| "CONFIG_UNAVAILABLE")?)
            .map_err(|_| "INVALID_CONFIG")?;
    input.application.id = ApplicationId::new();
    input.application.revision = 0;
    input.expected = None;
    let app: LibraryApplication = client.post("/v1/library/applications", &input, None)?;
    let job: LibraryJob = client.post(
        "/v1/library/jobs",
        &LibraryTask::Check {
            application_id: app.id,
            revision: app.revision,
        },
        Some(&format!("source-{}", Uuid::new_v4())),
    )?;
    let checked = wait(&client, job.id)?;
    if checked.state != LibraryJobState::Succeeded {
        return Err(checked.code);
    }
    let release_id = checked.release_id.ok_or("RELEASE_UNAVAILABLE")?;
    let job: LibraryJob = client.post(
        "/v1/library/jobs",
        &LibraryTask::Download { release_id },
        Some(&format!("download-{}", Uuid::new_v4())),
    )?;
    let downloaded = wait(&client, job.id)?;
    let releases: Vec<ReleaseCandidate> = client.get("/v1/library/releases")?;
    let release = releases
        .into_iter()
        .find(|r| r.id == release_id)
        .ok_or("RELEASE_UNAVAILABLE")?;
    std::fs::write(data.join("source-download-pilot-result.json"),serde_json::to_vec_pretty(&serde_json::json!({"application":app,"check":checked,"download":downloaded,"release":release})).map_err(|_|"REPORT_FAILED")?).map_err(|_|"REPORT_FAILED")?;
    println!(
        "SOURCE_VERSION {}\nDOWNLOAD {:?} {}\nRELEASE {:?}\nSHA256 {}\nBYTES {}",
        release.version,
        downloaded.state,
        downloaded.code,
        release.state,
        release.sha256.as_deref().unwrap_or(""),
        release.bytes.unwrap_or(0)
    );
    if downloaded.state != LibraryJobState::Succeeded || release.state != ReleaseState::Verified {
        return Err("SOURCE_OR_TRUST_NOT_VERIFIED".into());
    }
    Ok(())
}
