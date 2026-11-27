use crate::{server::audit, store::*, Error, Result, Store};
use chrono::Utc;
use orkestr_protocol::*;
use rusqlite::{params, OptionalExtension, TransactionBehavior};
use uuid::Uuid;

impl Store {
    pub fn quotas(&self) -> Result<(u64, Quotas)> {
        let row: Option<(u64, String)> = self
            .db
            .query_row(
                "SELECT revision,data FROM runtime_settings WHERE id=1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        row.map(|(revision, data)| Ok((revision, serde_json::from_str(&data)?)))
            .unwrap_or_else(|| Ok((0, Quotas::default())))
    }
    pub fn save_quotas(
        &mut self,
        actor: Uuid,
        expected: u64,
        quotas: &Quotas,
    ) -> Result<(u64, Quotas)> {
        if !quotas.valid() {
            return Err(Error::Invalid("invalid quotas"));
        }
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let prior: Option<u64> = tx
            .query_row(
                "SELECT revision FROM runtime_settings WHERE id=1",
                [],
                |r| r.get(0),
            )
            .optional()?;
        if prior.unwrap_or(0) != expected {
            return Err(Error::Conflict);
        }
        tx.execute("INSERT INTO runtime_settings VALUES(1,?1,?2) ON CONFLICT(id) DO UPDATE SET revision=excluded.revision,data=excluded.data",params![expected+1,serde_json::to_string(quotas)?])?;
        audit(
            &tx,
            &actor.to_string(),
            "scheduler.quotas",
            "global",
            "QUOTAS_SAVED",
        )?;
        tx.commit()?;
        Ok((expected + 1, quotas.clone()))
    }
    pub fn acquire_resource(&mut self, id: Uuid, site: &str, phase: Phase) -> Result<bool> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let settings: Option<String> = tx
            .query_row("SELECT data FROM runtime_settings WHERE id=1", [], |r| {
                r.get(0)
            })
            .optional()?;
        let quotas: Quotas = settings
            .map(|s| serde_json::from_str(&s))
            .transpose()?
            .unwrap_or_default();
        let leases: Vec<Lease> = json_rows(&tx, "SELECT data FROM resource_leases")?;
        if leases.iter().any(|lease| lease.id == id) {
            return Ok(false);
        }
        if !orkestr_scheduler::can_claim(&quotas, &leases, site, phase) {
            return Ok(false);
        }
        let lease = Lease {
            id,
            site: orkestr_scheduler::site_key(site),
            phase,
            acquired_at: Utc::now(),
        };
        tx.execute(
            "INSERT INTO resource_leases VALUES(?1,?2,?3,?4)",
            params![
                id.to_string(),
                lease.site,
                format!("{:?}", phase),
                serde_json::to_string(&lease)?
            ],
        )?;
        tx.commit()?;
        Ok(true)
    }
    pub fn release_resource(&mut self, id: Uuid) -> Result<()> {
        self.db
            .execute("DELETE FROM resource_leases WHERE id=?1", [id.to_string()])?;
        Ok(())
    }
    pub fn resource_leases(&self) -> Result<Vec<Lease>> {
        json_rows(&self.db, "SELECT data FROM resource_leases ORDER BY rowid")
    }
    pub fn recover_resources(&mut self) -> Result<()> {
        self.db.execute("DELETE FROM resource_leases", [])?;
        Ok(())
    }
    pub fn policies(&self) -> Result<Vec<Policy>> {
        json_rows(
            &self.db,
            "SELECT data FROM policies ORDER BY rowid LIMIT 1024",
        )
    }
    pub fn save_policy(&mut self, actor: Uuid, input: &SavePolicy) -> Result<Policy> {
        if !input.policy.valid() {
            return Err(Error::Invalid("invalid policy"));
        }
        if self
            .library_application(input.policy.application_id)?
            .is_none()
            || !self
                .snapshot()?
                .collections
                .iter()
                .any(|c| c.id == input.policy.collection_id)
            || (self
                .managed_installer(input.policy.package_revision_id)?
                .is_none()
                && self
                    .workflow_definition(input.policy.package_revision_id)?
                    .is_none())
        {
            return Err(Error::Invalid("policy references unavailable"));
        }
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut policy = input.policy.clone();
        let prior: Option<u64> = tx
            .query_row(
                "SELECT revision FROM policies WHERE id=?1",
                [policy.id.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        if prior != input.expected {
            return Err(Error::Conflict);
        }
        if prior.is_none() {
            let count: u64 = tx.query_row("SELECT count(*) FROM policies", [], |r| r.get(0))?;
            if count >= 1024 {
                return Err(Error::Invalid("policy capacity"));
            }
        }
        policy.revision = prior.unwrap_or(0) + 1;
        let data = serde_json::to_string(&policy)?;
        tx.execute("INSERT INTO policies VALUES(?1,?2,?3) ON CONFLICT(id) DO UPDATE SET revision=excluded.revision,data=excluded.data",params![policy.id.to_string(),policy.revision,data])?;
        tx.execute(
            "INSERT INTO policy_history VALUES(?1,?2,?3,?4)",
            params![
                policy.id.to_string(),
                policy.revision,
                actor.to_string(),
                data
            ],
        )?;
        let next = orkestr_scheduler::retry_at(policy.id, 0, Utc::now(), policy.jitter_seconds)
            .to_rfc3339();
        tx.execute("INSERT INTO policy_clock VALUES(?1,?2,?2) ON CONFLICT(id) DO UPDATE SET next_check=excluded.next_check,next_deploy=excluded.next_deploy",params![policy.id.to_string(),next])?;
        audit(
            &tx,
            &actor.to_string(),
            "policy.save",
            &policy.id.to_string(),
            "POLICY_REVISION_SAVED",
        )?;
        tx.commit()?;
        Ok(policy)
    }
    pub fn policy_history(&self, id: Uuid) -> Result<Vec<Policy>> {
        let mut q = self
            .db
            .prepare("SELECT data FROM policy_history WHERE id=?1 ORDER BY revision")?;
        let rows = q.query_map([id.to_string()], |r| r.get::<_, String>(0))?;
        rows.map(|row| Ok(serde_json::from_str(&row?)?)).collect()
    }
    pub fn pending_server_work(&self) -> Result<Vec<Uuid>> {
        let cursor: Option<String> = self
            .db
            .query_row(
                "SELECT value FROM scheduler_state WHERE key='dispatch_cursor'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        let sql="SELECT id FROM (SELECT id FROM deployments WHERE json_extract(data,'$.state') IN ('Queued','ReconcileQueued') UNION ALL SELECT id FROM server_jobs WHERE json_extract(data,'$.state')='Queued' UNION ALL SELECT id FROM library_jobs WHERE state='Queued' UNION ALL SELECT id FROM workflow_runs WHERE json_extract(data,'$.state') IN ('Queued','ReconcileQueued')) WHERE id>?1 ORDER BY id LIMIT 256";
        let mut query = self.db.prepare(sql)?;
        let mut values = query
            .query_map([cursor.as_deref().unwrap_or("")], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        if values.is_empty() && cursor.is_some() {
            values = query
                .query_map([""], |r| r.get::<_, String>(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?;
        }
        values
            .into_iter()
            .map(|id| Uuid::parse_str(&id).map_err(|_| Error::Invalid("invalid work id")))
            .collect()
    }
    pub fn record_dispatch(&mut self, id: Uuid) -> Result<()> {
        self.db.execute("INSERT INTO scheduler_state VALUES('dispatch_cursor',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[id.to_string()])?;
        Ok(())
    }
}
