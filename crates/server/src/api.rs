use axum::{
    body::to_bytes,
    extract::{Request, State},
    http::{Method, StatusCode},
    response::{IntoResponse, Response},
    routing::any,
    Json, Router,
};
use orkestr_application::{preview, workspace, Selection};
use orkestr_domain::canonical_host;
use orkestr_protocol::*;
use orkestr_storage::{Error, Store};
use serde::{de::DeserializeOwned, Serialize};
use std::sync::{Arc, Mutex};
use tokio::sync::{mpsc, Semaphore};
use uuid::Uuid;

#[derive(Clone)]
pub struct Api {
    pub store: Arc<Mutex<Store>>,
    pub jobs: mpsc::Sender<Uuid>,
    pub sid: String,
    pub requests: Arc<Semaphore>,
    pub rate: Arc<Mutex<(std::time::Instant, u32)>>,
    pub instance_guard: Option<Arc<std::fs::File>>,
    pub artifact_directory: Option<std::path::PathBuf>,
    pub stopping: Arc<std::sync::atomic::AtomicBool>,
}
pub fn router(api: Api) -> Router {
    Router::new()
        .route("/{*path}", any(handle))
        .layer(axum::middleware::from_fn(
            |request: Request, next: axum::middleware::Next| async move {
                let mut response = next.run(request).await;
                response.headers_mut().insert(
                    "cache-control",
                    axum::http::HeaderValue::from_static("no-store"),
                );
                response.headers_mut().insert(
                    "x-content-type-options",
                    axum::http::HeaderValue::from_static("nosniff"),
                );
                response
            },
        ))
        .with_state(api)
}
type Failure = (StatusCode, Json<ApiError>);
fn fail(status: StatusCode, code: &str) -> Failure {
    (status, Json(ApiError { code: code.into() }))
}
fn db_error(e: Error) -> Failure {
    match e {
        Error::Conflict => fail(StatusCode::CONFLICT, "CONFLICT"),
        Error::PreviewUnavailable => fail(StatusCode::CONFLICT, "PREVIEW_UNAVAILABLE"),
        Error::DiscoveryBusy => fail(StatusCode::SERVICE_UNAVAILABLE, "DISCOVERY_BUSY"),
        Error::DeploymentDeviceBusy => fail(StatusCode::CONFLICT, "DEPLOYMENT_DEVICE_BUSY"),
        Error::Invalid(_) | Error::Domain(_) | Error::Inventory(_) => {
            fail(StatusCode::UNPROCESSABLE_ENTITY, "INVALID_DATA")
        }
        _ => fail(StatusCode::INTERNAL_SERVER_ERROR, "STORAGE_UNAVAILABLE"),
    }
}
fn json<T: Serialize>(value: T) -> Response {
    Json(value).into_response()
}
fn decode<T: DeserializeOwned>(body: &[u8]) -> Result<T, Failure> {
    serde_json::from_slice(body).map_err(|_| fail(StatusCode::BAD_REQUEST, "INVALID_JSON"))
}

