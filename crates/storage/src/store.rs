use crate::*;
use orkestr_domain::*;
use rusqlite::{params, Connection, OpenFlags, OptionalExtension, TransactionBehavior};
use serde::de::DeserializeOwned;
use std::{path::Path, time::Duration};

pub struct Store {
    pub(crate) db: Connection,
    pub(crate) discovery_sessions:
        std::collections::BTreeMap<uuid::Uuid, crate::discovery::Session>,
}

impl Store {
    /// Existing corrupt/newer/foreign databases fail closed, never replaced with an empty DB.
    pub fn open(path: &Path) -> Result<Self> {
        let mut db = Connection::open(path)?;
        db.busy_timeout(Duration::from_secs(5))?;
        db.execute_batch("PRAGMA foreign_keys=ON;")?;
        // Additive upgrade still gets a consistent, non-overwriting pre-upgrade backup.
        let prior: i64 = db.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        let prior_app: i64 = db.query_row("PRAGMA application_id", [], |r| r.get(0))?;
        if matches!(prior, 1..=9) && prior_app == 1330793285 {
            let next = match prior {
                5 => 6,
                6 => 7,
                7 => 8,
                8 => 9,
                9 => 10,
                _ => 5,
            };
            let backup =
                path.with_extension(format!("before-v{next}-{}.sqlite", uuid::Uuid::new_v4()));
            db.execute("VACUUM INTO ?1", [backup.to_string_lossy().as_ref()])?;
        }
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let version: i64 = tx.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        let app_id: i64 = tx.query_row("PRAGMA application_id", [], |r| r.get(0))?;
        match (version, app_id) {
            (0, 0) => {
                let count: i64 = tx.query_row(
                    "SELECT count(*) FROM sqlite_master WHERE name NOT LIKE 'sqlite_%'",
                    [],
                    |r| r.get(0),
                )?;
                if count != 0 {
                    return Err(Error::Schema);
                }
                tx.execute_batch(include_str!("migrations/001_core.sql"))?;
                tx.execute_batch("PRAGMA application_id=1330793285; PRAGMA user_version=1;")?;
            }
            (1..=10, 1330793285) => {}
            _ => return Err(Error::Schema),
        }
        if version < 2 {
            tx.execute_batch(include_str!("migrations/002_review.sql"))?;
            tx.execute_batch("PRAGMA user_version=2;")?;
        }
        if version < 3 {
            tx.execute_batch(include_str!("migrations/003_server.sql"))?;
            tx.execute_batch("PRAGMA user_version=3;")?;
        }
        if version < 4 {
            tx.execute_batch(include_str!("migrations/004_inventory.sql"))?;
            tx.execute_batch("PRAGMA user_version=4;")?;
        }
        if version < 5 {
            tx.execute_batch(include_str!("migrations/005_discovery.sql"))?;
            tx.execute_batch("PRAGMA user_version=5;")?;
        }
        // v5 readers ignore the new JSON field and would run Kerberos profiles with Negotiate.
        if version < 6 {
            tx.execute_batch("PRAGMA user_version=6;")?;
        }
        if version < 7 {
            tx.execute_batch(include_str!("migrations/007_deployment.sql"))?;
            tx.execute_batch("PRAGMA user_version=7;")?;
        }
        // Older binaries must not reinterpret a managed EXE as an MSI.
        if version < 8 {
            tx.execute_batch("PRAGMA user_version=8;")?;
        }
        if version < 9 {
            tx.execute_batch(include_str!("migrations/009_pipeline.sql"))?;
            tx.execute_batch("PRAGMA user_version=9;")?;
        }
        if version < 10 {
            tx.execute_batch("CREATE TABLE scheduler_state(key TEXT PRIMARY KEY,value TEXT NOT NULL);PRAGMA user_version=10;")?;
        }
        tx.commit()?;
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;")?;
        Ok(Self {
            db,
            discovery_sessions: Default::default(),
        })
    }

    /// CLI listings/dry-run can open Orkestr without creating or migrating anything.
    pub fn open_read_only(path: &Path) -> Result<Self> {
        let db = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        db.busy_timeout(Duration::from_secs(5))?;
        let version: i64 = db.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        let app: i64 = db.query_row("PRAGMA application_id", [], |r| r.get(0))?;
        if !matches!(version, 1..=10) || app != 1330793285 {
            return Err(Error::Schema);
        }
        Ok(Self {
            db,
            discovery_sessions: Default::default(),
        })
    }

