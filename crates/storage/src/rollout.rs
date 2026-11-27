use crate::{server::audit, store::*, Error, Result, Store};
use orkestr_domain::PackageRevisionId;
use orkestr_protocol::*;
use rusqlite::{params, OptionalExtension, TransactionBehavior};
use uuid::Uuid;
impl Store {
    pub fn existing_rollout(&self, actor: Uuid, key: &str) -> Result<Option<Rollout>> {
        let data: Option<String> = self
            .db
            .query_row(
                "SELECT data FROM rollouts WHERE actor_id=?1 AND idempotency=?2",
                params![actor.to_string(), key],
                |r| r.get(0),
            )
            .optional()?;
        data.map(|data| serde_json::from_str(&data).map_err(Error::from))
            .transpose()
    }
    pub fn deployment_schedule_allows(
        &self,
        id: Uuid,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<bool> {
        // Admission must inspect every parent, including terminal history beyond
        // the bounded UI list; a cancelled old wave can never become standalone.
        let data:Option<String>=self.db.query_row("SELECT data FROM rollouts WHERE EXISTS(SELECT 1 FROM json_each(rollouts.data,'$.targets') AS target WHERE json_extract(target.value,'$.deployment_id')=?1) LIMIT 1",[id.to_string()],|r|r.get(0)).optional()?;
        if let Some(data) = data {
            let run: Rollout = serde_json::from_str(&data)?;
            if let Some(index) = run
                .targets
                .iter()
                .position(|target| target.deployment_id == Some(id))
            {
                return Ok(
                    matches!(run.state, RolloutState::Pilot | RolloutState::Production)
                        && (run.state != RolloutState::Pilot || index < run.pilot_count)
                        && run
                            .policy
                            .window
                            .as_ref()
                            .is_none_or(|window| window.contains(now)),
                );
            }
        }
        Ok(true)
    }
    pub fn package_source_current(&self, id: PackageRevisionId) -> Result<bool> {
        let package = self
            .packages()?
            .into_iter()
            .find(|p| p.id == id)
            .ok_or(Error::Invalid("package unavailable"))?;
        let Some(id) = package.metadata["release_id"].as_str() else {
            return Ok(true);
        };
        let id = Uuid::parse_str(id).map_err(|_| Error::Invalid("invalid release origin"))?;
        let Some(release) = self.library_release(id)? else {
            return Ok(false);
        };
        let Some(application) = self.library_application(release.application_id)? else {
            return Ok(false);
        };
        let check:Option<String>=self.db.query_row("SELECT data FROM library_jobs WHERE json_extract(data,'$.task.kind')='Check' AND json_extract(data,'$.task.application_id')=?1 AND json_extract(data,'$.task.revision')=?2 AND state IN ('Succeeded','Failed','Interrupted') ORDER BY rowid DESC LIMIT 1",params![application.id.to_string(),application.revision],|row|row.get(0)).optional()?;
        if let Some(data) = check {
            let job: LibraryJob = serde_json::from_str(&data)?;
            if job.state != LibraryJobState::Succeeded {
                return Ok(false);
            }
        }
        Ok(release.state == ReleaseState::Approved
            && application.revision == release.application_revision
            && package.artifact_hashes == release.sha256.into_iter().collect::<Vec<_>>())
    }
    pub fn policy_owner(&self, id: Uuid, revision: u64) -> Result<Uuid> {
        let actor: String = self.db.query_row(
            "SELECT actor_id FROM policy_history WHERE id=?1 AND revision=?2",
            params![id.to_string(), revision],
            |r| r.get(0),
        )?;
        Uuid::parse_str(&actor).map_err(|_| Error::Invalid("invalid policy owner"))
    }
    pub fn policy_check_due(&self, id: Uuid) -> Result<String> {
        Ok(self.db.query_row(
            "SELECT next_check FROM policy_clock WHERE id=?1",
            [id.to_string()],
            |r| r.get(0),
        )?)
    }
    pub fn set_policy_check_due(
        &mut self,
        id: Uuid,
        next: chrono::DateTime<chrono::Utc>,
    ) -> Result<()> {
        self.db.execute(
            "UPDATE policy_clock SET next_check=?2 WHERE id=?1",
            params![id.to_string(), next.to_rfc3339()],
        )?;
        Ok(())
    }
    pub fn rollouts(&self) -> Result<Vec<Rollout>> {
        json_rows(
            &self.db,
            "SELECT data FROM rollouts ORDER BY (json_extract(data,'$.state') NOT IN ('Succeeded','Cancelled')) DESC,rowid DESC LIMIT 256",
        )
    }
    pub fn rollout(&self, id: Uuid) -> Result<Option<Rollout>> {
        let data: Option<String> = self
            .db
            .query_row(
                "SELECT data FROM rollouts WHERE id=?1",
                [id.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        data.map(|s| serde_json::from_str(&s).map_err(Error::from))
            .transpose()
    }
    pub fn create_rollout(
        &mut self,
        actor: Uuid,
        key: &str,
        policy: Policy,
        targets: Vec<RolloutTarget>,
    ) -> Result<Rollout> {
        let prior: Option<String> = self
            .db
            .query_row(
                "SELECT data FROM rollouts WHERE actor_id=?1 AND idempotency=?2",
                params![actor.to_string(), key],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(data) = prior {
            let run: Rollout = serde_json::from_str(&data)?;
            if run.policy.id != policy.id || run.policy.revision != policy.revision {
                return Err(Error::Conflict);
            }
            return Ok(run);
        }
        if targets.is_empty() || targets.len() > 10000 {
            return Err(Error::Invalid("invalid rollout targets"));
        }
        let run = Rollout {
            id: Uuid::new_v4(),
            revision: 1,
            actor_id: actor,
            package_revision_id: policy.package_revision_id,
            pilot_count: (policy.pilot_count as usize).min(targets.len()),
            policy,
            targets,
            state: RolloutState::WaitingApproval,
            code: "WAITING_APPROVAL".into(),
            soak_until: None,
            created_at: chrono::Utc::now(),
        };
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let active: u64 = tx.query_row("SELECT count(*) FROM rollouts WHERE json_extract(data,'$.state') NOT IN ('Succeeded','Cancelled')", [], |r|r.get(0))?;
        if active >= 128 {
            return Err(Error::Invalid("active rollout capacity"));
        }
        tx.execute(
            "INSERT INTO rollouts VALUES(?1,?2,?3,1,?4)",
            params![
                run.id.to_string(),
                actor.to_string(),
                key,
                serde_json::to_string(&run)?
            ],
        )?;
        audit(
            &tx,
            &actor.to_string(),
            "rollout.plan",
            &run.id.to_string(),
            "TARGETS_SNAPSHOTTED",
        )?;
        tx.commit()?;
        Ok(run)
    }
    pub fn cancel_rollout_children(&mut self, actor: Uuid, run: &Rollout) -> Result<()> {
        for id in run.targets.iter().filter_map(|t| t.deployment_id) {
            if let Some(mut child) = self.deployment(id)? {
                if matches!(
                    child.state,
                    DeploymentState::WaitingApproval | DeploymentState::Queued
                ) {
                    child.state = DeploymentState::Cancelled;
                    child.code = "PARENT_ROLLOUT_CANCELLED".into();
                    self.update_deployment(&mut child, actor)?;
                }
            } else if let Some(mut child) = self.workflow_run(id)? {
                if matches!(
                    child.state,
                    DeploymentState::WaitingApproval | DeploymentState::Queued
                ) {
                    child.state = DeploymentState::Cancelled;
                    child.code = "PARENT_ROLLOUT_CANCELLED".into();
                    self.update_workflow(actor, &mut child)?;
                } else if child.state == DeploymentState::Running && !child.cancel_requested {
                    child.cancel_requested = true;
                    self.update_workflow(actor, &mut child)?;
                }
            }
        }
        Ok(())
    }
    pub fn update_rollout(&mut self, actor: Uuid, run: &mut Rollout) -> Result<()> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut updated = run.clone();
        updated.revision += 1;
        if tx.execute(
            "UPDATE rollouts SET revision=?2,data=?3 WHERE id=?1 AND revision=?4",
            params![
                run.id.to_string(),
                updated.revision,
                serde_json::to_string(&updated)?,
                run.revision
            ],
        )? != 1
        {
            return Err(Error::Conflict);
        }
        audit(
            &tx,
            &actor.to_string(),
            "rollout.state",
            &run.id.to_string(),
            &run.code,
        )?;
        tx.commit()?;
        run.revision = updated.revision;
        Ok(())
    }
}
