use crate::api::Api;
use chrono::Utc;
use orkestr_protocol::*;
use std::{
    io::Read,
    sync::atomic::Ordering,
    time::{Duration, Instant},
};
use uuid::Uuid;

#[derive(serde::Deserialize)]
pub(crate) struct StepResult {
    #[serde(default)]
    transfer: Option<TransferStatistics>,
    pub(crate) state: String,
    exit_code: Option<i32>,
    pub(crate) evidence: Option<String>,
}
pub(crate) fn execute(api: &Api, id: Uuid) {
    let prepared = (|| {
        let mut store = api.store.lock().ok()?;
        let mut run = store.workflow_run(id).ok()??;
        let reconcile = run.state == DeploymentState::ReconcileQueued;
        if run.state != DeploymentState::Queued && !reconcile {
            return None;
        }
        let actor = run.action_by?;
        if !reconcile && !store.deployment_schedule_allows(id, Utc::now()).ok()? {
            return None;
        }
        if !store
            .operators()
            .ok()?
            .iter()
            .any(|a| a.id == actor && a.role == Role::Admin && a.expires_at > Utc::now())
        {
            run.state = if reconcile {
                DeploymentState::NeedsReconciliation
            } else {
                DeploymentState::Failed
            };
            run.code = "AUTHORIZATION_CHANGED".into();
            let _ = store.update_workflow(actor, &mut run);
            return None;
        }
        let target = crate::deployment::target(&store, &run.plan, Some(&run.host), reconcile);
        let valid = target.as_ref().is_ok_and(|(_, credential, _)| {
            reconcile || credential.revision == run.credential_revision
        });
        if !valid {
            run.state = if reconcile {
                DeploymentState::NeedsReconciliation
            } else {
                DeploymentState::Failed
            };
            run.code = "WORKFLOW_SCOPE_CHANGED".into();
            let _ = store.update_workflow(actor, &mut run);
            return None;
        }
        let (connection, credential, blob) = target.ok()?;
        let site = store.device(run.plan.device_id).ok()??.site;
        if !store.acquire_resource(id, &site, Phase::Install).ok()? {
            return None;
        }
        run.state = if reconcile {
            DeploymentState::Reconciling
        } else {
            DeploymentState::Running
        };
        run.code = "WORKFLOW_RUNNING".into();
        if store.update_workflow(actor, &mut run).is_err() {
            let _ = store.release_resource(id);
            return None;
        }
        Some((run, actor, connection, credential, blob, reconcile))
    })();
    let Some((mut run, actor, connection, credential, blob, reconcile)) = prepared else {
        return;
    };
    loop {
        if run.next_step >= run.definition.steps.len() {
            run.state = if run.reboot_required {
                DeploymentState::RebootRequired
            } else {
                DeploymentState::Succeeded
            };
            run.code = "WORKFLOW_COMPLETE".into();
            break;
        }
        if api.stopping.load(Ordering::Relaxed) {
            run.state = DeploymentState::NeedsReconciliation;
            run.code = "WORKFLOW_INTERRUPTED".into();
            break;
        }
        let step = run.definition.steps[run.next_step].clone();
        if !reconcile {
            let authorized = api.store.lock().is_ok_and(|store| {
                store.operators().is_ok_and(|actors| {
                    actors.iter().any(|a| {
                        a.id == actor && a.role == Role::Admin && a.expires_at > Utc::now()
                    })
                }) && crate::deployment::target(&store, &run.plan, Some(&run.host), false)
                    .is_ok_and(|(_, credential, _)| credential.revision == run.credential_revision)
            });
            if !authorized {
                run.state = DeploymentState::Failed;
                run.code = "AUTHORIZATION_CHANGED_BEFORE_STEP".into();
                break;
            }
            if let Ok(store) = api.store.lock() {
                if let Ok(Some(latest)) = store.workflow_run(id) {
                    run.cancel_requested = latest.cancel_requested;
                    run.revision = latest.revision;
                }
            }
            if run.cancel_requested {
                run.state = DeploymentState::Cancelled;
                run.code = "CANCELLED_AFTER_STEP".into();
                break;
            }
            run.checkpoints.push(StepCheckpoint {
                index: run.next_step,
                started_at: Utc::now(),
                finished_at: None,
                code: "STEP_STARTED".into(),
                exit_code: None,
                evidence: None,
                transfer: None,
            });
            if !save(api, actor, &mut run) {
                break;
            }
        }
        let result = perform(api, &run, &step, &connection, &credential, &blob, reconcile);
        match result.state.as_str() {
            "Succeeded" | "RebootRequired" => {
                if matches!(step, WorkflowStep::Reboot { .. }) && result.state == "Succeeded" {
                    run.reboot_required = false;
                }
                if result.state == "RebootRequired" {
                    run.reboot_required = true;
                }
                if let Some(checkpoint) = run.checkpoints.last_mut() {
                    checkpoint.finished_at = Some(Utc::now());
                    checkpoint.code = "STEP_CONFIRMED".into();
                    checkpoint.exit_code = result.exit_code;
                    checkpoint.evidence = result.evidence;
                    checkpoint.transfer = result.transfer;
                }
                run.next_step += 1;
                if reconcile {
                    run.state = DeploymentState::WaitingApproval;
                    run.code = "STEP_RECONCILED_REAPPROVAL_REQUIRED".into();
                    break;
                }
                if !save(api, actor, &mut run) {
                    break;
                }
            }
            "Failed" | "BeforeStep" => {
                if let Some(checkpoint) = run.checkpoints.last_mut() {
                    checkpoint.finished_at = Some(Utc::now());
                    checkpoint.code = "STEP_FAILED".into();
                    checkpoint.exit_code = result.exit_code;
                    checkpoint.evidence = result.evidence;
                    checkpoint.transfer = result.transfer;
                }
                run.state = DeploymentState::Failed;
                run.code = if result.state == "BeforeStep" {
                    "FAILED_BEFORE_STEP"
                } else {
                    "STEP_FAILED"
                }
                .into();
                break;
            }
            _ => {
                if let Some(checkpoint) = run.checkpoints.last_mut() {
                    checkpoint.evidence = result.evidence;
                    checkpoint.code = result.state;
                }
                run.state = DeploymentState::NeedsReconciliation;
                run.code = "STEP_NEEDS_RECONCILIATION".into();
                break;
            }
        }
    }
    let _ = save(api, actor, &mut run);
    if let Ok(mut store) = api.store.lock() {
        let _ = store.release_resource(id);
    }
}
fn save(api: &Api, actor: Uuid, run: &mut WorkflowRun) -> bool {
    api.store.lock().is_ok_and(|mut store| {
        if let Ok(Some(latest)) = store.workflow_run(run.id) {
            if latest.revision != run.revision
                && latest.state == DeploymentState::Running
                && latest.cancel_requested
            {
                run.revision = latest.revision;
                run.cancel_requested = true;
            }
        }
        store.update_workflow(actor, run).is_ok()
    })
}
fn perform(
    api: &Api,
    run: &WorkflowRun,
    step: &WorkflowStep,
    connection: &ConnectionProfile,
    credential: &CredentialProfile,
    blob: &[u8],
    reconcile: bool,
) -> StepResult {
    let unknown = || StepResult {
        state: "Unknown".into(),
        exit_code: None,
        evidence: None,
        transfer: None,
    };
    let Ok(secret) = orkestr_credentials::unprotect(&credential.owner_sid, blob) else {
        return StepResult {
            state: "BeforeStep".into(),
            exit_code: None,
            evidence: None,
            transfer: None,
        };
    };
    match step {
        WorkflowStep::Sleep { seconds } => {
            let remaining = if reconcile {
                run.checkpoints
                    .last()
                    .map(|c| {
                        (c.started_at + chrono::Duration::seconds(*seconds as i64) - Utc::now())
                            .num_milliseconds()
                            .max(0) as u64
                    })
                    .unwrap_or(0)
            } else {
                *seconds as u64 * 1000
            };
            let started = Instant::now();
            while started.elapsed() < Duration::from_millis(remaining) {
                if api.stopping.load(Ordering::Relaxed) {
                    return unknown();
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            StepResult {
                state: "Succeeded".into(),
                exit_code: None,
                evidence: None,
                transfer: None,
            }
        }
        WorkflowStep::SystemScan {} => {
            match crate::inventory::scan(
                connection,
                credential,
                secret,
                run.plan.device_id,
                Uuid::new_v4(),
                &api.stopping,
            ) {
                Ok((inventory, diagnostic)) => record_scan(api, &inventory, diagnostic),
                _ => StepResult {
                    state: "Failed".into(),
                    exit_code: None,
                    evidence: Some("INVENTORY_NORMALIZATION_FAILED".into()),
                    transfer: None,
                },
            }
        }
        WorkflowStep::Install { installer } => {
            let record = Deployment {
                manual_resolution: None,
                id: run.id,
                revision: run.revision,
                plan: run.plan.clone(),
                installer: installer.clone(),
                host: run.host.clone(),
                created_by: run.created_by,
                approved_by: run.action_by,
                approved_at: None,
                action_by: run.action_by,
                credential_revision: run.credential_revision,
                state: run.state,
                code: run.code.clone(),
                created_at: run.created_at,
                attempts: vec![],
                reconciliation: None,
            };
            let result = crate::deployment::run_adapter(
                api, &record, connection, credential, secret, reconcile,
            );
            crate::deployment::workflow_result(result, reconcile)
        }
        WorkflowStep::NestedPackage { .. } => unknown(),
        _ => {
            let mut held = None;
            let mut path = None;
            if let WorkflowStep::FileCopy { sha256, .. } = step {
                if !reconcile {
                    let Some(root) = &api.artifact_directory else {
                        return unknown();
                    };
                    let file_path = root.join(format!("{sha256}.bin"));
                    let Ok(mut file) = crate::deployment::open_artifact(&file_path) else {
                        return StepResult {
                            state: "BeforeStep".into(),
                            exit_code: None,
                            evidence: None,
                            transfer: None,
                        };
                    };
                    use sha2::Digest;
                    let mut hash = sha2::Sha256::new();
                    let mut bytes = [0; 65536];
                    loop {
                        match file.read(&mut bytes) {
                            Ok(0) => break,
                            Ok(n) => hash.update(&bytes[..n]),
                            Err(_) => return unknown(),
                        }
                    }
                    if format!("{:x}", hash.finalize()) != *sha256 {
                        return StepResult {
                            state: "BeforeStep".into(),
                            exit_code: None,
                            evidence: None,
                            transfer: None,
                        };
                    }
                    path = Some(file_path);
                    held = Some(file);
                }
            }
            let timeout = match step {
                WorkflowStep::Command {
                    timeout_seconds, ..
                }
                | WorkflowStep::PowerShell {
                    timeout_seconds, ..
                } => *timeout_seconds as u64,
                _ => 180,
            };
            let options = serde_json::json!({"step":step,"path":path,"reconcile":reconcile,"bytes_per_second":crate::deployment::transfer_rate(api),"evidence":run.checkpoints.last().and_then(|c|c.evidence.as_ref())});
            let result = crate::probe::query(
                &format!(
                    "{}\n{}",
                    include_str!("transfer.ps1"),
                    include_str!("workflow-step.ps1")
                ),
                connection,
                credential,
                secret,
                &options,
                crate::probe::QueryLimits {
                    deadline: Duration::from_secs(timeout),
                    max_output: 4096,
                    stopping: Some(&api.stopping),
                },
            );
            drop(held);
            result
                .ok()
                .and_then(|bytes| serde_json::from_slice(&bytes).ok())
                .unwrap_or_else(unknown)
        }
    }
}
pub(crate) fn record_scan(
    api: &Api,
    inventory: &orkestr_inventory::NormalizedInventory,
    diagnostic: Option<&str>,
) -> StepResult {
    let state = inventory.state();
    let saved = api
        .store
        .lock()
        .is_ok_and(|mut store| store.record_inventory(inventory).is_ok());
    let code = if !saved {
        "INVENTORY_SAVE_FAILED"
    } else {
        diagnostic.unwrap_or(match state {
            InventoryState::Complete => "INVENTORY_COMPLETE",
            InventoryState::Partial => "INVENTORY_PARTIAL",
            _ => "INVENTORY_SCAN_FAILED",
        })
    };
    StepResult {
        state: if !saved {
            "Unknown"
        } else if matches!(state, InventoryState::Complete | InventoryState::Partial) {
            "Succeeded"
        } else {
            "Failed"
        }
        .into(),
        exit_code: None,
        transfer: None,
        evidence: Some(format!("{code}; report={}", inventory.report().id)),
    }
}
pub(crate) fn step_result(state: &str, exit_code: Option<i32>) -> StepResult {
    StepResult {
        state: state.into(),
        exit_code,
        evidence: None,
        transfer: None,
    }
}