/// Permissions are determined from exact method/path before reading a body.
fn permission(method: &Method, path: &str) -> Option<Role> {
    match (method.as_str(), path) {
        ("GET", "/v1/me" | "/v1/health" | "/v1/workspace" | "/v1/events")
        | ("POST", "/v1/preview") => Some(Role::Viewer),
        ("POST", "/v1/collections" | "/v1/collections/delete" | "/v1/jobs") => Some(Role::Operator),
        ("GET", "/v1/credentials" | "/v1/connections" | "/v1/operators" | "/v1/deployments")
        | (
            "POST",
            "/v1/credentials"
            | "/v1/connections"
            | "/v1/connections/winrm"
            | "/v1/connections/setup"
            | "/v1/operators"
            | "/v1/discovery/preview"
            | "/v1/discovery/apply"
            | "/v1/discovery/discard"
            | "/v1/inventory/jobs"
            | "/v1/deployments"
            | "/v1/deploy/revisions",
        ) => Some(Role::Admin),
        ("POST", p) if deployment_action(p).is_some() => Some(Role::Admin),
        (
            "GET",
            "/v1/library/applications"
            | "/v1/library/releases"
            | "/v1/library/jobs"
            | "/v1/scheduler"
            | "/v1/policies",
        )
        | (
            "POST",
            "/v1/library/applications"
            | "/v1/library/jobs"
            | "/v1/library/approve"
            | "/v1/library/reject"
            | "/v1/scheduler"
            | "/v1/policies",
        ) => Some(Role::Admin),
        ("POST", "/v1/library/publish" | "/v1/workflows/revisions" | "/v1/workflows")
        | ("GET", "/v1/workflows") => Some(Role::Admin),
        ("POST", p) if workflow_action(p).is_some() => Some(Role::Admin),
        ("GET", "/v1/rollouts") | ("POST", "/v1/rollouts" | "/v1/rollouts/preview") => {
            Some(Role::Admin)
        }
        ("GET", p)
            if p.strip_prefix("/v1/policies/")
                .and_then(|s| s.strip_suffix("/history"))
                .is_some_and(|id| Uuid::parse_str(id).is_ok()) =>
        {
            Some(Role::Admin)
        }
        ("POST", p) if rollout_action(p).is_some() => Some(Role::Admin),
        ("GET", p)
            if [
                "/v1/deployments/",
                "/v1/workflows/",
                "/v1/rollouts/",
                "/v1/library/jobs/",
                "/v1/library/releases/",
            ]
            .iter()
            .any(|prefix| entity_id(p, prefix).is_some()) =>
        {
            Some(Role::Admin)
        }
        ("POST", p)
            if p.strip_suffix("/cancel")
                .and_then(|p| entity_id(p, "/v1/library/jobs/"))
                .is_some() =>
        {
            Some(Role::Admin)
        }
        ("GET", p)
            if p.strip_prefix("/v1/devices/")
                .and_then(|s| s.strip_suffix("/inventory-status"))
                .is_some_and(|id| Uuid::parse_str(id).is_ok()) =>
        {
            Some(Role::Viewer)
        }
        ("GET", p)
            if p.strip_prefix("/v1/jobs/")
                .is_some_and(|id| Uuid::parse_str(id).is_ok()) =>
        {
            Some(Role::Viewer)
        }
        ("POST", p)
            if p.strip_prefix("/v1/jobs/")
                .and_then(|p| p.strip_suffix("/cancel"))
                .is_some_and(|id| Uuid::parse_str(id).is_ok()) =>
        {
            Some(Role::Operator)
        }
        ("POST", p)
            if p.strip_prefix("/v1/operators/")
                .and_then(|p| p.strip_suffix("/revoke"))
                .is_some_and(|id| Uuid::parse_str(id).is_ok()) =>
        {
            Some(Role::Admin)
        }
        _ => None,
    }
}
fn entity_id(path: &str, prefix: &str) -> Option<Uuid> {
    Uuid::parse_str(path.strip_prefix(prefix)?).ok()
}
fn allows(actual: Role, required: Role) -> bool {
    actual == Role::Admin
        || actual == required
        || (actual == Role::Operator && required == Role::Viewer)
}
async fn handle(State(api): State<Api>, request: Request) -> Result<Response, Failure> {
    {
        let mut budget = api
            .rate
            .lock()
            .map_err(|_| fail(StatusCode::SERVICE_UNAVAILABLE, "BUSY"))?;
        if budget.0.elapsed() >= std::time::Duration::from_secs(1) {
            *budget = (std::time::Instant::now(), 0);
        }
        if budget.1 >= 128 {
            return Err(fail(StatusCode::TOO_MANY_REQUESTS, "RATE_LIMITED"));
        }
        budget.1 += 1;
    }
    let slot = Arc::new(
        api.requests
            .clone()
            .try_acquire_owned()
            .map_err(|_| fail(StatusCode::SERVICE_UNAVAILABLE, "BUSY"))?,
    );
    let (parts, body) = request.into_parts();
    let method = parts.method;
    let path = parts.uri.path().to_string();
    let required =
        permission(&method, &path).ok_or_else(|| fail(StatusCode::NOT_FOUND, "NOT_FOUND"))?;
    let token = zeroize::Zeroizing::new(
        parts
            .headers
            .get("authorization")
            .and_then(|h| h.to_str().ok())
            .and_then(|s| s.strip_prefix("Bearer "))
            .filter(|s| s.len() == 64 && s.bytes().all(|c| c.is_ascii_hexdigit()))
            .ok_or_else(|| fail(StatusCode::UNAUTHORIZED, "UNAUTHORIZED"))?
            .to_string(),
    );
    let hash = crate::hash_token(&token);
    drop(token);
    {
        let database = api.store.clone();
        let hash = hash.clone();
        let work_slot = slot.clone();
        tokio::task::spawn_blocking(move || {
            let _slot = work_slot;
            let store = database
                .lock()
                .map_err(|_| fail(StatusCode::INTERNAL_SERVER_ERROR, "STORAGE_UNAVAILABLE"))?;
            let actor = store
                .authenticate(&hash)
                .map_err(db_error)?
                .ok_or_else(|| fail(StatusCode::UNAUTHORIZED, "UNAUTHORIZED"))?;
            if !allows(actor.role, required) {
                store.audit_denied(&actor).map_err(db_error)?;
                return Err(fail(StatusCode::FORBIDDEN, "FORBIDDEN"));
            }
            Ok(())
        })
        .await
        .map_err(|_| fail(StatusCode::INTERNAL_SERVER_ERROR, "WORKER_UNAVAILABLE"))??;
    }
    let bytes = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        to_bytes(body, API_BODY_LIMIT),
    )
    .await
    .map_err(|_| fail(StatusCode::REQUEST_TIMEOUT, "BODY_TIMEOUT"))?
    .map_err(|_| fail(StatusCode::PAYLOAD_TOO_LARGE, "BODY_TOO_LARGE"))?;
    let bytes = zeroize::Zeroizing::new(bytes.to_vec());
    let query = parts.uri.query().unwrap_or_default().to_string();
    let key = parts
        .headers
        .get("idempotency-key")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    // Serialize SQLite and recheck revocation immediately before every operation.
    tokio::task::spawn_blocking(move || {
        let _slot = slot;
        let setup = if method == Method::POST && path == "/v1/connections/setup" {
            let mut input: ConfigureConnection = decode(&bytes)?;
            if input.locate_domain_controller {
                if input.kind != ProfileKind::AdRead
                    || !matches!(input.account, ConnectionAccount::Password { .. })
                {
                    return Err(fail(StatusCode::UNPROCESSABLE_ENTITY, "INVALID_DOMAIN"));
                }
                input.host = locate_domain_controller(&input.host, &api.stopping)?;
            }
            Some(input)
        } else {
            None
        };
        let mut store = api
            .store
            .lock()
            .map_err(|_| fail(StatusCode::INTERNAL_SERVER_ERROR, "STORAGE_UNAVAILABLE"))?;
        let actor = store
            .authenticate(&hash)
            .map_err(db_error)?
            .ok_or_else(|| fail(StatusCode::UNAUTHORIZED, "UNAUTHORIZED"))?;
        if !allows(actor.role, required) {
            return Err(fail(StatusCode::FORBIDDEN, "FORBIDDEN"));
        }
        if let Some(input) = setup {
            return Ok(json(configure_connection(&api, &mut store, &actor, input)?));
        }
        dispatch(
            &api, &mut store, &actor, &method, &path, &query, &key, &bytes,
        )
    })
    .await
    .map_err(|_| fail(StatusCode::INTERNAL_SERVER_ERROR, "WORKER_UNAVAILABLE"))?
}
#[allow(clippy::too_many_arguments)]
fn dispatch(
    api: &Api,
    store: &mut Store,
    actor: &Actor,
    method: &Method,
    path: &str,
    query: &str,
    key: &str,
    body: &[u8],
) -> Result<Response, Failure> {
    let result = match (method.as_str(), path) {
        ("GET", p) if entity_id(p, "/v1/deployments/").is_some() => json(
            store
                .deployment(entity_id(p, "/v1/deployments/").unwrap())
                .map_err(db_error)?
                .ok_or_else(|| fail(StatusCode::NOT_FOUND, "DEPLOYMENT_NOT_FOUND"))?,
        ),
        ("GET", p) if entity_id(p, "/v1/workflows/").is_some() => json(
            store
                .workflow_run(entity_id(p, "/v1/workflows/").unwrap())
                .map_err(db_error)?
                .ok_or_else(|| fail(StatusCode::NOT_FOUND, "WORKFLOW_NOT_FOUND"))?,
        ),
        ("GET", p) if entity_id(p, "/v1/rollouts/").is_some() => bounded_json(
            &store
                .rollout(entity_id(p, "/v1/rollouts/").unwrap())
                .map_err(db_error)?
                .ok_or_else(|| fail(StatusCode::NOT_FOUND, "ROLLOUT_NOT_FOUND"))?,
        )?,
        ("GET", p) if entity_id(p, "/v1/library/releases/").is_some() => bounded_json(
            &store
                .library_release(entity_id(p, "/v1/library/releases/").unwrap())
                .map_err(db_error)?
                .ok_or_else(|| fail(StatusCode::NOT_FOUND, "RELEASE_NOT_FOUND"))?,
        )?,
        ("GET", p) if entity_id(p, "/v1/library/jobs/").is_some() => json(
            store
                .library_job(entity_id(p, "/v1/library/jobs/").unwrap())
                .map_err(db_error)?
                .ok_or_else(|| fail(StatusCode::NOT_FOUND, "LIBRARY_JOB_NOT_FOUND"))?,
        ),
        ("POST", p)
            if p.strip_suffix("/cancel")
                .and_then(|p| entity_id(p, "/v1/library/jobs/"))
                .is_some() =>
        {
            decode::<()>(body)?;
            let id = p
                .strip_suffix("/cancel")
                .and_then(|p| entity_id(p, "/v1/library/jobs/"))
                .unwrap();
            let mut job = store
                .library_job(id)
                .map_err(db_error)?
                .ok_or_else(|| fail(StatusCode::NOT_FOUND, "LIBRARY_JOB_NOT_FOUND"))?;
            if job.state != LibraryJobState::Queued {
                return Err(db_error(Error::Conflict));
            }
            job.state = LibraryJobState::Cancelled;
            job.code = "CANCELLED_BEFORE_START".into();
            job.retry_at = None;
            store.set_library_job(&job).map_err(db_error)?;
            json(job)
        }
        ("GET", p) if p.starts_with("/v1/policies/") => {
            let id = parse_id(p, "/v1/policies/", "/history")?;
            bounded_json(&store.policy_history(id).map_err(db_error)?)?
        }
        ("POST", "/v1/rollouts/preview") => {
            let input: PlanRollout = decode(body)?;
            let (policy, targets) = crate::scheduling::prepare(store, &input).map_err(db_error)?;
            let waves = orkestr_scheduler::waves(
                &policy,
                targets.iter().map(|t| t.device_id).collect(),
                chrono::Utc::now(),
            );
            json(
                serde_json::json!({"generation":input.generation,"policy":policy,"targets":targets,"waves":waves}),
            )
        }
        ("GET", "/v1/rollouts") => bounded_json(&store.rollouts().map_err(db_error)?)?,
        ("POST", "/v1/rollouts") => {
            validate_idempotency_key(key)?;
            json(
                crate::scheduling::plan(store, actor.id, key, &decode::<PlanRollout>(body)?)
                    .map_err(db_error)?,
            )
        }
        ("POST", p) if rollout_action(p).is_some() => {
            let (id, action) = rollout_action(p).unwrap();
            let change: DeploymentChange = decode(body)?;
            let mut run = store
                .rollout(id)
                .map_err(db_error)?
                .ok_or_else(|| fail(StatusCode::NOT_FOUND, "ROLLOUT_NOT_FOUND"))?;
            if run.revision != change.expected_revision {
                return Err(db_error(Error::Conflict));
            }
            run.state = match (action, &run.state) {
                ("approve", RolloutState::WaitingApproval) => RolloutState::Pilot,
                (
                    "pause",
                    RolloutState::Pilot | RolloutState::Soaking | RolloutState::Production,
                ) => RolloutState::Paused,
                ("resume", RolloutState::Paused) => RolloutState::Pilot,
                ("cancel", _) => RolloutState::Cancelled,
                _ => return Err(db_error(Error::Conflict)),
            };
            run.actor_id = actor.id;
            run.code = format!("ROLLOUT_{}", action.to_ascii_uppercase());
            store.update_rollout(actor.id, &mut run).map_err(db_error)?;
            if action == "cancel" {
                store
                    .cancel_rollout_children(actor.id, &run)
                    .map_err(db_error)?;
            }
            json(run)
        }
        ("GET", "/v1/workflows") => bounded_json(&store.workflow_runs().map_err(db_error)?)?,
        ("POST", "/v1/workflows/revisions") => json(
            store
                .register_workflow(actor.id, &decode::<WorkflowDefinition>(body)?)
                .map_err(db_error)?,
        ),
        ("POST", "/v1/workflows") => {
            validate_idempotency_key(key)?;
            let plan: PlanDeployment = decode(body)?;
            let (connection, credential, _) =
                crate::deployment::target(store, &plan, None, false).map_err(db_error)?;
            json(
                store
                    .plan_workflow(actor.id, key, plan, &connection, credential.revision)
                    .map_err(db_error)?,
            )
        }
        ("POST", p) if workflow_action(p).is_some() => {
            let (id, action) = workflow_action(p).unwrap();
            let mut run = store
                .workflow_run(id)
                .map_err(db_error)?
                .ok_or_else(|| fail(StatusCode::NOT_FOUND, "WORKFLOW_NOT_FOUND"))?;
            if action == "resolve-step" {
                #[derive(serde::Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Resolution {
                    expected_revision: u64,
                    completed: bool,
                    evidence: String,
                }
                let input: Resolution = decode(body)?;
                if run.revision != input.expected_revision
                    || run.state != DeploymentState::NeedsReconciliation
                {
                    return Err(db_error(Error::Conflict));
                }
                validate_text(&input.evidence, 512)?;
                let checkpoint = run
                    .checkpoints
                    .last_mut()
                    .ok_or_else(|| db_error(Error::Conflict))?;
                checkpoint.finished_at = Some(chrono::Utc::now());
                checkpoint.evidence = Some(input.evidence);
                checkpoint.code = if input.completed {
                    "MANUALLY_CONFIRMED"
                } else {
                    "MANUALLY_REJECTED"
                }
                .into();
                if input.completed {
                    run.next_step += 1;
                    run.state = DeploymentState::WaitingApproval;
                } else {
                    run.state = DeploymentState::Failed;
                }
                run.code = checkpoint.code.clone();
                run.action_by = Some(actor.id);
                store
                    .update_workflow(actor.id, &mut run)
                    .map_err(db_error)?;
                return Ok(json(run));
            }
            let change: DeploymentChange = decode(body)?;
            if run.revision != change.expected_revision {
                return Err(db_error(Error::Conflict));
            }
            let (state, code) = match (action, run.state) {
                ("approve", DeploymentState::WaitingApproval) => {
                    (DeploymentState::Queued, "APPROVED_QUEUED")
                }
                ("retry", DeploymentState::Failed)
                    if (run.code == "FAILED_BEFORE_STEP"
                        || run.code == "RESTARTED_BEFORE_STEP")
                        && run
                            .checkpoints
                            .iter()
                            .filter(|c| c.index == run.next_step)
                            .count()
                            < 3 =>
                {
                    (DeploymentState::Queued, "REAPPROVED_QUEUED")
                }
                ("reconcile", DeploymentState::NeedsReconciliation) => {
                    (DeploymentState::ReconcileQueued, "RECONCILIATION_QUEUED")
                }
                ("cancel", DeploymentState::WaitingApproval | DeploymentState::Queued) => {
                    (DeploymentState::Cancelled, "CANCELLED_BEFORE_STEP")
                }
                ("cancel", DeploymentState::Running) => {
                    run.cancel_requested = true;
                    (DeploymentState::Running, "CANCEL_REQUESTED")
                }
                _ => return Err(db_error(Error::Conflict)),
            };
            if action != "cancel" {
                crate::deployment::target(store, &run.plan, Some(&run.host), action == "reconcile")
                    .map_err(db_error)?;
            }
            run.state = state;
            run.code = code.into();
            run.action_by = Some(actor.id);
            store
                .update_workflow(actor.id, &mut run)
                .map_err(db_error)?;
            if matches!(
                state,
                DeploymentState::Queued | DeploymentState::ReconcileQueued
            ) {
                let _ = api.jobs.try_send(id);
            }
            json(run)
        }
        ("POST", "/v1/library/publish") => {
            let input: PublishRelease = decode(body)?;
            let release = store
                .library_release(input.release.id)
                .map_err(db_error)?
                .ok_or_else(|| fail(StatusCode::UNPROCESSABLE_ENTITY, "RELEASE_UNAVAILABLE"))?;
            let app = store
                .library_application(release.application_id)
                .map_err(db_error)?
                .ok_or_else(|| fail(StatusCode::UNPROCESSABLE_ENTITY, "SOURCE_UNAVAILABLE"))?;
            if release.state != ReleaseState::Approved
                || release.application_revision != input.release.application_revision
                || app.revision != release.application_revision
                || !input.execution.valid()
                || (input.product_code.is_nil() && input.registry_key.is_none())
            {
                return Err(fail(StatusCode::CONFLICT, "APPROVED_RELEASE_REQUIRED"));
            }
            let installed_version = input.installed_version.unwrap_or_else(|| {
                release
                    .version_raw
                    .trim()
                    .trim_start_matches('v')
                    .to_owned()
            });
            let comparison_version = if let Some(count) = app.software_match.version_components {
                installed_version
                    .split('.')
                    .take(count as usize)
                    .collect::<Vec<_>>()
                    .join(".")
            } else {
                installed_version.clone()
            };
            if orkestr_domain::NumericVersion::parse(&installed_version).is_err()
                || orkestr_domain::NumericVersion::compare(&comparison_version, &release.version)
                    != Some(std::cmp::Ordering::Equal)
            {
                return Err(fail(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "INSTALLED_VERSION_SOURCE_MISMATCH",
                ));
            }
            let installer = ManagedInstaller {
                allow_upgrade: input.allow_upgrade,
                version_components: app.software_match.version_components,
                name: app.name,
                version: installed_version,
                sha256: release
                    .sha256
                    .ok_or_else(|| fail(StatusCode::CONFLICT, "ARTIFACT_REQUIRED"))?,
                product_code: input.product_code,
                execution: input.execution,
                registry_key: input.registry_key,
            };
            if !crate::deployment::artifact_path(api, &installer).is_some_and(|path| path.is_file())
            {
                return Err(fail(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "ARTIFACT_UNAVAILABLE",
                ));
            }
            json(store.register_installer_origin(actor.id,&installer,serde_json::json!({"release_id":release.id,"application_id":release.application_id,"application_revision":release.application_revision})).map_err(db_error)?)
        }
        ("GET", "/v1/library/applications") => {
            bounded_json(&store.library_applications().map_err(db_error)?)?
        }
        ("POST", "/v1/library/applications") => json(
            store
                .save_application(actor.id, &decode::<SaveApplication>(body)?)
                .map_err(db_error)?,
        ),
        ("GET", "/v1/library/releases") => {
            bounded_json(&store.library_releases().map_err(db_error)?)?
        }
        ("GET", "/v1/library/jobs") => bounded_json(&store.library_jobs().map_err(db_error)?)?,
        ("POST", "/v1/library/jobs") => {
            validate_idempotency_key(key)?;
            let task: LibraryTask = decode(body)?;
            let permit = api
                .jobs
                .try_reserve()
                .map_err(|_| fail(StatusCode::SERVICE_UNAVAILABLE, "JOBS_BUSY"))?;
            let job = store
                .queue_library_job(actor.id, key, task)
                .map_err(db_error)?;
            if job.state == LibraryJobState::Queued {
                permit.send(job.id);
            }
            json(job)
        }
        ("POST", "/v1/library/approve" | "/v1/library/reject") => json(
            store
                .approve_release(
                    actor.id,
                    &decode::<ChangeRelease>(body)?,
                    path.ends_with("/approve"),
                )
                .map_err(db_error)?,
        ),
        ("GET", "/v1/scheduler") => json(
            serde_json::json!({"settings":store.quotas().map_err(db_error)?,"leases":store.resource_leases().map_err(db_error)?}),
        ),
        ("POST", "/v1/scheduler") => {
            #[derive(serde::Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Input {
                expected_revision: u64,
                quotas: Quotas,
            }
            let input: Input = decode(body)?;
            json(
                store
                    .save_quotas(actor.id, input.expected_revision, &input.quotas)
                    .map_err(db_error)?,
            )
        }
        ("GET", "/v1/policies") => bounded_json(&store.policies().map_err(db_error)?)?,
        ("POST", "/v1/policies") => json(
            store
                .save_policy(actor.id, &decode::<SavePolicy>(body)?)
                .map_err(db_error)?,
        ),
        ("GET", "/v1/me") => json(actor),
        ("POST", "/v1/discovery/preview") => bounded_json(
            &store
                .prepare_discovery_session(actor.id, &decode::<PrepareDiscovery>(body)?)
                .map_err(db_error)?,
        )?,
        ("POST", "/v1/discovery/apply") => json(
            store
                .apply_discovery_session(actor.id, &decode::<ApplyDiscovery>(body)?)
                .map_err(db_error)?,
        ),
        ("POST", "/v1/discovery/discard") => {
            decode::<()>(body)?;
            store.discard_discovery_session(actor.id);
            json(())
        }
        ("GET", p) if p.starts_with("/v1/devices/") => {
            let id = orkestr_domain::DeviceId(parse_id(p, "/v1/devices/", "/inventory-status")?);
            let status = store.device_inventory_status(id).map_err(|e| match e {
                Error::Invalid("device not found") => {
                    fail(StatusCode::NOT_FOUND, "DEVICE_NOT_FOUND")
                }
                e => db_error(e),
            })?;
            bounded_json(&status)?
        }
        ("GET", "/v1/health") => json(Health {
            status: "READY".into(),
            service_sid: api.sid.clone(),
            jobs_workers: 2,
            jobs_capacity: 8,
        }),
        ("GET", "/v1/workspace") => {
            let mut w = workspace(store).map_err(db_error)?;
            if actor.role != Role::Admin {
                redact_devices(&mut w.inventory.devices);
                for p in &mut w.packages {
                    p.steps.clear();
                    p.metadata = serde_json::Value::Null;
                }
                w.reviews.clear();
            }
            bounded_json(&w)?
        }
        ("POST", "/v1/preview") => {
            let mut p = preview(store, &decode::<Selection>(body)?).map_err(db_error)?;
            if actor.role != Role::Admin {
                redact_devices(&mut p.members);
            }
            bounded_json(&p)?
        }
        ("POST", "/v1/collections") => {
            let p: SaveCollection = decode(body)?;
            json(
                store
                    .server_save_collection(actor, &p.collection, p.expected)
                    .map_err(db_error)?,
            )
        }
        ("POST", "/v1/collections/delete") => {
            let p: DeleteCollection = decode(body)?;
            store
                .server_delete_collection(actor, p.id, p.expected)
                .map_err(db_error)?;
            json(())
        }
        ("GET", "/v1/credentials") => json(store.credentials().map_err(db_error)?),
        ("GET", "/v1/connections") => json(store.connections().map_err(db_error)?),
        ("GET", "/v1/operators") => json(store.operators().map_err(db_error)?),
        ("GET", "/v1/events") => {
            let after = query
                .strip_prefix("after=")
                .unwrap_or("0")
                .parse::<u64>()
                .map_err(|_| fail(StatusCode::BAD_REQUEST, "INVALID_CURSOR"))?;
            json(store.events(after).map_err(db_error)?)
        }
        ("POST", "/v1/operators") => {
            let p: IssueToken = decode(body)?;
            let issued = crate::new_token(p.label, p.role, p.days)
                .map_err(|_| fail(StatusCode::UNPROCESSABLE_ENTITY, "INVALID_TOKEN_POLICY"))?;
            store
                .issue_token(actor, &issued.actor, &crate::hash_token(&issued.token))
                .map_err(db_error)?;
            json(issued)
        }
        ("POST", "/v1/credentials") => {
            let p: SaveCredential = decode(body)?;
            if p.username == "SERVER_IDENTITY" {
                return Err(fail(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "EXPLICIT_SERVER_IDENTITY_SETUP_REQUIRED",
                ));
            }
            validate_text(&p.label, 80)?;
            validate_text(&p.username, 256)?;
            if p.password.is_empty() || p.password.len() > 4096 {
                return Err(fail(StatusCode::UNPROCESSABLE_ENTITY, "INVALID_SECRET"));
            }
            let target = canonical_host(&p.target)
                .map_err(|_| fail(StatusCode::UNPROCESSABLE_ENTITY, "INVALID_HOST"))?;
            let ciphertext = orkestr_credentials::protect(p.password.as_bytes())
                .map_err(|_| fail(StatusCode::SERVICE_UNAVAILABLE, "CREDENTIAL_UNAVAILABLE"))?;
            let meta = CredentialProfile {
                id: p.id,
                revision: 0,
                label: p.label,
                username: p.username,
                kind: p.kind,
                target,
                owner_sid: api.sid.clone(),
                updated_at: chrono::Utc::now(),
            };
            json(
                store
                    .save_credential(actor, &meta, p.expected, &ciphertext)
                    .map_err(db_error)?,
            )
        }
        ("POST", "/v1/connections" | "/v1/connections/winrm") => {
            let mut p: SaveConnection = decode(body)?;
            let explicit_winrm = path == "/v1/connections/winrm";
            if explicit_winrm != p.profile.winrm_authentication.is_some()
                || (explicit_winrm && p.profile.kind != ProfileKind::Execution)
            {
                return Err(fail(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "INVALID_WINRM_PROFILE",
                ));
            }
            validate_text(&p.profile.label, 80)?;
            p.profile.host = canonical_host(&p.profile.host)
                .map_err(|_| fail(StatusCode::UNPROCESSABLE_ENTITY, "INVALID_HOST"))?;
            let port = match p.profile.kind {
                ProfileKind::AdRead => 636,
                ProfileKind::Execution => 5986,
            };
            if p.profile.port != port || p.profile.host.parse::<std::net::IpAddr>().is_ok() {
                return Err(fail(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "TLS_DNS_NAME_AND_DEFAULT_SECURE_PORT_REQUIRED",
                ));
            }
            json(
                store
                    .save_connection(actor, &p.profile, p.expected)
                    .map_err(db_error)?,
            )
        }
        ("POST", "/v1/jobs") => {
            let p: TestConnection = decode(body)?;
            return queue_inventory_job(
                api,
                store,
                actor,
                key,
                p.connection_id,
                p.expected_revision,
                InventoryTask::TestConnection,
            );
        }
        ("POST", "/v1/inventory/jobs") => {
            let p: StartInventoryJob = decode(body)?;
            match &p.task {
                InventoryTask::DiscoverOu { base_dn }
                    if base_dn.len() <= 2048
                        && base_dn
                            .get(..3)
                            .is_some_and(|p| p.eq_ignore_ascii_case("OU="))
                        && !base_dn.chars().any(char::is_control) => {}
                InventoryTask::ScanDevice { .. } => {}
                _ => {
                    return Err(fail(
                        StatusCode::UNPROCESSABLE_ENTITY,
                        "INVALID_INVENTORY_TASK",
                    ))
                }
            }
            return queue_inventory_job(
                api,
                store,
                actor,
                key,
                p.connection_id,
                Some(p.expected_revision),
                p.task,
            );
        }
        ("POST", "/v1/deploy/revisions") => {
            let mut installer: ManagedInstaller = decode(body)?;
            validate_text(&installer.name, 128)?;
            validate_text(&installer.version, 80)?;
            installer.sha256.make_ascii_lowercase();
            if !installer.valid() {
                return Err(fail(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "INVALID_INSTALLER_MANIFEST",
                ));
            }
            if !crate::deployment::artifact_path(api, &installer).is_some_and(|p| p.is_file()) {
                return Err(fail(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "ARTIFACT_UNAVAILABLE",
                ));
            }
            json(
                store
                    .register_installer(actor.id, &installer)
                    .map_err(db_error)?,
            )
        }
        ("GET", "/v1/deployments") => bounded_json(&store.deployments().map_err(db_error)?)?,
        ("POST", "/v1/deployments") => {
            validate_idempotency_key(key)?;
            let plan: PlanDeployment = decode(body)?;
            if let Some(existing) = store.existing_deployment(actor.id, key).map_err(db_error)? {
                if existing.plan != plan {
                    return Err(fail(StatusCode::CONFLICT, "IDEMPOTENCY_CONFLICT"));
                }
                return Ok(json(existing));
            }
            let installer = store
                .managed_installer(plan.package_revision_id)
                .map_err(db_error)?
                .ok_or_else(|| {
                    fail(
                        StatusCode::UNPROCESSABLE_ENTITY,
                        "MANAGED_INSTALLER_REVISION_REQUIRED",
                    )
                })?;
            let (connection, credential, _) =
                crate::deployment::target(store, &plan, None, false).map_err(db_error)?;
            json(
                store
                    .create_deployment(
                        actor.id,
                        key,
                        plan,
                        &connection,
                        credential.revision,
                        installer,
                    )
                    .map_err(db_error)?,
            )
        }
        ("POST", p) if deployment_action(p).is_some() => {
            let (id, action) =
                deployment_action(p).ok_or_else(|| fail(StatusCode::NOT_FOUND, "NOT_FOUND"))?;
            if action == "resolve" {
                #[derive(serde::Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Resolution {
                    expected_revision: u64,
                    confirmed_stopped: bool,
                    evidence: String,
                }
                let input: Resolution = decode(body)?;
                validate_text(&input.evidence, 512)?;
                let mut record = store
                    .deployment(id)
                    .map_err(db_error)?
                    .ok_or_else(|| fail(StatusCode::NOT_FOUND, "DEPLOYMENT_NOT_FOUND"))?;
                if record.revision != input.expected_revision
                    || record.state != DeploymentState::NeedsReconciliation
                    || !input.confirmed_stopped
                {
                    return Err(db_error(Error::Conflict));
                }
                record.state = DeploymentState::Failed;
                record.code = "MANUALLY_RESOLVED_FAILED".into();
                record.manual_resolution = Some(ManualResolution {
                    actor_id: actor.id,
                    at: chrono::Utc::now(),
                    evidence: input.evidence,
                });
                store
                    .update_deployment(&mut record, actor.id)
                    .map_err(db_error)?;
                return Ok(json(record));
            }
            let change: DeploymentChange = decode(body)?;
            let mut record = store
                .deployment(id)
                .map_err(db_error)?
                .ok_or_else(|| fail(StatusCode::NOT_FOUND, "DEPLOYMENT_NOT_FOUND"))?;
            if record.revision != change.expected_revision {
                return Err(db_error(Error::Conflict));
            }
            let permit = if action == "cancel" {
                None
            } else {
                Some(
                    api.jobs
                        .try_reserve()
                        .map_err(|_| fail(StatusCode::SERVICE_UNAVAILABLE, "JOBS_BUSY"))?,
                )
            };
            crate::deployment::transition(store, &mut record, actor.id, action)
                .map_err(db_error)?;
            if let Some(permit) = permit {
                permit.send(id);
            }
            json(record)
        }
        ("POST", p) if p.starts_with("/v1/operators/") => {
            let id = parse_id(p, "/v1/operators/", "/revoke")?;
            store.revoke_token(actor, id).map_err(db_error)?;
            json(())
        }
        ("GET", p) if p.starts_with("/v1/jobs/") => {
            let job = store.job(parse_id(p, "/v1/jobs/", "")?).map_err(db_error)?;
            if actor.role != Role::Admin && actor.id != job.actor_id {
                return Err(fail(StatusCode::FORBIDDEN, "FORBIDDEN"));
            }
            json(job)
        }
        ("POST", p) if p.starts_with("/v1/jobs/") => {
            let mut job = store
                .job(parse_id(p, "/v1/jobs/", "/cancel")?)
                .map_err(db_error)?;
            if actor.role != Role::Admin && actor.id != job.actor_id {
                return Err(fail(StatusCode::FORBIDDEN, "FORBIDDEN"));
            }
            if job.state != JobState::Queued {
                return Err(fail(StatusCode::CONFLICT, "ONLY_QUEUED_TEST_CAN_CANCEL"));
            }
            job.state = JobState::Cancelled;
            job.code = "CANCELLED_BEFORE_START".into();
            store.set_job(&job).map_err(db_error)?;
            json(job)
        }
        _ => return Err(fail(StatusCode::NOT_FOUND, "NOT_FOUND")),
    };
    Ok(result)
}
fn queue_inventory_job(
    api: &Api,
    store: &mut Store,
    actor: &Actor,
    key: &str,
    connection_id: Uuid,
    expected_revision: Option<u64>,
    task: InventoryTask,
) -> Result<Response, Failure> {
    validate_idempotency_key(key)?;
    if let Some(job) = store.existing_job(actor.id, key).map_err(db_error)? {
        if job.connection_id != connection_id
            || job.task != task
            || (expected_revision.is_some()
                && expected_revision != Some(store.job_revisions(job.id).map_err(db_error)?.0))
        {
            return Err(fail(StatusCode::CONFLICT, "IDEMPOTENCY_CONFLICT"));
        }
        return Ok((StatusCode::ACCEPTED, Json(job)).into_response());
    }
    let connection = store
        .connection(connection_id)
        .map_err(db_error)?
        .ok_or_else(|| fail(StatusCode::NOT_FOUND, "CONNECTION_NOT_FOUND"))?;
    if expected_revision.is_some_and(|revision| revision != connection.revision) {
        return Err(db_error(Error::Conflict));
    }
    if !inventory_target_matches(store, &task, &connection).map_err(db_error)? {
        return Err(fail(
            StatusCode::UNPROCESSABLE_ENTITY,
            "INVENTORY_TARGET_SCOPE_MISMATCH",
        ));
    }
    let permit = api
        .jobs
        .try_reserve()
        .map_err(|_| fail(StatusCode::SERVICE_UNAVAILABLE, "JOBS_BUSY"))?;
    let job = store
        .create_inventory_job(actor, key, &connection, task)
        .map_err(db_error)?;
    permit.send(job.id);
    Ok((StatusCode::ACCEPTED, Json(job)).into_response())
}

