use crate::{store::*, Error, Result, Store};
use orkestr_domain::{Device, DeviceId};
use orkestr_inventory::*;
use orkestr_protocol::{DeviceInventoryStatus, InventoryRunStatus, ProviderStatus};
use rusqlite::{params, OptionalExtension, TransactionBehavior};
use uuid::Uuid;

impl Store {
    pub fn device_allows_domain_account(&self, host: &str, domain: &str) -> Result<bool> {
        let data: Option<String> = self
            .db
            .query_row("SELECT data FROM devices WHERE host=?1", [host], |row| {
                row.get(0)
            })
            .optional()?;
        Ok(data
            .map(|data| serde_json::from_str::<Device>(&data))
            .transpose()?
            .is_some_and(|d| {
                !d.disabled && (d.domain.is_empty() || d.domain.eq_ignore_ascii_case(domain))
            }))
    }
    pub fn device(&self, id: DeviceId) -> Result<Option<Device>> {
        let data: Option<String> = self
            .db
            .query_row(
                "SELECT data FROM devices WHERE id=?1",
                [id.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        data.map(|data| serde_json::from_str(&data).map_err(Error::from))
            .transpose()
    }
    /// Safe, on-demand metadata: no raw rows, software identities/SIDs or exception text.
    pub fn device_inventory_status(&mut self, device: DeviceId) -> Result<DeviceInventoryStatus> {
        let tx = self.db.transaction()?;
        let generation = tx.query_row("SELECT generation FROM metadata WHERE id=1", [], |r| {
            r.get(0)
        })?;
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM devices WHERE id=?1)",
            [device.to_string()],
            |r| r.get(0),
        )?;
        if !exists {
            return Err(Error::Invalid("device not found"));
        }
        let latest: Option<String> = tx.query_row(
            "SELECT data FROM inventory_reports WHERE device_id=?1 ORDER BY started_seconds DESC, started_nanos DESC, finished_seconds DESC, finished_nanos DESC, id DESC LIMIT 1",
            [device.to_string()],|r|r.get(0),
        ).optional()?;
        let head: Option<String> = tx.query_row(
            "SELECT r.data FROM inventory_reports r JOIN scan_heads h ON h.scan_id=r.scan_id WHERE h.device_id=?1",
            [device.to_string()],|r|r.get(0),
        ).optional()?;
        let result = DeviceInventoryStatus {
            device_id: device,
            generation,
            latest: latest.map(|s| status(&s)).transpose()?,
            head: head.map(|s| status(&s)).transpose()?,
        };
        tx.commit()?;
        Ok(result)
    }
    pub fn preview_discovery(&mut self, batch: &DiscoveryBatch) -> Result<DiscoveryPreview> {
        Ok(batch.preview(&self.snapshot()?))
    }

    /// All rows or none, with snapshot conflict detection; never creates a scan/job.
    /// The caller must create a rollback backup before an explicitly approved apply.
    pub fn apply_discovery(&mut self, plan: &DiscoveryPreview) -> Result<usize> {
        if !plan.can_apply() {
            return Err(Error::Invalid("discovery preview has conflicts"));
        }
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if !plan.is_current(&load_snapshot(&tx)?) {
            return Err(Error::Conflict);
        }
        for device in plan.writes() {
            write_device(&tx, device)?;
        }
        if !plan.writes().is_empty() {
            bump(&tx)?;
        }
        tx.commit()?;
        Ok(plan.writes().len())
    }

    /// Persists the normalized report and optional immutable scan in one transaction.
    /// Failed/Unknown reports DO NOT replace the previous scan head or refresh its age.
    pub fn record_inventory(&mut self, inventory: &NormalizedInventory) -> Result<()> {
        self.commit_inventory(inventory, None)
    }

    pub fn complete_inventory_job(
        &mut self,
        inventory: &NormalizedInventory,
        job: &orkestr_protocol::Job,
    ) -> Result<()> {
        self.commit_inventory(inventory, Some(job))
    }

    fn commit_inventory(
        &mut self,
        inventory: &NormalizedInventory,
        job: Option<&orkestr_protocol::Job>,
    ) -> Result<()> {
        let report = inventory.report();
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(scan) = inventory.scan() {
            write_scan(&tx, scan)?;
        }
        tx.execute(
            "INSERT INTO inventory_reports VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
            params![
                report.id.to_string(),
                report.device_id.to_string(),
                report.started_at.timestamp(),
                report.started_at.timestamp_subsec_nanos(),
                report.finished_at.timestamp(),
                report.finished_at.timestamp_subsec_nanos(),
                inventory.scan().map(|s| s.id.to_string()),
                serde_json::to_string(report)?,
            ],
        )?;
        bump(&tx)?;
        if let Some(job) = job {
            crate::server::write_job_state(&tx, job)?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Private evidence, not a public export. No remote endpoint is added in this increment.
    pub fn inventory_report(&self, id: Uuid) -> Result<Option<InventoryReport>> {
        let data: Option<String> = self
            .db
            .query_row(
                "SELECT data FROM inventory_reports WHERE id=?1",
                [id.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        data.map(|s| serde_json::from_str(&s).map_err(Error::from))
            .transpose()
    }

    pub fn latest_inventory_report(&self, device: DeviceId) -> Result<Option<InventoryReport>> {
        let data: Option<String> = self.db.query_row(
            "SELECT data FROM inventory_reports WHERE device_id=?1 ORDER BY started_seconds DESC, started_nanos DESC, finished_seconds DESC, finished_nanos DESC, id DESC LIMIT 1",
            [device.to_string()], |r| r.get(0),
        ).optional()?;
        data.map(|s| serde_json::from_str(&s).map_err(Error::from))
            .transpose()
    }
}

fn status(data: &str) -> Result<InventoryRunStatus> {
    let report: InventoryReport = serde_json::from_str(data)?;
    Ok(InventoryRunStatus {
        id: report.id,
        started_at: report.started_at,
        finished_at: report.finished_at,
        state: report.state(),
        providers: report
            .providers
            .iter()
            .map(|p| ProviderStatus {
                provider: p.provider,
                state: p.state,
                failure: p.failure,
                rows: p.rows.len(),
            })
            .collect(),
    })
}
