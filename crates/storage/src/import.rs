//! Import only explicitly selected, closed backup copies. Never opens old app paths itself.
use crate::{store::*, *};
use chrono::Utc;
use orkestr_domain::*;
use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
    path::{Path, PathBuf},
};
use uuid::Uuid;

const MAX_SOURCE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_ENTITIES: usize = 100_000;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImportReport {
    pub source_sha256: String,
    pub format: String,
    pub base_generation: u64,
    pub devices: usize,
    pub scans: usize,
    pub collections: usize,
    pub packages: usize,
    pub updated_devices: usize,
    pub excluded_schedules: usize,
    pub excluded_history: usize,
    #[serde(default)]
    pub review_items: usize,
    /// Stable diagnostic codes only: never source text or legacy output.
    pub warnings: BTreeSet<String>,
    pub errors: Vec<String>,
    pub rollback: String,
}
impl ImportReport {
    fn new(hash: String, format: &str, generation: u64) -> Self {
        Self { source_sha256:hash,format:format.into(),base_generation:generation,devices:0,scans:0,collections:0,packages:0,updated_devices:0,
            excluded_schedules:0,excluded_history:0,review_items:0,warnings:BTreeSet::new(),errors:vec![],
            rollback:"No source writes. Apply is atomic. Before apply, create a NEW Orkestr backup; restore it with all Orkestr handles closed. No remote actions occur.".into() }
    }
    pub fn can_apply(&self) -> bool {
        self.errors.is_empty()
    }
}