    pub fn snapshot(&mut self) -> Result<InventorySnapshot> {
        let tx = self.db.transaction()?;
        let mut result = load_snapshot(&tx)?;
        tx.commit()?;
        let applications = self.library_applications()?;
        for record in &mut result.devices {
            if let Some(scan) = &mut record.scan {
                for installed in &mut scan.software {
                    let candidates: Vec<_> = applications
                        .iter()
                        .filter(|app| {
                            app.software_match
                                .name
                                .eq_ignore_ascii_case(&installed.name)
                                && app
                                    .software_match
                                    .publisher
                                    .eq_ignore_ascii_case(&installed.publisher)
                                && installed
                                    .architecture
                                    .as_ref()
                                    .is_some_and(|a| a.eq_ignore_ascii_case(&app.architecture))
                                && app.software_match.product_code.is_none_or(|expected| {
                                    scan.details
                                        .pointer("/OrkestrInventory/software")
                                        .and_then(|v| v.as_array())
                                        .is_some_and(|rows| {
                                            rows.iter().any(|row| {
                                                row["name"].as_str().is_some_and(|name| {
                                                    name.eq_ignore_ascii_case(&installed.name)
                                                }) && row["publisher"].as_str().is_some_and(
                                                    |publisher| {
                                                        publisher.eq_ignore_ascii_case(
                                                            &installed.publisher,
                                                        )
                                                    },
                                                ) && row["version_raw"].as_str()
                                                    == Some(installed.version.as_str())
                                                    && row["product_code"].as_str().and_then(|id| {
                                                        uuid::Uuid::parse_str(id).ok()
                                                    }) == Some(expected)
                                            })
                                        })
                                })
                        })
                        .collect();
                    if let [app] = candidates.as_slice() {
                        installed.application_id = Some(app.id);
                        installed.channel = Some(app.channel.clone());
                    }
                }
            }
        }
        Ok(result)
    }

    pub fn upsert_device(&mut self, device: &Device) -> Result<()> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        write_device(&tx, device)?;
        bump(&tx)?;
        tx.commit()?;
        Ok(())
    }

    pub fn append_scan(&mut self, scan: &DeviceScan) -> Result<()> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        write_scan(&tx, scan)?;
        bump(&tx)?;
        tx.commit()?;
        Ok(())
    }

    /// None means create only. Some(n) means update exactly revision n.
    pub fn save_collection(
        &mut self,
        collection: &Collection,
        expected: Option<u64>,
    ) -> Result<Collection> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let saved = write_collection(&tx, collection, expected)?;
        bump(&tx)?;
        tx.commit()?;
        Ok(saved)
    }

    pub fn delete_collection(&mut self, id: CollectionId, expected: u64) -> Result<()> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let children: i64 = tx.query_row(
            "SELECT count(*) FROM collections WHERE parent_id=?1",
            [id.to_string()],
            |r| r.get(0),
        )?;
        if children > 0 {
            return Err(Error::Invalid("move children before deleting folder"));
        }
        if tx.execute(
            "DELETE FROM collections WHERE id=?1 AND revision=?2",
            params![id.to_string(), expected],
        )? != 1
        {
            return Err(Error::Conflict);
        }
        bump(&tx)?;
        tx.commit()?;
        Ok(())
    }

    pub fn add_package_revision(&mut self, package: &PackageRevision) -> Result<()> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        write_package(&tx, package)?;
        bump(&tx)?;
        tx.commit()?;
        Ok(())
    }

    pub fn packages(&self) -> Result<Vec<PackageRevision>> {
        json_rows(
            &self.db,
            "SELECT data FROM package_revisions ORDER BY rowid",
        )
    }

    /// Adapted from Open Inventory's MIT backup pattern. Refuses overwrite; consistent with WAL.
    pub fn backup(&self, destination: &Path) -> Result<()> {
        if destination.exists() {
            return Err(Error::Invalid("backup already exists"));
        }
        self.db
            .execute("VACUUM INTO ?1", [destination.to_string_lossy().as_ref()])?;
        Ok(())
    }
}