fn validate_idempotency_key(key: &str) -> Result<(), Failure> {
    if key.is_empty()
        || key.len() > 80
        || !key.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
    {
        return Err(fail(StatusCode::BAD_REQUEST, "IDEMPOTENCY_KEY_REQUIRED"));
    }
    Ok(())
}

fn workflow_action(path: &str) -> Option<(Uuid, &str)> {
    let (id, action) = path.strip_prefix("/v1/workflows/")?.split_once('/')?;
    if !matches!(
        action,
        "approve" | "retry" | "reconcile" | "cancel" | "resolve-step"
    ) {
        return None;
    }
    Some((Uuid::parse_str(id).ok()?, action))
}
fn rollout_action(path: &str) -> Option<(Uuid, &str)> {
    let (id, action) = path.strip_prefix("/v1/rollouts/")?.split_once('/')?;
    if !matches!(action, "approve" | "pause" | "resume" | "cancel") {
        return None;
    }
    Some((Uuid::parse_str(id).ok()?, action))
}
fn deployment_action(path: &str) -> Option<(Uuid, &str)> {
    let (id, action) = path.strip_prefix("/v1/deployments/")?.split_once('/')?;
    if !matches!(
        action,
        "approve" | "cancel" | "retry" | "reconcile" | "resolve"
    ) {
        return None;
    }
    Some((Uuid::parse_str(id).ok()?, action))
}

