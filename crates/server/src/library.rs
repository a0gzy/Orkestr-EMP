use crate::api::Api;
use orkestr_library::{ByteBudget, GuardedHttp};
use orkestr_protocol::*;
use std::{sync::atomic::Ordering, time::Duration};
use uuid::Uuid;
use zeroize::Zeroizing;

pub(crate) fn execute(api: &Api, id: Uuid) {
    let prepared = (|| {
        let mut store = api.store.lock().ok()?;
        let mut job = store.library_job(id).ok()??;
        if job.state != LibraryJobState::Queued {
            return None;
        }
        if job.retry_at.is_some_and(|time| time > chrono::Utc::now()) {
            return None;
        }
        let actor_ok = store.operators().ok()?.iter().any(|a| {
            a.id == job.actor_id && a.role == Role::Admin && a.expires_at > chrono::Utc::now()
        });
        let app = match job.task {
            LibraryTask::Check {
                application_id,
                revision,
            } => store
                .library_application(application_id)
                .ok()?
                .filter(|a| a.revision == revision),
            LibraryTask::Download { release_id } => {
                store.library_release(release_id).ok()?.and_then(|r| {
                    store
                        .library_application(r.application_id)
                        .ok()
                        .flatten()
                        .filter(|a| {
                            a.revision == r.application_revision
                                && matches!(
                                    r.state,
                                    ReleaseState::Candidate | ReleaseState::Quarantined
                                )
                        })
                })
            }
        };
        if !actor_ok || app.is_none() {
            job.state = LibraryJobState::Failed;
            job.code = "LIBRARY_SCOPE_OR_AUTHORIZATION_CHANGED".into();
            let _ = store.set_library_job(&job);
            return None;
        }
        let phase = if matches!(job.task, LibraryTask::Download { .. }) {
            Phase::Download
        } else {
            Phase::ReadOnly
        };
        let quotas = store.quotas().ok()?.1;
        if !store.acquire_resource(id, "repository", phase).ok()? {
            return None;
        }
        job.state = LibraryJobState::Running;
        job.attempts += 1;
        job.retry_at = None;
        job.code = "RUNNING".into();
        if store.set_library_job(&job).is_err() {
            let _ = store.release_resource(id);
            return None;
        }
        Some((job, app?, quotas))
    })();
    let Some((mut job, app, quotas)) = prepared else {
        return;
    };
    let outcome = (|| -> Result<Uuid, String> {
        let http = GuardedHttp::new(&app.network).map_err(|e| e.to_string())?;
        match job.task {
            LibraryTask::Check { .. } => {
                let (bytes, final_url, redirects) = if let Some(url) = app.source.url() {
                    let (bytes, url, redirects) = http.fetch(url).map_err(|e| e.to_string())?;
                    (bytes, Some(url), redirects)
                } else {
                    (vec![], None, vec![])
                };
                let page_url = match &app.download {
                    DownloadSource::Link { page_url, .. }
                    | DownloadSource::HtmlLink { page_url, .. } => Some(page_url),
                    _ => None,
                };
                let page = if let Some(page_url) = page_url {
                    if Some(page_url.as_str()) == app.source.url() {
                        Some((
                            bytes.clone(),
                            final_url.clone().ok_or("SOURCE_URL_REQUIRED")?,
                            redirects.clone(),
                        ))
                    } else {
                        Some(http.fetch(page_url).map_err(|e| e.to_string())?)
                    }
                } else {
                    None
                };
                let mut preview = orkestr_library::preview_source(
                    &app,
                    &bytes,
                    final_url,
                    redirects,
                    page.as_ref()
                        .map(|(bytes, url, _)| (url.as_str(), bytes.as_slice())),
                )
                .map_err(|e| e.to_string())?;
                preview.evidence.download_page_redirects = page
                    .as_ref()
                    .map(|(_, _, redirects)| redirects.clone())
                    .unwrap_or_default();
                let mut store = api.store.lock().map_err(|_| "STORAGE_UNAVAILABLE")?;
                if !authorized(&store, job.actor_id) {
                    return Err("AUTHORIZATION_CHANGED".into());
                }
                store
                    .add_release(job.actor_id, &app, preview)
                    .map(|r| r.id)
                    .map_err(|_| "SOURCE_CHANGED".into())
            }
            LibraryTask::Download { release_id } => {
                let mut release = api
                    .store
                    .lock()
                    .map_err(|_| "STORAGE_UNAVAILABLE")?
                    .library_release(release_id)
                    .map_err(|_| "RELEASE_UNAVAILABLE")?
                    .ok_or("RELEASE_UNAVAILABLE")?;
                let root = api
                    .artifact_directory
                    .as_ref()
                    .and_then(|p| p.parent())
                    .ok_or("ARTIFACT_UNAVAILABLE")?;
                let quarantine = root.join("quarantine");
                std::fs::create_dir_all(&quarantine).map_err(|_| "QUARANTINE_UNAVAILABLE")?;
                let part = quarantine.join(format!("{}.part", release.id));
                // One shared download slot by default; the global budget is shared across workers.
                let budget = shared_download_budget(quotas.download_bytes_per_second);
                let downloaded =
                    if let DownloadSource::LocalArtifact { sha256, extension } = &app.download {
                        use sha2::Digest;
                        use std::io::{Read, Write};
                        let source = root.join("artifacts").join(format!("{sha256}.{extension}"));
                        let mut input =
                            crate::deployment::open_artifact(&source).map_err(str::to_string)?;
                        let mut output =
                            std::fs::File::create(&part).map_err(|_| "QUARANTINE_UNAVAILABLE")?;
                        let mut hash = sha2::Sha256::new();
                        let mut bytes = 0u64;
                        let mut buffer = [0; 65536];
                        loop {
                            let n = input
                                .read(&mut buffer)
                                .map_err(|_| "ARTIFACT_UNAVAILABLE")?;
                            if n == 0 {
                                break;
                            }
                            bytes += n as u64;
                            if bytes > app.network.max_bytes {
                                return Err("ARTIFACT_TOO_LARGE".into());
                            }
                            hash.update(&buffer[..n]);
                            output
                                .write_all(&buffer[..n])
                                .map_err(|_| "QUARANTINE_UNAVAILABLE")?;
                        }
                        output.sync_all().map_err(|_| "QUARANTINE_UNAVAILABLE")?;
                        let computed = format!("{:x}", hash.finalize());
                        if computed != *sha256 {
                            return Err("ARTIFACT_HASH_MISMATCH".into());
                        }
                        orkestr_library::Downloaded {
                            sha256: computed,
                            bytes,
                            final_url: release.url.clone(),
                        }
                    } else {
                        orkestr_library::download(
                            &http,
                            &release.url,
                            &part,
                            app.network.max_bytes,
                            &budget,
                            &api.stopping,
                        )
                        .map_err(|e| e.to_string())?
                    };
                release.state = ReleaseState::Quarantined;
                release.sha256 = Some(downloaded.sha256.clone());
                release.bytes = Some(downloaded.bytes);
                api.store
                    .lock()
                    .map_err(|_| "STORAGE_UNAVAILABLE")?
                    .update_release(job.actor_id, &release)
                    .map_err(|_| "SOURCE_CHANGED")?;
                let mut file = std::fs::File::open(&part).map_err(|_| "ARTIFACT_UNAVAILABLE")?;
                let mut prefix = [0u8; 2];
                std::io::Read::read_exact(&mut file, &mut prefix)
                    .map_err(|_| "INVALID_INSTALLER")?;
                drop(file);
                let extension = if prefix == *b"MZ" { "exe" } else { "msi" };
                let candidate = quarantine.join(format!("{}.{extension}", release.id));
                if candidate.exists() {
                    std::fs::remove_file(&candidate).map_err(|_| "QUARANTINE_UNAVAILABLE")?;
                }
                std::fs::rename(&part, &candidate).map_err(|_| "QUARANTINE_UNAVAILABLE")?;
                match &app.verification {
                    VerificationPolicy::Hash { expected_sha256 } => {
                        if expected_sha256 != &downloaded.sha256 {
                            return Err("ARTIFACT_HASH_MISMATCH".into());
                        }
                    }
                    VerificationPolicy::Authenticode { publisher } => {
                        let input = serde_json::json!({"path":candidate,"publisher":publisher});
                        let bytes = crate::probe::invoke(
                            include_str!("verify-signature.ps1"),
                            Zeroizing::new(
                                serde_json::to_vec(&input).map_err(|_| "SIGNATURE_INPUT_FAILED")?,
                            ),
                            crate::probe::QueryLimits {
                                deadline: Duration::from_secs(45),
                                max_output: 64,
                                stopping: Some(&api.stopping),
                            },
                        )
                        .map_err(str::to_string)?;
                        if bytes != b"SIGNATURE_VERIFIED" {
                            return Err("SIGNATURE_VERIFICATION_FAILED".into());
                        }
                    }
                }
                let hash = if extension == "exe" {
                    crate::deployment::stage_exe(root, &candidate)
                } else {
                    crate::deployment::stage_msi(root, &candidate)
                }
                .map_err(str::to_string)?;
                if hash != downloaded.sha256 {
                    return Err("ARTIFACT_CHANGED".into());
                }
                let metadata_input = serde_json::json!({"path":candidate,"extension":extension});
                release.artifact_metadata = crate::probe::invoke(
                    include_str!("installer-metadata.ps1"),
                    Zeroizing::new(
                        serde_json::to_vec(&metadata_input).map_err(|_| "METADATA_INPUT_FAILED")?,
                    ),
                    crate::probe::QueryLimits {
                        deadline: Duration::from_secs(30),
                        max_output: 4096,
                        stopping: Some(&api.stopping),
                    },
                )
                .ok()
                .and_then(|bytes| serde_json::from_slice(&bytes).ok());
                if api.stopping.load(Ordering::Relaxed) {
                    return Err("SERVER_STOPPING".into());
                }
                release.state = ReleaseState::Verified;
                let mut store = api.store.lock().map_err(|_| "STORAGE_UNAVAILABLE")?;
                if !authorized(&store, job.actor_id) {
                    return Err("AUTHORIZATION_CHANGED".into());
                }
                store
                    .update_release(job.actor_id, &release)
                    .map_err(|_| "SOURCE_CHANGED")?;
                Ok(release.id)
            }
        }
    })();
    match outcome {
        Ok(id) => {
            job.release_id = Some(id);
            job.state = LibraryJobState::Succeeded;
            job.code = "LIBRARY_TASK_COMPLETE".into();
        }
        Err(code) => {
            if job.attempts < 3
                && matches!(
                    code.as_str(),
                    "HTTP_REQUEST_FAILED"
                        | "HTTP_RATE_LIMITED"
                        | "DOWNLOAD_TRUNCATED"
                        | "LIBRARY_IO_FAILED"
                )
            {
                job.state = LibraryJobState::Queued;
                job.retry_at = Some(orkestr_scheduler::retry_at(
                    job.id,
                    job.attempts,
                    chrono::Utc::now(),
                    120,
                ));
            } else {
                job.state = LibraryJobState::Failed;
            }
            job.code = code;
        }
    }
    if let Ok(mut store) = api.store.lock() {
        let _ = store.set_library_job(&job);
        let _ = store.release_resource(job.id);
    }
}
fn authorized(store: &orkestr_storage::Store, actor: Uuid) -> bool {
    store.operators().is_ok_and(|actors| {
        actors
            .iter()
            .any(|a| a.id == actor && a.role == Role::Admin && a.expires_at > chrono::Utc::now())
    })
}
fn shared_download_budget(rate: u64) -> std::sync::Arc<ByteBudget> {
    static BUDGET: std::sync::OnceLock<std::sync::Mutex<(u64, std::sync::Arc<ByteBudget>)>> =
        std::sync::OnceLock::new();
    let shared = BUDGET
        .get_or_init(|| std::sync::Mutex::new((rate, std::sync::Arc::new(ByteBudget::new(rate)))));
    let mut value = shared.lock().unwrap_or_else(|e| e.into_inner());
    if value.0 != rate {
        *value = (rate, std::sync::Arc::new(ByteBudget::new(rate)));
    }
    value.1.clone()
}