/// Contents are private: callers cannot alter a reviewed revision/identity map.
pub struct ImportPlan {
    source: PathBuf,
    report: ImportReport,
    devices: Vec<Device>,
    scans: Vec<DeviceScan>,
    collections: Vec<Collection>,
    packages: Vec<PackageRevision>,
    reviews: Vec<ImportReview>,
    origin_key: String,
    map: Vec<(&'static str, String, String)>,
}
impl ImportPlan {
    pub fn report(&self) -> &ImportReport {
        &self.report
    }
    fn add_review(&mut self, kind: &str, name: String, data: Value) {
        self.reviews.push(ImportReview {
            id: Uuid::new_v4().to_string(),
            source_sha256: self.report.source_sha256.clone(),
            kind: kind.into(),
            name,
            enabled: false,
            data,
        });
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportOutcome {
    Applied,
    AlreadyImported,
}

fn source_bytes(path: &Path) -> Result<Vec<u8>> {
    // No URL/UNC source resolution and no execution. A local copy is required.
    let path = std::fs::canonicalize(path)?;
    let text = path.to_string_lossy();
    if text.starts_with("\\\\") && !text.starts_with("\\\\?\\") || text.starts_with("\\\\?\\UNC\\")
    {
        return Err(Error::Invalid(
            "select a local backup copy, not a network path",
        ));
    }
    let f = std::fs::File::open(path)?;
    if !f.metadata()?.is_file() || f.metadata()?.len() > MAX_SOURCE_BYTES {
        return Err(Error::Invalid("source exceeds 64 MiB or is not a file"));
    }
    let mut bytes = vec![];
    f.take(MAX_SOURCE_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_SOURCE_BYTES {
        return Err(Error::Invalid("source exceeds 64 MiB"));
    }
    Ok(bytes)
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn empty_plan(path: &Path, bytes: &[u8], format: &str, generation: u64) -> Result<ImportPlan> {
    Ok(ImportPlan {
        source: std::fs::canonicalize(path)?,
        report: ImportReport::new(hash(bytes), format, generation),
        devices: vec![],
        scans: vec![],
        collections: vec![],
        packages: vec![],
        reviews: vec![],
        origin_key: hash(std::fs::canonicalize(path)?.to_string_lossy().as_bytes()),
        map: vec![],
    })
}

impl Store {
    pub fn preview_deploy_copy(&mut self, path: &Path) -> Result<ImportPlan> {
        self.preview_deploy_copy_with_origin(path, None)
    }

    /// Explicit origin joins revisions across differently named backups of ONE old server.
    pub fn preview_deploy_copy_with_origin(
        &mut self,
        path: &Path,
        origin: Option<&str>,
    ) -> Result<ImportPlan> {
        let bytes = source_bytes(path)?;
        let snapshot = self.snapshot()?;
        let mut plan = empty_plan(
            path,
            &bytes,
            "OpenDeploy.console.json/v1",
            snapshot.generation,
        )?;
        if let Some(origin) = origin {
            if origin.trim().is_empty() || origin.len() > 128 {
                return Err(Error::Invalid("origin must contain 1..128 bytes"));
            }
            plan.origin_key = hash(format!("OpenDeploy:{}", origin.trim()).as_bytes());
        }
        let root: Value = serde_json::from_slice(&bytes)?;
        let packages = root
            .get("packages")
            .and_then(Value::as_array)
            .ok_or(Error::Invalid("missing packages array"))?;
        if packages.len() > MAX_ENTITIES {
            return Err(Error::Invalid("too many packages"));
        }
        let mut seen = BTreeSet::new();
        for (index, p) in packages.iter().enumerate() {
            let legacy = p.get("id").and_then(Value::as_u64);
            let name = p
                .get("name")
                .and_then(Value::as_str)
                .filter(|n| !n.trim().is_empty());
            let steps = p.get("steps").and_then(Value::as_array);
            let (Some(legacy), Some(name), Some(steps)) = (legacy, name, steps) else {
                plan.report.errors.push(format!("PACKAGE_INVALID:{index}"));
                continue;
            };
            if !seen.insert(legacy) {
                plan.report
                    .errors
                    .push(format!("PACKAGE_DUPLICATE_ID:{index}"));
                continue;
            }
            let mut converted = vec![];
            for (step_index, step) in steps.iter().enumerate() {
                let kind = step
                    .get("kind")
                    .cloned()
                    .and_then(|k| serde_json::from_value(k).ok());
                match kind {
                    Some(kind) => converted.push(PackageStep {
                        kind,
                        definition: step.clone(),
                    }),
                    None => plan
                        .report
                        .errors
                        .push(format!("STEP_UNSUPPORTED:{index}:{step_index}")),
                }
            }
            let schema: i64 = self.db.query_row("PRAGMA user_version", [], |r| r.get(0))?;
            let old_package: Option<String> = if schema >= 2 {
                self.db.query_row("SELECT package_id FROM package_lineage WHERE origin_key=?1 AND legacy_key=?2",
                rusqlite::params![plan.origin_key,legacy.to_string()],|r|r.get(0)).optional()?
            } else {
                None
            };
            let package_id = match old_package {
                Some(id) => PackageId(
                    Uuid::parse_str(&id)
                        .map_err(|_| Error::Invalid("invalid stored package identity"))?,
                ),
                None => PackageId::new(),
            };
            let revision = PackageRevision {
                id: PackageRevisionId::new(),
                package_id,
                name: name.into(),
                version: p
                    .get("version")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .into(),
                steps: converted,
                artifact_hashes: vec![],
                metadata: serde_json::json!({"description":p.get("description").and_then(Value::as_str),"folder":p.get("folder").and_then(Value::as_str),"modified":p.get("modified").and_then(Value::as_str)}),
            };
            plan.map.push((
                "package",
                legacy.to_string(),
                revision.package_id.to_string(),
            ));
            plan.packages.push(revision);
        }
        plan.report.packages = plan.packages.len();
        plan.report.excluded_schedules = root
            .get("schedules")
            .and_then(Value::as_array)
            .map_or(0, Vec::len);
        plan.report.excluded_history = root
            .get("deployments")
            .and_then(Value::as_array)
            .map_or(0, Vec::len);
        plan.report.warnings.extend([
            "NO_APPROVAL_ARTIFACTS_OR_EXECUTION".into(),
            "SETTINGS_SECRETS_AND_HISTORY_OUTPUTS_NOT_IMPORTED".into(),
        ]);
        for (field, kind, allowed) in [
            ("lists", "target_list", &["name", "computers"][..]),
            (
                "schedules",
                "schedule",
                &[
                    "id",
                    "name",
                    "package_id",
                    "targets",
                    "next",
                    "recurrence",
                    "enabled",
                    "last",
                ][..],
            ),
            (
                "deployments",
                "history",
                &[
                    "id",
                    "package_id",
                    "package_name",
                    "created",
                    "finished",
                    "retries",
                ][..],
            ),
        ] {
            if let Some(items) = root.get(field).and_then(Value::as_array) {
                if items.len() > MAX_ENTITIES {
                    return Err(Error::Invalid("too many review records"));
                }
                for item in items {
                    let mut data = serde_json::Map::new();
                    for key in allowed {
                        if let Some(value) =
                            item.get(*key).filter(|v| !v.is_array() && !v.is_object())
                        {
                            data.insert((*key).into(), value.clone());
                        }
                    }
                    if kind == "history" {
                        let targets=item.get("targets").and_then(Value::as_array).map(|rows| rows.iter().map(|r| {
                            serde_json::json!({"computer":r.get("computer"),"status":r.get("status"),"step":r.get("step"),"total":r.get("total"),"elapsed":r.get("elapsed")})
                        }).collect::<Vec<_>>()).unwrap_or_default();
                        data.insert("targets".into(), Value::Array(targets));
                    }
                    data.insert("enabled".into(), Value::Bool(false));
                    if kind == "schedule" {
                        data.insert(
                            "was_enabled".into(),
                            Value::Bool(
                                item.get("enabled")
                                    .and_then(Value::as_bool)
                                    .unwrap_or(false),
                            ),
                        );
                    }
                    data.insert("review_only".into(), Value::Bool(true));
                    if kind == "target_list" || kind == "schedule" {
                        let raw = item
                            .get(if kind == "target_list" {
                                "computers"
                            } else {
                                "targets"
                            })
                            .and_then(Value::as_str)
                            .unwrap_or("");
                        let mut resolved = BTreeSet::new();
                        let mut unresolved = BTreeSet::new();
                        for (index, target) in raw
                            .split(|c: char| c.is_whitespace() || c == ',' || c == ';')
                            .filter(|s| !s.is_empty())
                            .enumerate()
                        {
                            match canonical_host(target) {
                                Ok(host) => {
                                    if let Some(row) = snapshot.devices.iter().find(|r| {
                                        r.device.host == host || r.device.aliases.contains(&host)
                                    }) {
                                        resolved.insert(row.device.id.to_string());
                                    } else {
                                        unresolved.insert(host);
                                    }
                                }
                                Err(_) => {
                                    unresolved
                                        .insert(format!("invalid target token #{}", index + 1));
                                }
                            }
                        }
                        if !unresolved.is_empty() {
                            plan.report
                                .warnings
                                .insert("UNRESOLVED_LEGACY_TARGETS_REVIEW_ONLY".into());
                        }
                        data.insert("resolved_device_ids".into(), serde_json::json!(resolved));
                        data.insert("unresolved_targets".into(), serde_json::json!(unresolved));
                    }
                    let name = item
                        .get("name")
                        .or_else(|| item.get("package_name"))
                        .and_then(Value::as_str)
                        .unwrap_or("Legacy review")
                        .to_owned();
                    plan.add_review(kind, name, Value::Object(data));
                }
            }
        }
        plan.report.review_items = plan.reviews.len();
        if !plan.reviews.is_empty() {
            plan.report
                .warnings
                .insert("REVIEW_ONLY_AUTOMATION_DISABLED".into());
        }
        Ok(plan)
    }

    pub fn preview_inventory_copy(&mut self, path: &Path) -> Result<ImportPlan> {
        let bytes = source_bytes(path)?;
        // Require a VACUUM INTO/SQLite backup, not a live WAL file copied without sidecars.
        if bytes.get(..16) != Some(b"SQLite format 3\0")
            || bytes.get(18) != Some(&1)
            || bytes.get(19) != Some(&1)
        {
            return Err(Error::Invalid(
                "select a closed SQLite backup (VACUUM INTO), not a live WAL database",
            ));
        }
        for suffix in ["-wal", "-journal"] {
            let mut sidecar = path.as_os_str().to_os_string();
            sidecar.push(suffix);
            if Path::new(&sidecar).exists() {
                return Err(Error::Invalid(
                    "source has a WAL/journal; create a consistent backup first",
                ));
            }
        }
        let source = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        // Bound SQLite allocations before any schema/query evaluation on an untrusted copy.
        source.set_limit(
            rusqlite::limits::Limit::SQLITE_LIMIT_LENGTH,
            MAX_SOURCE_BYTES as i32,
        );
        source.set_limit(rusqlite::limits::Limit::SQLITE_LIMIT_SQL_LENGTH, 16 * 1024);
        source.set_limit(rusqlite::limits::Limit::SQLITE_LIMIT_COLUMN, 128);
        source.execute_batch("PRAGMA query_only=ON; PRAGMA trusted_schema=OFF; BEGIN;")?;
        for table in ["computers", "collections"] {
            let kind: Option<String> = source
                .query_row(
                    "SELECT type FROM sqlite_master WHERE name=?1",
                    [table],
                    |r| r.get(0),
                )
                .optional()?;
            if kind.as_deref() != Some("table") {
                return Err(Error::Invalid("unsupported Inventory schema"));
            }
        }
        let rows: Vec<String> =
            bounded_source_rows(&source, "SELECT data FROM computers LIMIT 100001")?;
        let collections: Vec<String> =
            bounded_source_rows(&source, "SELECT data FROM collections LIMIT 100001")?;
        let snapshot = self.snapshot()?;
        let mut plan = empty_plan(path, &bytes, "OpenInventory.sqlite/v1", snapshot.generation)?;
        let mut known: Vec<Device> = snapshot.devices.iter().map(|r| r.device.clone()).collect();
        let (mut seen_hosts, mut seen_guids) = (BTreeSet::new(), BTreeSet::new());
        for (index, row) in rows.iter().enumerate() {
            let old: LegacyComputer = serde_json::from_str(row)?;
            let host = match canonical_host(&old.host) {
                Ok(h) => h,
                Err(_) => {
                    plan.report.errors.push(format!("HOST_INVALID:{index}"));
                    continue;
                }
            };
            let guid = if old.guid.trim().is_empty() {
                None
            } else {
                match Uuid::parse_str(&old.guid) {
                    Ok(g) => Some(g),
                    Err(_) => {
                        plan.report.errors.push(format!("GUID_INVALID:{index}"));
                        continue;
                    }
                }
            };
            if !seen_hosts.insert(host.clone()) || guid.is_some_and(|g| !seen_guids.insert(g)) {
                plan.report
                    .errors
                    .push(format!("DUPLICATE_IDENTITY:{index}"));
                continue;
            }
            let by_guid = guid.and_then(|g| known.iter().find(|d| d.ad_guid == Some(g)));
            let by_host = known
                .iter()
                .find(|d| d.host == host || d.aliases.contains(&host));
            if by_guid.zip(by_host).is_some_and(|(a, b)| a.id != b.id)
                || by_host.is_some_and(|d| d.ad_guid.zip(guid).is_some_and(|(a, b)| a != b))
            {
                plan.report
                    .errors
                    .push(format!("IDENTITY_CONFLICT:{index}"));
                continue;
            }
            let existing = by_guid.or(by_host).cloned();
            let id = existing.as_ref().map_or_else(DeviceId::new, |d| d.id);
            let mut aliases = existing
                .as_ref()
                .map_or_else(Vec::new, |d| d.aliases.clone());
            if let Some(d) = &existing {
                if d.host != host {
                    aliases.push(d.host.clone());
                }
                plan.report.updated_devices += 1;
            }
            let device = Device {
                id,
                host: host.clone(),
                aliases,
                ad_guid: guid.or(existing.as_ref().and_then(|d| d.ad_guid)),
                name: old.name,
                domain: old.domain,
                ou: old.dn,
                os: old.os,
                source: old.source,
                site: String::new(),
                disabled: old.disabled,
                inventory_metadata: serde_json::json!({"ip":old.ip,"groups":old.groups,"status":old.status,"ad_details":old.ad_details}),
            };
            if let Some(pos) = known.iter().position(|d| d.id == id) {
                known[pos] = device.clone();
            } else {
                known.push(device.clone());
            }
            if !old.details.trim().is_empty() {
                let details: Value = serde_json::from_str(&old.details)?;
                if !details.is_object() {
                    return Err(Error::Invalid("inventory details must be an object"));
                }
                let scanned_at = chrono::DateTime::parse_from_rfc3339(&old.last_scan)
                    .ok()
                    .map(|t| t.with_timezone(&Utc));
                if let Some(scanned_at) = scanned_at {
                    let ram = installed_ram(&details);
                    if ram.is_none() {
                        plan.report
                            .warnings
                            .insert("RAM_EXACT_PHYSICAL_BYTES_UNAVAILABLE".into());
                    }
                    let software = details.get("Software").and_then(Value::as_array);
                    // Old scanner covers HKLM only, not per-user installs: preserve explicit partial coverage.
                    let software_rows = software
                        .into_iter()
                        .flatten()
                        .map(|s| InstalledSoftware {
                            application_id: None,
                            name: string(s, "Name"),
                            publisher: string(s, "Publisher"),
                            version: string(s, "Version"),
                            channel: None,
                            architecture: None,
                        })
                        .collect();
                    plan.scans.push(DeviceScan {
                        id: ScanId::new(),
                        device_id: id,
                        scanned_at,
                        installed_ram_bytes: ram,
                        software_state: ProviderState::Partial,
                        software: software_rows,
                        details,
                    });
                    plan.report
                        .warnings
                        .insert("LEGACY_SOFTWARE_HKLM_ONLY_PARTIAL_UNMAPPED".into());
                } else {
                    plan.report
                        .errors
                        .push(format!("SCAN_TIMESTAMP_INVALID:{index}"));
                }
            }
            plan.map.push(("device", host, id.to_string()));
            plan.devices.push(device);
        }
        let mut collection_keys = BTreeSet::new();
        for (index, row) in collections.iter().enumerate() {
            let old: LegacyCollection = serde_json::from_str(row)?;
            if old.id.is_empty() || !collection_keys.insert(old.id.clone()) {
                plan.report
                    .errors
                    .push(format!("COLLECTION_DUPLICATE_OR_EMPTY_ID:{index}"));
                continue;
            }
            let kind = if old.dynamic {
                let converted = old
                    .rules
                    .iter()
                    .map(convert_rule)
                    .collect::<Option<Vec<_>>>();
                match converted {
                    Some(items) if !items.is_empty() => CollectionKind::Dynamic(if old.match_any {
                        Expression::Or(items)
                    } else {
                        Expression::And(items)
                    }),
                    _ => {
                        plan.report
                            .warnings
                            .insert("COLLECTION_RULE_REQUIRES_REVIEW".into());
                        plan.add_review("collection",old.name.clone(),serde_json::json!({"legacy_id":old.id,"rules":old.rules,"match_any":old.match_any,"enabled":false,"reason":"Unsupported legacy semantics; recreate as a typed group after review"}));
                        continue;
                    }
                }
            } else {
                let mut ids = BTreeSet::new();
                for host in old.hosts {
                    let id = canonical_host(&host)
                        .ok()
                        .and_then(|h| known.iter().find(|d| d.host == h || d.aliases.contains(&h)))
                        .map(|d| d.id);
                    if let Some(id) = id {
                        ids.insert(id);
                    } else {
                        plan.report
                            .errors
                            .push(format!("STATIC_MEMBER_UNRESOLVED:{index}"));
                    }
                }
                CollectionKind::Static(ids)
            };
            let c = Collection {
                id: CollectionId::new(),
                parent_id: None,
                name: old.name,
                revision: 0,
                kind,
            };
            let check = InventorySnapshot {
                generation: snapshot.generation,
                devices: known
                    .iter()
                    .map(|d| DeviceRecord {
                        device: d.clone(),
                        scan: None,
                    })
                    .collect(),
                collections: vec![],
            };
            if check.validate_collection(&c).is_err() {
                plan.report
                    .errors
                    .push(format!("COLLECTION_INVALID:{index}"));
                continue;
            }
            plan.map.push(("collection", old.id, c.id.to_string()));
            plan.collections.push(c);
        }
        source.execute_batch("COMMIT;")?;
        if hash(&source_bytes(path)?) != plan.report.source_sha256 {
            return Err(Error::Invalid("source changed while reading"));
        }
        plan.report.devices = plan.devices.len();
        plan.report.scans = plan.scans.len();
        plan.report.collections = plan.collections.len();
        plan.report.review_items = plan.reviews.len();
        plan.report
            .warnings
            .insert("CREDENTIALS_CONFIG_AND_LOGS_NOT_IMPORTED".into());
        Ok(plan)
    }

    pub fn apply_import(&mut self, plan: &ImportPlan) -> Result<ImportOutcome> {
        if !plan.report.can_apply() {
            return Err(Error::Invalid("dry-run has blocking errors"));
        }
        if hash(&source_bytes(&plan.source)?) != plan.report.source_sha256 {
            return Err(Error::Invalid("source changed since dry-run"));
        }
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let imported: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM imports WHERE source_hash=?1)",
            [&plan.report.source_sha256],
            |r| r.get(0),
        )?;
        if imported {
            return Ok(ImportOutcome::AlreadyImported);
        }
        let current: u64 = tx.query_row("SELECT generation FROM metadata WHERE id=1", [], |r| {
            r.get(0)
        })?;
        if current != plan.report.base_generation {
            return Err(Error::Conflict);
        }
        tx.execute(
            "INSERT INTO imports VALUES(?1,?2,?3,?4)",
            rusqlite::params![
                plan.report.source_sha256,
                plan.report.format,
                Utc::now().to_rfc3339(),
                serde_json::to_string(&plan.report)?
            ],
        )?;
        for device in &plan.devices {
            write_device(&tx, device)?;
        }
        for scan in &plan.scans {
            write_scan(&tx, scan)?;
        }
        for c in &plan.collections {
            write_collection(&tx, c, None)?;
        }
        for package in &plan.packages {
            write_package(&tx, package)?;
        }
        for review in &plan.reviews {
            tx.execute(
                "INSERT INTO import_review VALUES(?1,?2,?3,?4,?5)",
                rusqlite::params![
                    review.id,
                    review.source_sha256,
                    review.kind,
                    review.name,
                    serde_json::to_string(review)?
                ],
            )?;
        }
        for (entity, key, id) in &plan.map {
            if *entity == "package" {
                tx.execute("INSERT INTO package_lineage VALUES(?1,?2,?3) ON CONFLICT(origin_key,legacy_key) DO NOTHING",rusqlite::params![plan.origin_key,key,id])?;
            }
            tx.execute(
                "INSERT INTO legacy_ids VALUES(?1,?2,?3,?4)",
                rusqlite::params![plan.report.source_sha256, entity, key, id],
            )?;
        }
        bump(&tx)?;
        tx.commit()?;
        Ok(ImportOutcome::Applied)
    }
}

fn bounded_source_rows(db: &Connection, sql: &str) -> Result<Vec<String>> {
    let mut query = db.prepare(sql)?;
    let rows = query.query_map([], |r| r.get::<_, String>(0))?;
    let mut out = vec![];
    let mut bytes = 0;
    for row in rows {
        let row = row?;
        bytes += row.len();
        if bytes as u64 > MAX_SOURCE_BYTES || out.len() >= MAX_ENTITIES {
            return Err(Error::Invalid("too many/large source entities"));
        }
        out.push(row);
    }
    Ok(out)
}
fn string(v: &Value, field: &str) -> String {
    v.get(field).and_then(Value::as_str).unwrap_or("").into()
}
fn installed_ram(details: &Value) -> Option<u64> {
    let modules = details.get("MemoryModules")?.as_array()?;
    if modules.is_empty()
        || details
            .get("Warnings")
            .and_then(Value::as_array)
            .is_some_and(|w| {
                w.iter()
                    .any(|v| v.as_str().is_some_and(|s| s.starts_with("MemoryModules:")))
            })
    {
        return None;
    }
    modules.iter().try_fold(0u64, |sum, m| {
        let capacity = m
            .get("Capacity")
            .and_then(|v| v.as_u64().or_else(|| v.as_str()?.parse().ok()))?;
        if capacity == 0 {
            None
        } else {
            sum.checked_add(capacity)
        }
    })
}

// Only lossless text rules are migrated in M1. Old rounded MemoryGB/JSON comparisons
// are BLOCKING, never silently changed into exact installed-physical-RAM predicates.
fn convert_rule(rule: &LegacyRule) -> Option<Expression> {
    let field = match rule.field.as_str() {
        "Host" => TextField::Host,
        "Name" => TextField::Name,
        "OS" => TextField::Os,
        "Domain" => TextField::Domain,
        "OU" => TextField::Ou,
        "Source" => TextField::Source,
        _ => return None,
    };
    let op = match rule.operator.as_str() {
        "equals" => TextOp::Equals,
        "not equals" => TextOp::NotEquals,
        "contains" => TextOp::Contains,
        "not contains" => TextOp::NotContains,
        "starts with" => TextOp::StartsWith,
        "is empty" => TextOp::IsEmpty,
        _ => return None,
    };
    Some(Expression::Text {
        field,
        op,
        value: rule.value.clone(),
    })
}
#[derive(Deserialize, Default)]
#[serde(default)]
struct LegacyComputer {
    host: String,
    name: String,
    domain: String,
    os: String,
    guid: String,
    dn: String,
    disabled: bool,
    source: String,
    last_scan: String,
    details: String,
    ip: String,
    groups: String,
    status: String,
    ad_details: String,
}
#[derive(Deserialize, Default)]
#[serde(default)]
struct LegacyCollection {
    id: String,
    name: String,
    dynamic: bool,
    match_any: bool,
    rules: Vec<LegacyRule>,
    hosts: Vec<String>,
}
#[derive(Deserialize, Serialize)]
struct LegacyRule {
    field: String,
    operator: String,
    value: String,
}

/// Diagnostic identity map for tests/tools without exposing raw legacy data.
pub fn imported_identity_map(store: &Store, source_hash: &str) -> Result<BTreeMap<String, String>> {
    let mut query = store.db.prepare(
        "SELECT entity || ':' || legacy_key,new_id FROM legacy_ids WHERE source_hash=?1",
    )?;
    let rows = query.query_map([source_hash], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}