fn inventory_target_matches(
    store: &Store,
    task: &InventoryTask,
    connection: &ConnectionProfile,
) -> orkestr_storage::Result<bool> {
    Ok(match task {
        InventoryTask::TestConnection => true,
        InventoryTask::DiscoverOu { .. } => {
            connection.kind == ProfileKind::AdRead && connection.port == 636
        }
        InventoryTask::ScanDevice { device_id } => {
            connection.kind == ProfileKind::Execution
                && connection.port == 5986
                && store
                    .device(*device_id)?
                    .is_some_and(|device| !device.disabled && device.host == connection.host)
        }
    })
}
fn parse_id(path: &str, prefix: &str, suffix: &str) -> Result<Uuid, Failure> {
    path.strip_prefix(prefix)
        .and_then(|s| s.strip_suffix(suffix))
        .and_then(|s| Uuid::parse_str(s).ok())
        .ok_or_else(|| fail(StatusCode::BAD_REQUEST, "INVALID_ID"))
}
fn validate_text(text: &str, max: usize) -> Result<(), Failure> {
    if text.trim().is_empty() || text.len() > max || text.chars().any(char::is_control) {
        Err(fail(StatusCode::UNPROCESSABLE_ENTITY, "INVALID_TEXT"))
    } else {
        Ok(())
    }
}