pub(crate) fn load_snapshot(db: &Connection) -> Result<InventorySnapshot> {
    let generation = db.query_row("SELECT generation FROM metadata WHERE id=1", [], |r| {
        r.get(0)
    })?;
    let mut q = db.prepare("SELECT d.data,s.data FROM devices d LEFT JOIN scan_heads h ON h.device_id=d.id LEFT JOIN scans s ON s.id=h.scan_id ORDER BY d.host")?;
    let rows = q.query_map([], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
    })?;
    let mut devices = vec![];
    for row in rows {
        let (device, scan) = row?;
        devices.push(DeviceRecord {
            device: serde_json::from_str(&device)?,
            scan: scan.map(|s| serde_json::from_str(&s)).transpose()?,
        });
    }
    Ok(InventorySnapshot {
        generation,
        devices,
        collections: json_rows(db, "SELECT data FROM collections ORDER BY rowid")?,
    })
}
pub(crate) fn json_rows<T: DeserializeOwned>(db: &Connection, sql: &str) -> Result<Vec<T>> {
    let mut query = db.prepare(sql)?;
    let rows = query.query_map([], |r| r.get::<_, String>(0))?;
    rows.map(|r| Ok(serde_json::from_str(&r?)?)).collect()
}
pub(crate) fn bump(db: &Connection) -> Result<()> {
    db.execute("UPDATE metadata SET generation=generation+1 WHERE id=1", [])?;
    Ok(())
}
pub(crate) fn write_device(db: &Connection, device: &Device) -> Result<()> {
    let host = canonical_host(&device.host)?;
    let mut normalized = device.clone();
    normalized.host = host.clone();
    normalized.aliases = device
        .aliases
        .iter()
        .map(|a| canonical_host(a))
        .collect::<std::result::Result<Vec<_>, _>>()?;
    normalized.aliases.sort();
    normalized.aliases.dedup();
    // An identity's old names stay reserved so a rename cannot silently reassign static members.
    let old: Option<String> = db
        .query_row(
            "SELECT data FROM devices WHERE id=?1",
            [device.id.to_string()],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(old) = old {
        let old: Device = serde_json::from_str(&old)?;
        if old.ad_guid.is_some() && old.ad_guid != device.ad_guid {
            return Err(Error::Invalid("AD identity cannot be reassigned"));
        }
        normalized.aliases.extend(old.aliases);
        if old.host != host {
            normalized.aliases.push(old.host);
        }
    }
    normalized.aliases.retain(|a| a != &host);
    normalized.aliases.sort();
    normalized.aliases.dedup();
    for name in std::iter::once(&host).chain(normalized.aliases.iter()) {
        let owner: Option<String> = db
            .query_row(
                "SELECT device_id FROM device_aliases WHERE host=?1",
                [name],
                |r| r.get(0),
            )
            .optional()?;
        if owner.is_some_and(|id| id != device.id.to_string()) {
            return Err(Error::Invalid("hostname/alias belongs to another identity"));
        }
    }
    db.execute("INSERT INTO devices(id,host,ad_guid,data) VALUES(?1,?2,?3,?4) ON CONFLICT(id) DO UPDATE SET host=excluded.host,ad_guid=excluded.ad_guid,data=excluded.data",
        params![device.id.to_string(),host,device.ad_guid.map(|g|g.to_string()),serde_json::to_string(&normalized)?])?;
    for name in std::iter::once(&normalized.host).chain(normalized.aliases.iter()) {
        db.execute(
            "INSERT INTO device_aliases VALUES(?1,?2) ON CONFLICT(host) DO NOTHING",
            params![name, device.id.to_string()],
        )?;
    }
    Ok(())
}
pub(crate) fn write_scan(db: &Connection, scan: &DeviceScan) -> Result<()> {
    if scan.installed_ram_bytes == Some(0) {
        return Err(Error::Invalid("installed RAM must be positive or unknown"));
    }
    db.execute(
        "INSERT INTO scans VALUES(?1,?2,?3,?4)",
        params![
            scan.id.to_string(),
            scan.device_id.to_string(),
            scan.scanned_at.to_rfc3339(),
            serde_json::to_string(scan)?
        ],
    )?;
    for (i, app) in scan.software.iter().enumerate() {
        db.execute(
            "INSERT INTO installed_software VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
            params![
                scan.id.to_string(),
                i as i64,
                app.application_id.map(|a| a.to_string()),
                app.name,
                app.publisher,
                app.version,
                app.channel,
                app.architecture
            ],
        )?;
    }
    let old: Option<String>=db.query_row("SELECT s.scanned_at FROM scan_heads h JOIN scans s ON s.id=h.scan_id WHERE h.device_id=?1",[scan.device_id.to_string()],|r|r.get(0)).optional()?;
    let is_newer = match old {
        Some(old) => {
            chrono::DateTime::parse_from_rfc3339(&old)
                .map_err(|_| Error::Invalid("invalid stored timestamp"))?
                < scan.scanned_at
        }
        None => true,
    };
    if is_newer {
        db.execute("INSERT INTO scan_heads VALUES(?1,?2) ON CONFLICT(device_id) DO UPDATE SET scan_id=excluded.scan_id",params![scan.device_id.to_string(),scan.id.to_string()])?;
    }
    Ok(())
}
pub(crate) fn write_collection(
    db: &Connection,
    c: &Collection,
    expected: Option<u64>,
) -> Result<Collection> {
    let old: Option<u64> = db
        .query_row(
            "SELECT revision FROM collections WHERE id=?1",
            [c.id.to_string()],
            |r| r.get(0),
        )
        .optional()?;
    if old != expected {
        return Err(Error::Conflict);
    }
    load_snapshot(db)?.validate_collection(c)?;
    let mut saved = c.clone();
    saved.revision = old.unwrap_or(0) + 1;
    db.execute("INSERT INTO collections VALUES(?1,?2,?3,?4) ON CONFLICT(id) DO UPDATE SET parent_id=excluded.parent_id,revision=excluded.revision,data=excluded.data",params![saved.id.to_string(),saved.parent_id.map(|p|p.to_string()),saved.revision,serde_json::to_string(&saved)?])?;
    Ok(saved)
}
pub(crate) fn write_package(db: &Connection, package: &PackageRevision) -> Result<()> {
    if package.name.trim().is_empty() {
        return Err(Error::Invalid("package name required"));
    }
    if package
        .artifact_hashes
        .iter()
        .any(|hash| hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return Err(Error::Invalid("invalid SHA-256"));
    }
    db.execute(
        "INSERT INTO package_revisions VALUES(?1,?2,?3)",
        params![
            package.id.to_string(),
            package.package_id.to_string(),
            serde_json::to_string(package)?
        ],
    )?;
    Ok(())
}
