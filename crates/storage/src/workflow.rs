use crate::{server::audit, store::*, Error, Result, Store};
use chrono::Utc;
use orkestr_domain::{PackageId, PackageRevision, PackageRevisionId, PackageStep};
use orkestr_protocol::*;
use rusqlite::{params, OptionalExtension, TransactionBehavior};
use uuid::Uuid;
impl Store {
    pub fn workflow_definition(&self, id: PackageRevisionId) -> Result<Option<WorkflowDefinition>> {
        let data: Option<String> = self
            .db
            .query_row(
                "SELECT data FROM workflow_revisions WHERE id=?1",
                [id.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        data.map(|s| serde_json::from_str(&s).map_err(Error::from))
            .transpose()
    }
    pub fn register_workflow(
        &mut self,
        actor: Uuid,
        input: &WorkflowDefinition,
    ) -> Result<PackageRevision> {
        if !input.valid() {
            return Err(Error::Invalid("invalid workflow"));
        }
        fn expand(
            store: &Store,
            steps: &[WorkflowStep],
            stack: &mut Vec<PackageRevisionId>,
            output: &mut Vec<WorkflowStep>,
        ) -> Result<()> {
            if stack.len() > 16 {
                return Err(Error::Invalid("nested package depth"));
            }
            for step in steps {
                if let WorkflowStep::NestedPackage { revision_id } = step {
                    if stack.contains(revision_id) {
                        return Err(Error::Invalid("nested package cycle"));
                    }
                    let definition = store
                        .workflow_definition(*revision_id)?
                        .ok_or(Error::Invalid("nested revision unavailable"))?;
                    stack.push(*revision_id);
                    expand(store, &definition.steps, stack, output)?;
                    stack.pop();
                } else {
                    output.push(step.clone());
                }
                if output.len() > 64 {
                    return Err(Error::Invalid("too many workflow steps"));
                }
            }
            Ok(())
        }
        let mut definition = input.clone();
        definition.steps.clear();
        expand(self, &input.steps, &mut vec![], &mut definition.steps)?;
        let mut artifacts = vec![];
        for step in &definition.steps {
            match step {
                WorkflowStep::Install { installer } => artifacts.push(installer.sha256.clone()),
                WorkflowStep::FileCopy { sha256, .. } => artifacts.push(sha256.clone()),
                _ => {}
            }
        }
        artifacts.sort();
        artifacts.dedup();
        let package = PackageRevision {
            id: PackageRevisionId::new(),
            package_id: PackageId::new(),
            name: definition.name.clone(),
            version: definition.version.clone(),
            steps: definition
                .steps
                .iter()
                .map(|step| {
                    Ok(PackageStep {
                        kind: step.step_kind(),
                        definition: serde_json::to_value(step)?,
                    })
                })
                .collect::<Result<_>>()?,
            artifact_hashes: artifacts,
            metadata: serde_json::json!({"managed_workflow":true}),
        };
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        write_package(&tx, &package)?;
        tx.execute(
            "INSERT INTO workflow_revisions VALUES(?1,?2)",
            params![package.id.to_string(), serde_json::to_string(&definition)?],
        )?;
        audit(
            &tx,
            &actor.to_string(),
            "workflow.package",
            &package.id.to_string(),
            "WORKFLOW_REVISION_REGISTERED",
        )?;
        bump(&tx)?;
        tx.commit()?;
        Ok(package)
    }
    pub fn workflow_runs(&self) -> Result<Vec<WorkflowRun>> {
        json_rows(
            &self.db,
            "SELECT data FROM workflow_runs ORDER BY holds_device DESC,rowid DESC LIMIT 128",
        )
    }
    pub fn workflow_run(&self, id: Uuid) -> Result<Option<WorkflowRun>> {
        let data: Option<String> = self
            .db
            .query_row(
                "SELECT data FROM workflow_runs WHERE id=?1",
                [id.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        data.map(|s| serde_json::from_str(&s).map_err(Error::from))
            .transpose()
    }
    pub fn plan_workflow(
        &mut self,
        actor: Uuid,
        key: &str,
        plan: PlanDeployment,
        connection: &ConnectionProfile,
        credential_revision: u64,
    ) -> Result<WorkflowRun> {
        let prior: Option<String> = self
            .db
            .query_row(
                "SELECT data FROM workflow_runs WHERE actor_id=?1 AND idempotency=?2",
                params![actor.to_string(), key],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(data) = prior {
            let run: WorkflowRun = serde_json::from_str(&data)?;
            if run.plan != plan {
                return Err(Error::Conflict);
            }
            return Ok(run);
        }
        let definition = self
            .workflow_definition(plan.package_revision_id)?
            .ok_or(Error::Invalid("managed workflow required"))?;
        let run = WorkflowRun {
            id: Uuid::new_v4(),
            revision: 1,
            plan,
            definition,
            host: connection.host.clone(),
            created_by: actor,
            action_by: None,
            credential_revision,
            state: DeploymentState::WaitingApproval,
            code: "WAITING_APPROVAL".into(),
            created_at: Utc::now(),
            next_step: 0,
            checkpoints: vec![],
            reboot_required: false,
            cancel_requested: false,
        };
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT INTO workflow_runs VALUES(?1,?2,?3,?4,0,1,?5)",
            params![
                run.id.to_string(),
                actor.to_string(),
                key,
                run.plan.device_id.to_string(),
                serde_json::to_string(&run)?
            ],
        )?;
        audit(
            &tx,
            &actor.to_string(),
            "workflow.plan",
            &run.id.to_string(),
            "WAITING_APPROVAL",
        )?;
        tx.commit()?;
        Ok(run)
    }
    pub fn update_workflow(&mut self, actor: Uuid, run: &mut WorkflowRun) -> Result<()> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if run.state.holds_device() {
            let busy:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM deployments WHERE device_id=?1 AND holds_device=1 UNION ALL SELECT 1 FROM workflow_runs WHERE device_id=?1 AND holds_device=1 AND id<>?2)",params![run.plan.device_id.to_string(),run.id.to_string()],|r|r.get(0))?;
            if busy {
                return Err(Error::DeploymentDeviceBusy);
            }
        }
        let mut updated = run.clone();
        updated.revision += 1;
        if tx.execute("UPDATE workflow_runs SET revision=?2,holds_device=?3,data=?4 WHERE id=?1 AND revision=?5",params![run.id.to_string(),updated.revision,run.state.holds_device(),serde_json::to_string(&updated)?,run.revision])?!=1{return Err(Error::Conflict);}
        audit(
            &tx,
            &actor.to_string(),
            "workflow.state",
            &run.id.to_string(),
            &run.code,
        )?;
        tx.commit()?;
        run.revision = updated.revision;
        Ok(())
    }
    pub fn recover_workflows(&mut self) -> Result<()> {
        let runs: Vec<WorkflowRun> = json_rows(
            &self.db,
            "SELECT data FROM workflow_runs WHERE holds_device=1",
        )?;
        for mut run in runs {
            match run.state {
                DeploymentState::Running
                | DeploymentState::Reconciling
                | DeploymentState::ReconcileQueued => {
                    run.state = DeploymentState::NeedsReconciliation;
                    run.code = "RESTARTED_NEEDS_RECONCILIATION".into();
                }
                DeploymentState::Queued => {
                    run.state = DeploymentState::Failed;
                    run.code = "RESTARTED_BEFORE_STEP".into();
                }
                _ => continue,
            }
            self.update_workflow(run.created_by, &mut run)?;
        }
        Ok(())
    }
}