fn configure_connection(
    api: &Api,
    store: &mut Store,
    actor: &Actor,
    input: ConfigureConnection,
) -> Result<ConfiguredConnection, Failure> {
    let host = canonical_host(&input.host)
        .map_err(|_| fail(StatusCode::UNPROCESSABLE_ENTITY, "INVALID_HOST"))?;
    if host.parse::<std::net::IpAddr>().is_ok()
        || host.bytes().all(|b| b.is_ascii_digit() || b == b'.')
    {
        return Err(fail(StatusCode::UNPROCESSABLE_ENTITY, "INVALID_HOST"));
    }
    let domain = host.split_once('.').map(|(_, domain)| domain);
    if matches!(
        (input.kind, input.winrm_authentication),
        (ProfileKind::AdRead, Some(_))
    ) || (input.winrm_authentication == Some(WinRmAuthentication::Kerberos) && domain.is_none())
    {
        return Err(fail(
            StatusCode::UNPROCESSABLE_ENTITY,
            "INVALID_WINRM_PROFILE",
        ));
    }
    let new_credential = |username| CredentialProfile {
        id: Uuid::new_v4(),
        revision: 1,
        label: host.clone(),
        username,
        kind: input.kind,
        target: host.clone(),
        owner_sid: api.sid.clone(),
        updated_at: chrono::Utc::now(),
    };
    let (credential, blob) = match input.account {
        ConnectionAccount::ServerIdentity {} => {
            if input.kind != ProfileKind::Execution
                || input.winrm_authentication != Some(WinRmAuthentication::Kerberos)
            {
                return Err(fail(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "KERBEROS_EXECUTION_REQUIRED",
                ));
            }
            let blob = orkestr_credentials::protect(b"OrkestrEMP integrated identity v1")
                .map_err(|_| fail(StatusCode::SERVICE_UNAVAILABLE, "CREDENTIAL_UNAVAILABLE"))?;
            (new_credential("SERVER_IDENTITY".into()), Some(blob))
        }
        ConnectionAccount::Password { username, password } => {
            validate_text(&username, 256)?;
            if password.is_empty() || password.len() > 4096 {
                return Err(fail(StatusCode::UNPROCESSABLE_ENTITY, "INVALID_SECRET"));
            }
            let blob = orkestr_credentials::protect(password.as_bytes())
                .map_err(|_| fail(StatusCode::SERVICE_UNAVAILABLE, "CREDENTIAL_UNAVAILABLE"))?;
            (new_credential(username), Some(blob))
        }
        ConnectionAccount::DomainAccount {
            credential_id,
            expected_revision,
        } => {
            let scope_error = || {
                fail(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "DOMAIN_ACCOUNT_SCOPE_MISMATCH",
                )
            };
            let domain = domain.ok_or_else(scope_error)?;
            if input.kind != ProfileKind::Execution
                || input.winrm_authentication != Some(WinRmAuthentication::Kerberos)
                || !store
                    .device_allows_domain_account(&host, domain)
                    .map_err(db_error)?
            {
                return Err(scope_error());
            }
            let (source, blob) = store.credential_secret(credential_id).map_err(db_error)?;
            if source.revision != expected_revision {
                return Err(db_error(Error::Conflict));
            }
            if source.owner_sid != api.sid || !source.is_domain_account(domain) {
                return Err(scope_error());
            }
            (
                new_credential(source.username),
                Some(zeroize::Zeroizing::new(blob)),
            )
        }
        ConnectionAccount::SavedTarget {
            credential_id,
            expected_revision,
        } => {
            let (source, _) = store.credential_secret(credential_id).map_err(db_error)?;
            if source.revision != expected_revision {
                return Err(db_error(Error::Conflict));
            }
            if source.owner_sid != api.sid || source.kind != input.kind || source.target != host {
                return Err(fail(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "CREDENTIAL_SCOPE_MISMATCH",
                ));
            }
            (source, None)
        }
    };
    let connection = ConnectionProfile {
        id: input.id,
        revision: 1,
        label: host.clone(),
        kind: input.kind,
        host,
        port: if input.kind == ProfileKind::AdRead {
            636
        } else {
            5986
        },
        credential_id: credential.id,
        winrm_authentication: input.winrm_authentication,
    };
    if let Some(blob) = blob {
        let configured = ConfiguredConnection {
            credential,
            connection,
        };
        store
            .configure_connection(actor, &configured, &blob)
            .map_err(db_error)?;
        Ok(configured)
    } else {
        let cached = store
            .connections()
            .map_err(db_error)?
            .into_iter()
            .find(|p| {
                p.credential_id == connection.credential_id
                    && p.host == connection.host
                    && p.kind == connection.kind
                    && p.winrm_authentication.unwrap_or_default()
                        == connection.winrm_authentication.unwrap_or_default()
            });
        let connection = match cached {
            Some(profile) => profile,
            None => store
                .save_connection(actor, &connection, None)
                .map_err(db_error)?,
        };
        Ok(ConfiguredConnection {
            credential,
            connection,
        })
    }
}

