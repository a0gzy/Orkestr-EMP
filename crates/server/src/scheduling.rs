use crate::api::Api;
use chrono::Utc;
use orkestr_application::{preview, Selection};
use orkestr_protocol::*;
use orkestr_storage::{Error, Result, Store};
use uuid::Uuid;
pub(crate) struct Reservation {
    api: Api,
    id: Uuid,
}
impl Reservation {
    pub fn new(api: &Api, id: Uuid) -> Self {
        Self {
            api: api.clone(),
            id,
        }
    }
}
impl Drop for Reservation {
    fn drop(&mut self) {
        if let Ok(mut store) = self.api.store.lock() {
            let _ = store.release_resource(self.id);
        }
    }
}

pub(crate) fn plan(
    store: &mut Store,
    actor: Uuid,
    key: &str,
    input: &PlanRollout,
) -> Result<Rollout> {
    if let Some(run) = store.existing_rollout(actor, key)? {
        if run.policy.id != input.policy_id
            || run.policy.revision != input.policy_revision
            || input
                .package_revision_id
                .is_some_and(|id| id != run.package_revision_id)
        {
            return Err(Error::Conflict);
        }
        return Ok(run);
    }
    let (policy, targets) = prepare(store, input)?;
    store.create_rollout(actor, key, policy, targets)
}
pub(crate) fn prepare(
    store: &mut Store,
    input: &PlanRollout,
) -> Result<(Policy, Vec<RolloutTarget>)> {
    let mut policy = store
        .policies()?
        .into_iter()
        .find(|p| p.id == input.policy_id && p.revision == input.policy_revision)
        .ok_or(Error::Conflict)?;
    if let Some(selected) = input.package_revision_id {
        if !crate::automation::override_allowed(store, &policy, selected)? {
            return Err(Error::Invalid("incompatible policy package"));
        }
        policy.package_revision_id = selected;
    }
    if !store.package_source_current(policy.package_revision_id)?
        || (store
            .managed_installer(policy.package_revision_id)?
            .is_none()
            && store
                .workflow_definition(policy.package_revision_id)?
                .is_none())
    {
        return Err(Error::Invalid("approved managed package required"));
    }
    let preview = preview(store, &Selection::Group(policy.collection_id))?;
    if preview.generation != input.generation {
        return Err(Error::Conflict);
    }
    let connections = store.connections()?;
    let mut targets = vec![];
    for member in preview
        .members
        .into_iter()
        .filter(|member| !member.device.disabled)
    {
        let profiles: Vec<_> = connections
            .iter()
            .filter(|p| {
                p.kind == ProfileKind::Execution && p.host == member.device.host && p.port == 5986
            })
            .collect();
        let [profile] = profiles.as_slice() else {
            return Err(Error::Invalid(
                "one exact execution profile per target required",
            ));
        };
        targets.push(RolloutTarget {
            device_id: member.device.id,
            connection_id: profile.id,
            connection_revision: profile.revision,
            deployment_id: None,
            retry_at: None,
        });
    }
    targets.sort_by_key(|target| target.device_id);
    Ok((policy, targets))
}
pub(crate) fn tick(api: &Api) {
    let Ok(mut store) = api.store.lock() else {
        return;
    };
    let now = Utc::now();
    let Ok(policies) = store.policies() else {
        return;
    };
    for policy in &policies {
        let Ok(actor) = store.policy_owner(policy.id, policy.revision) else {
            continue;
        };
        if !authorized(&store, actor) {
            continue;
        }
        if policy.automation.auto_check {
            if let Ok(due) = store.policy_check_due(policy.id) {
                if chrono::DateTime::parse_from_rfc3339(&due).is_ok_and(|time| time <= now) {
                    if let Ok(Some(app)) = store.library_application(policy.application_id) {
                        let key = format!("policy:{}:{}:check:{}", policy.id, policy.revision, due);
                        if store
                            .queue_library_job(
                                actor,
                                &key,
                                LibraryTask::Check {
                                    application_id: app.id,
                                    revision: app.revision,
                                },
                            )
                            .is_ok()
                        {
                            let next = now
                                + chrono::Duration::seconds(
                                    (policy.interval_seconds
                                        + (policy.id.as_u128() as u64)
                                            % (policy.jitter_seconds as u64 + 1))
                                        as i64,
                                );
                            let _ = store.set_policy_check_due(policy.id, next);
                        }
                    }
                }
            }
        }
        if policy.automation.auto_download {
            if let Ok(releases) = store.library_releases() {
                for release in releases.iter().filter(|r| {
                    r.application_id == policy.application_id && r.state == ReleaseState::Candidate
                }) {
                    let key = format!(
                        "policy:{}:{}:download:{}",
                        policy.id, policy.revision, release.id
                    );
                    let _ = store.queue_library_job(
                        actor,
                        &key,
                        LibraryTask::Download {
                            release_id: release.id,
                        },
                    );
                }
            }
        }
        if policy.automation.auto_deploy {
            let Ok(selected) = crate::automation::effective_package(&mut store, actor, policy)
            else {
                continue;
            };
            if !store.package_source_current(selected).unwrap_or(false) {
                continue;
            }
            let snapshot = store.snapshot();
            if let Ok(snapshot) = snapshot {
                let key = format!(
                    "policy:{}:{}:deploy:{}",
                    policy.id, policy.revision, selected
                );
                let input = PlanRollout {
                    package_revision_id: Some(selected),
                    policy_id: policy.id,
                    policy_revision: policy.revision,
                    generation: snapshot.generation,
                };
                if let Ok(mut run) = plan(&mut store, actor, &key, &input) {
                    if run.state == RolloutState::WaitingApproval && policy.automation.unattended {
                        // Unattended still requires an explicitly approved, published library artifact.
                        let library_package = store.packages().is_ok_and(|packages| {
                            packages.iter().any(|p| {
                                p.id == run.package_revision_id
                                    && p.metadata["release_id"].is_string()
                            })
                        });
                        if library_package {
                            run.state = RolloutState::Pilot;
                            run.code = "UNATTENDED_OPT_IN_PILOT".into();
                            let _ = store.update_rollout(actor, &mut run);
                        }
                    }
                }
            }
        }
    }
    let Ok(runs) = store.rollouts() else { return };
    for mut run in runs {
        if run.state == RolloutState::Cancelled {
            let _ = store.cancel_rollout_children(run.actor_id, &run);
            continue;
        }
        if !matches!(
            run.state,
            RolloutState::Pilot | RolloutState::Soaking | RolloutState::Production
        ) {
            continue;
        }
        let actor = run.actor_id;
        let unchanged = policies
            .iter()
            .any(|p| p.id == run.policy.id && p.revision == run.policy.revision);
        if !authorized(&store, actor)
            || !unchanged
            || !store
                .package_source_current(run.package_revision_id)
                .unwrap_or(false)
        {
            run.state = RolloutState::Paused;
            run.code = "ROLLOUT_SCOPE_OR_AUTHORIZATION_CHANGED".into();
            let _ = store.update_rollout(actor, &mut run);
            continue;
        }
        if run.state == RolloutState::Soaking {
            if run.soak_until.is_some_and(|until| now >= until) {
                run.state = RolloutState::Production;
                run.code = "PRODUCTION_ELIGIBLE".into();
                let _ = store.update_rollout(actor, &mut run);
            } else {
                continue;
            }
        }
        if run
            .policy
            .window
            .as_ref()
            .is_some_and(|window| !window.contains(now))
        {
            continue;
        }
        let count = if run.state == RolloutState::Pilot {
            run.pilot_count
        } else {
            run.targets.len()
        };
        let mut completed = 0usize;
        let mut failed = 0usize;
        let mut changed = false;
        for target in run.targets.iter_mut().take(count) {
            if let Some(id) = target.deployment_id {
                if let Ok(Some(mut deployment)) = store.deployment(id) {
                    match deployment.state {
                        DeploymentState::Succeeded | DeploymentState::RebootRequired => {
                            completed += 1
                        }
                        DeploymentState::NeedsReconciliation => {
                            run.state = RolloutState::Paused;
                            run.code = "TARGET_NEEDS_RECONCILIATION".into();
                            changed = true;
                            break;
                        }
                        DeploymentState::Failed => {
                            let retryable = (deployment.code == "RESTARTED_BEFORE_START"
                                || deployment.attempts.last().is_some_and(|a| a.safe_to_retry))
                                && deployment.attempts.len() < (run.policy.max_attempts as usize);
                            if retryable && run.policy.automation.unattended {
                                if target.retry_at.is_none() {
                                    target.retry_at = Some(orkestr_scheduler::retry_at(
                                        id,
                                        deployment.attempts.len() as u8,
                                        now,
                                        run.policy.jitter_seconds,
                                    ));
                                    changed = true;
                                }
                                if target.retry_at.is_some_and(|due| due <= now)
                                    && crate::deployment::transition(
                                        &mut store,
                                        &mut deployment,
                                        actor,
                                        "retry",
                                    )
                                    .is_ok()
                                {
                                    target.retry_at = None;
                                    changed = true;
                                    let _ = api.jobs.try_send(id);
                                }
                            } else {
                                completed += 1;
                                failed += 1;
                            }
                        }
                        DeploymentState::Cancelled => {
                            completed += 1;
                            failed += 1;
                        }
                        DeploymentState::WaitingApproval => {
                            let approved = crate::deployment::transition(
                                &mut store,
                                &mut deployment,
                                actor,
                                "approve",
                            )
                            .is_ok();
                            if approved {
                                let _ = api.jobs.try_send(id);
                            }
                        }
                        _ => {}
                    }
                } else if let Ok(Some(mut workflow)) = store.workflow_run(id) {
                    match workflow.state {
                        DeploymentState::Succeeded | DeploymentState::RebootRequired => {
                            completed += 1
                        }
                        DeploymentState::Failed | DeploymentState::Cancelled => {
                            completed += 1;
                            failed += 1;
                        }
                        DeploymentState::NeedsReconciliation => {
                            run.state = RolloutState::Paused;
                            run.code = "TARGET_NEEDS_RECONCILIATION".into();
                            changed = true;
                            break;
                        }
                        DeploymentState::WaitingApproval => {
                            if !crate::deployment::target(
                                &store,
                                &workflow.plan,
                                Some(&workflow.host),
                                false,
                            )
                            .is_ok_and(|(_, credential, _)| {
                                credential.revision == workflow.credential_revision
                            }) {
                                run.state = RolloutState::Paused;
                                run.code = "TARGET_SCOPE_CHANGED".into();
                                changed = true;
                                break;
                            }
                            workflow.state = DeploymentState::Queued;
                            workflow.action_by = Some(actor);
                            workflow.code = "ROLLOUT_APPROVED_QUEUED".into();
                            if store.update_workflow(actor, &mut workflow).is_ok() {
                                let _ = api.jobs.try_send(id);
                            }
                        }
                        _ => {}
                    }
                }
                continue;
            }
            let plan = PlanDeployment {
                device_id: target.device_id,
                package_revision_id: run.package_revision_id,
                connection_id: target.connection_id,
                connection_revision: target.connection_revision,
            };
            let key = format!("rollout:{}:{}", run.id, target.device_id);
            if store
                .workflow_definition(plan.package_revision_id)
                .ok()
                .flatten()
                .is_some()
            {
                let result = (|| {
                    let (connection, credential, _) =
                        crate::deployment::target(&store, &plan, None, false)?;
                    store.plan_workflow(actor, &key, plan, &connection, credential.revision)
                })();
                match result {
                    Ok(workflow) => {
                        target.deployment_id = Some(workflow.id);
                        changed = true;
                    }
                    Err(_) => {
                        run.state = RolloutState::Paused;
                        run.code = "TARGET_SCOPE_CHANGED".into();
                        changed = true;
                        break;
                    }
                }
                continue;
            }
            let result = (|| {
                if let Some(existing) = store.existing_deployment(actor, &key)? {
                    return Ok(existing);
                }
                let (connection, credential, _) =
                    crate::deployment::target(&store, &plan, None, false)?;
                let installer = store
                    .managed_installer(plan.package_revision_id)?
                    .ok_or(Error::Invalid("installer unavailable"))?;
                store.create_deployment(
                    actor,
                    &key,
                    plan,
                    &connection,
                    credential.revision,
                    installer,
                )
            })();
            if let Ok(deployment) = result {
                target.deployment_id = Some(deployment.id);
                changed = true;
            } else {
                run.state = RolloutState::Paused;
                run.code = "TARGET_SCOPE_CHANGED".into();
                changed = true;
                break;
            }
        }
        if failed > 0 && failed * 100 > completed.max(1) * (run.policy.max_failure_percent as usize)
        {
            run.state = RolloutState::Paused;
            run.code = "ERROR_THRESHOLD_EXCEEDED".into();
            changed = true;
        } else if completed == count && run.state == RolloutState::Pilot {
            run.state = RolloutState::Soaking;
            run.soak_until = Some(now + chrono::Duration::seconds(run.policy.soak_seconds as i64));
            run.code = "PILOT_SOAKING".into();
            changed = true;
        } else if completed == count && run.state == RolloutState::Production {
            run.state = RolloutState::Succeeded;
            run.code = "ROLLOUT_COMPLETE".into();
            changed = true;
        }
        if changed {
            let _ = store.update_rollout(actor, &mut run);
        }
    }
}
fn authorized(store: &Store, id: Uuid) -> bool {
    store.operators().is_ok_and(|actors| {
        actors
            .iter()
            .any(|a| a.id == id && a.role == Role::Admin && a.expires_at > Utc::now())
    })
}
