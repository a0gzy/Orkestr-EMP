use crate::{server::audit, store::*, Error, Result, Store};
use chrono::Utc;
use orkestr_domain::{PackageId, PackageRevision, PackageRevisionId, PackageStep, StepKind};
use orkestr_protocol::*;
use rusqlite::{params, OptionalExtension, TransactionBehavior};
use uuid::Uuid;

impl Store {
    pub fn register_installer(
        &mut self,
        actor: Uuid,
        installer: &ManagedInstaller,
    ) -> Result<PackageRevision> {
        self.register_installer_origin(actor, installer, serde_json::Value::Null)
    }
    pub fn register_installer_origin(
        &mut self,
        actor: Uuid,
        installer: &ManagedInstaller,
        origin: serde_json::Value,
    ) -> Result<PackageRevision> {
        if !installer.valid() {
            return Err(Error::Invalid("invalid installer execution"));
        }
        let package = PackageRevision {
            id: PackageRevisionId::new(),
            package_id: PackageId::new(),
            name: installer.name.clone(),
            version: installer.version.clone(),
            steps: vec![PackageStep {
                kind: StepKind::Install,
                definition: serde_json::to_value(installer)?,
            }],
            artifact_hashes: vec![installer.sha256.clone()],
            metadata: origin,
        };
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        write_package(&tx, &package)?;
        tx.execute(
            "INSERT INTO managed_installers VALUES(?1,?2)",
            params![package.id.to_string(), serde_json::to_string(installer)?],
        )?;
        audit(
            &tx,
            &actor.to_string(),
            "deploy.package",
            &package.id.to_string(),
            if installer.execution.is_msi() {
                "MSI_REVISION_REGISTERED"
            } else {
                "EXE_REVISION_REGISTERED"
            },
        )?;
        tx.commit()?;
        Ok(package)
    }

    pub fn managed_installer(&self, id: PackageRevisionId) -> Result<Option<ManagedInstaller>> {
        let data: Option<String> = self
            .db
            .query_row(
                "SELECT data FROM managed_installers WHERE revision_id=?1",
                [id.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        data.map(|data| serde_json::from_str(&data).map_err(Error::from))
            .transpose()
    }

    pub fn deployments(&self) -> Result<Vec<Deployment>> {
        json_rows(
            &self.db,
            "SELECT data FROM deployments ORDER BY holds_device DESC,rowid DESC LIMIT 128",
        )
    }

    pub fn deployment(&self, id: Uuid) -> Result<Option<Deployment>> {
        let data: Option<String> = self
            .db
            .query_row(
                "SELECT data FROM deployments WHERE id=?1",
                [id.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        data.map(|data| serde_json::from_str(&data).map_err(Error::from))
            .transpose()
    }

    pub fn existing_deployment(&self, actor: Uuid, key: &str) -> Result<Option<Deployment>> {
        let data: Option<String> = self
            .db
            .query_row(
                "SELECT data FROM deployments WHERE actor_id=?1 AND idempotency=?2",
                params![actor.to_string(), key],
                |r| r.get(0),
            )
            .optional()?;
        data.map(|data| serde_json::from_str(&data).map_err(Error::from))
            .transpose()
    }

    pub fn create_deployment(
        &mut self,
        actor: Uuid,
        key: &str,
        plan: PlanDeployment,
        connection: &ConnectionProfile,
        credential_revision: u64,
        installer: ManagedInstaller,
    ) -> Result<Deployment> {
        let record = Deployment {
            manual_resolution: None,
            id: Uuid::new_v4(),
            revision: 1,
            plan,
            installer,
            host: connection.host.clone(),
            created_by: actor,
            approved_by: None,
            approved_at: None,
            action_by: None,
            credential_revision,
            state: DeploymentState::WaitingApproval,
            code: "WAITING_APPROVAL".into(),
            created_at: Utc::now(),
            attempts: vec![],
            reconciliation: None,
        };
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT INTO deployments VALUES(?1,?2,?3,?4,1,0,?5)",
            params![
                record.id.to_string(),
                actor.to_string(),
                key,
                record.plan.device_id.to_string(),
                serde_json::to_string(&record)?
            ],
        )?;
        audit(
            &tx,
            &actor.to_string(),
            "deploy.plan",
            &record.id.to_string(),
            &record.code,
        )?;
        tx.commit()?;
        Ok(record)
    }

    pub fn update_deployment(&mut self, record: &mut Deployment, actor: Uuid) -> Result<()> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let held = record.state.holds_device();
        if held {
            let workflow_busy: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM workflow_runs WHERE device_id=?1 AND holds_device=1)",
                [record.plan.device_id.to_string()],
                |r| r.get(0),
            )?;
            if workflow_busy {
                return Err(Error::DeploymentDeviceBusy);
            }
            let busy: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM deployments WHERE device_id=?1 AND holds_device=1 AND id<>?2)",
                params![record.plan.device_id.to_string(),record.id.to_string()], |r| r.get(0))?;
            if busy {
                return Err(Error::DeploymentDeviceBusy);
            }
        }
        let expected = record.revision;
        let mut updated = record.clone();
        updated.revision += 1;
        if tx.execute("UPDATE deployments SET revision=?2,holds_device=?3,data=?4 WHERE id=?1 AND revision=?5",
            params![record.id.to_string(),updated.revision,held,serde_json::to_string(&updated)?,expected])? != 1 {
            return Err(Error::Conflict);
        }
        audit(
            &tx,
            &actor.to_string(),
            "deploy.state",
            &record.id.to_string(),
            &record.code,
        )?;
        tx.commit()?;
        record.revision = updated.revision;
        Ok(())
    }

    pub fn recover_deployments(&mut self) -> Result<()> {
        let active: Vec<Deployment> = json_rows(
            &self.db,
            "SELECT data FROM deployments WHERE holds_device=1",
        )?;
        for mut record in active {
            let (state, code) = match record.state {
                DeploymentState::Queued => (DeploymentState::Failed, "RESTARTED_BEFORE_START"),
                DeploymentState::Running
                | DeploymentState::ReconcileQueued
                | DeploymentState::Reconciling => (
                    DeploymentState::NeedsReconciliation,
                    "RESTARTED_NEEDS_RECONCILIATION",
                ),
                _ => continue,
            };
            if let Some(attempt) = record
                .attempts
                .last_mut()
                .filter(|a| a.finished_at.is_none())
            {
                attempt.finished_at = Some(Utc::now());
                attempt.code = code.into();
            }
            record.state = state;
            record.code = code.into();
            let actor = record.action_by.unwrap_or(record.created_by);
            self.update_deployment(&mut record, actor)?;
        }
        Ok(())
    }
}