fn locate_domain_controller(
    domain: &str,
    stopping: &std::sync::atomic::AtomicBool,
) -> Result<String, Failure> {
    let domain = canonical_host(domain)
        .ok()
        .filter(|d| {
            d.contains('.')
                && !d.contains('_')
                && !d.bytes().all(|b| b.is_ascii_digit() || b == b'.')
        })
        .ok_or_else(|| fail(StatusCode::UNPROCESSABLE_ENTITY, "INVALID_DOMAIN"))?;
    let unavailable = || {
        fail(
            StatusCode::UNPROCESSABLE_ENTITY,
            "LDAPS_CONTROLLER_NOT_FOUND",
        )
    };
    let response = crate::probe::invoke(
        include_str!("locate-dc.ps1"),
        zeroize::Zeroizing::new(domain.as_bytes().to_vec()),
        crate::probe::QueryLimits {
            deadline: std::time::Duration::from_secs(12),
            max_output: 254,
            stopping: Some(stopping),
        },
    )
    .map_err(|_| unavailable())?;
    let result = std::str::from_utf8(&response).map_err(|_| unavailable())?;
    let host = canonical_host(result).map_err(|_| unavailable())?;
    if !host.ends_with(&format!(".{domain}")) {
        return Err(unavailable());
    }
    Ok(host)
}

fn redact_devices(devices: &mut [orkestr_domain::DeviceRecord]) {
    for record in devices {
        record.device.inventory_metadata = serde_json::Value::Null;
        if let Some(scan) = &mut record.scan {
            scan.details = serde_json::Value::Null;
        }
    }
}
fn bounded_json<T: Serialize>(value: &T) -> Result<Response, Failure> {
    let bytes = serde_json::to_vec(value)
        .map_err(|_| fail(StatusCode::INTERNAL_SERVER_ERROR, "SERIALIZATION_FAILED"))?;
    if bytes.len() > 8 * 1024 * 1024 {
        return Err(fail(StatusCode::SERVICE_UNAVAILABLE, "SNAPSHOT_TOO_LARGE"));
    }
    Ok(([("content-type", "application/json")], bytes).into_response())
}

pub async fn workers(
    api: Api,
    receiver: mpsc::Receiver<Uuid>,
    shutdown: tokio::sync::watch::Receiver<bool>,
) -> Vec<tokio::task::JoinHandle<()>> {
    let receiver = Arc::new(tokio::sync::Mutex::new(receiver));
    let mut handles = vec![];
    let dispatch_api = api.clone();
    let mut dispatch_shutdown = shutdown.clone();
    handles.push(tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = dispatch_shutdown.changed() => break,
                _ = tokio::time::sleep(std::time::Duration::from_secs(1)) => {}
            }
            let api = dispatch_api.clone();
            let pending = tokio::task::spawn_blocking(move || {
                crate::scheduling::tick(&api);
                api.store
                    .lock()
                    .ok()
                    .and_then(|store| store.pending_server_work().ok())
                    .unwrap_or_default()
            })
            .await
            .unwrap_or_default();
            let mut dispatched = None;
            for id in pending {
                if dispatch_api.jobs.try_send(id).is_err() {
                    break;
                }
                dispatched = Some(id);
            }
            if let Some(id) = dispatched {
                let api = dispatch_api.clone();
                let _ = tokio::task::spawn_blocking(move || {
                    if let Ok(mut store) = api.store.lock() {
                        let _ = store.record_dispatch(id);
                    }
                })
                .await;
            }
        }
    }));
    for _ in 0..2 {
        let receiver = receiver.clone();
        let api = api.clone();
        let mut shutdown = shutdown.clone();
        handles.push(tokio::spawn(async move {
            loop {
                let id = tokio::select! {
                    biased;
                    _=shutdown.changed()=>break,
                    id=async { receiver.lock().await.recv().await }=>id,
                };
                let Some(id) = id else {
                    break;
                };
                let api = api.clone();
                let _ =
                    tokio::task::spawn_blocking(move || {
                        let library = api.store.lock().ok().is_some_and(|store| {
                            store.library_job(id).is_ok_and(|job| job.is_some())
                        });
                        if library {
                            crate::library::execute(&api, id);
                            return;
                        }
                        let workflow = api.store.lock().ok().is_some_and(|store| {
                            store.workflow_run(id).is_ok_and(|run| run.is_some())
                        });
                        if workflow {
                            crate::workflow::execute(&api, id);
                            return;
                        }
                        let deploy = api.store.lock().ok().is_some_and(|store| {
                            store.deployment(id).is_ok_and(|record| record.is_some())
                        });
                        if deploy {
                            crate::deployment::execute(&api, id);
                        } else {
                            execute(&api, id);
                        }
                    })
                    .await;
            }
        }));
    }
    handles
}
fn execute(api: &Api, id: Uuid) {
    let prepared = (|| {
        let mut store = api.store.lock().ok()?;
        let mut job = store.job(id).ok()?;
        if job.state != JobState::Queued {
            return None;
        }
        let result = (|| {
            // Revocation/expiry also blocks queued jobs. No fallback service credentials.
            if !store.operators().ok()?.iter().any(|a| {
                a.id == job.actor_id
                    && a.expires_at > chrono::Utc::now()
                    && (a.role == Role::Admin
                        || (a.role == Role::Operator && job.task == InventoryTask::TestConnection))
            }) {
                return None;
            }
            let connection = store.connection(job.connection_id).ok()??;
            let (credential, blob) = store.credential_secret(connection.credential_id).ok()?;
            if store.job_revisions(id).ok()? != (connection.revision, credential.revision)
                || credential.kind != connection.kind
                || credential.target != connection.host
                || !inventory_target_matches(&store, &job.task, &connection).ok()?
            {
                return None;
            }
            let secret = orkestr_credentials::unprotect(&credential.owner_sid, &blob).ok()?;
            Some((connection, credential, secret))
        })();
        if result.is_none() {
            job.state = JobState::Failed;
            job.code = "CREDENTIAL_OR_AUTHORIZATION_UNAVAILABLE".into();
            let _ = store.set_job(&job);
            return None;
        }
        job.state = JobState::Running;
        let site = match job.task {
            InventoryTask::ScanDevice { device_id } => store
                .device(device_id)
                .ok()
                .flatten()
                .map(|d| d.site)
                .unwrap_or_default(),
            _ => "directory".into(),
        };
        if !store.acquire_resource(id, &site, Phase::ReadOnly).ok()? {
            return None;
        }
        job.code = "READ_ONLY_TEST_RUNNING".into();
        if store.set_job(&job).is_err() {
            let _ = store.release_resource(id);
            return None;
        }
        Some((job, result?))
    })();
    let Some((mut job, (connection, credential, secret))) = prepared else {
        return;
    };
    let _reservation = crate::scheduling::Reservation::new(api, id);
    match job.task.clone() {
        InventoryTask::TestConnection => {
            let code = crate::probe::run(&connection, &credential, secret);
            finish_job(&mut job, code == "READ_ONLY_OK", code);
        }
        InventoryTask::DiscoverOu { base_dn } => {
            let result = crate::inventory::discover(
                &connection,
                &credential,
                secret,
                &base_dn,
                &api.stopping,
            );
            if let Ok(mut store) = api.store.lock() {
                let authorized = store.operators().is_ok_and(|actors| {
                    actors.iter().any(|a| {
                        a.id == job.actor_id
                            && a.role == Role::Admin
                            && a.expires_at > chrono::Utc::now()
                    })
                });
                let result = result.and_then(|batch| {
                    if !authorized {
                        return Err("CREDENTIAL_OR_AUTHORIZATION_UNAVAILABLE");
                    }
                    store
                        .prepare_discovery_batch(job.actor_id, &batch)
                        .map_err(|_| "DISCOVERY_PREVIEW_UNAVAILABLE")
                });
                match result {
                    Ok(preview) => {
                        job.discovery = Some(preview);
                        finish_job(&mut job, true, "DISCOVERY_PREVIEW_READY");
                    }
                    Err(code) => finish_job(&mut job, false, code),
                }
                let _ = store.set_job(&job);
            }
            return;
        }
        InventoryTask::ScanDevice { device_id } => {
            let report = crate::inventory::scan(
                &connection,
                &credential,
                secret,
                device_id,
                job.id,
                &api.stopping,
            );
            if let Ok(mut store) = api.store.lock() {
                if inventory_target_matches(&store, &job.task, &connection).unwrap_or(false) {
                    if let Ok((report, diagnostic)) = report {
                        let (success, code) = match report.state() {
                            InventoryState::Complete => (true, "INVENTORY_COMPLETE"),
                            InventoryState::Partial => (true, "INVENTORY_PARTIAL"),
                            InventoryState::Failed => {
                                (false, diagnostic.unwrap_or("INVENTORY_FAILED"))
                            }
                            InventoryState::Unknown => (false, "INVENTORY_UNKNOWN"),
                        };
                        finish_job(&mut job, success, code);
                        if store.complete_inventory_job(&report, &job).is_ok() {
                            return;
                        }
                    }
                    finish_job(&mut job, false, "INVENTORY_REPORT_UNAVAILABLE");
                } else {
                    finish_job(&mut job, false, "INVENTORY_TARGET_CHANGED");
                }
                let _ = store.set_job(&job);
            }
            return;
        }
    }
    if let Ok(mut store) = api.store.lock() {
        let _ = store.set_job(&job);
    }
}

fn finish_job(job: &mut Job, succeeded: bool, code: &str) {
    job.state = if succeeded {
        JobState::Succeeded
    } else {
        JobState::Failed
    };
    job.code = code.into();
}
