use crate::{Error, Result};
use orkestr_domain::*;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, net::IpAddr};
use uuid::Uuid;

pub const MAX_DISCOVERY_BYTES: usize = 256 * 1024;
pub const MAX_DISCOVERY_ROWS: usize = 1000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiscoverySource {
    Manual,
    Csv,
    Ad,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirectoryComputer {
    pub guid: Uuid,
    pub host: String,
    pub name: String,
    pub ou: String,
    pub os: String,
    pub disabled: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Candidate {
    host: String,
    name: Option<String>,
    site: Option<String>,
    directory: Option<DirectoryComputer>,
}

/// Immutable parsed content, not a path that can change between preview and apply.
pub struct DiscoveryBatch {
    source: DiscoverySource,
    candidates: Vec<Candidate>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiscoveryAction {
    Add,
    Update,
    Unchanged,
    Duplicate,
    Conflict,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DiscoveryRow {
    /// One-based input record number, excluding the CSV header/blank manual lines.
    pub row: usize,
    pub host: String,
    pub device_id: Option<DeviceId>,
    pub action: DiscoveryAction,
    pub target_host: Option<String>,
    pub previous_name: Option<String>,
    pub name: Option<String>,
    pub previous_site: Option<String>,
    pub site: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub directory: Option<DirectoryComputer>,
}

/// Cannot be deserialized or edited into an arbitrary write plan.
pub struct DiscoveryPreview {
    generation: u64,
    baseline: Vec<Device>,
    rows: Vec<DiscoveryRow>,
    writes: Vec<Device>,
}
impl DiscoveryPreview {
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn rows(&self) -> &[DiscoveryRow] {
        &self.rows
    }
    pub fn can_apply(&self) -> bool {
        !self
            .rows
            .iter()
            .any(|r| r.action == DiscoveryAction::Conflict)
    }
    pub fn writes(&self) -> &[Device] {
        &self.writes
    }
    pub fn is_current(&self, snapshot: &InventorySnapshot) -> bool {
        self.generation == snapshot.generation
            && self.baseline.len() == snapshot.devices.len()
            && self
                .baseline
                .iter()
                .zip(&snapshot.devices)
                .all(|(a, b)| a == &b.device)
    }
}

fn input(raw: &str) -> Result<&str> {
    if raw.len() > MAX_DISCOVERY_BYTES {
        return Err(Error::Invalid("discovery input exceeds 256 KiB"));
    }
    Ok(raw.strip_prefix('\u{feff}').unwrap_or(raw))
}
fn label(raw: &str) -> Result<Option<String>> {
    if raw.len() > 256 || raw.chars().any(char::is_control) {
        return Err(Error::Invalid(
            "label exceeds 256 bytes or contains controls",
        ));
    }
    let raw = raw.trim();
    Ok((!raw.is_empty()).then(|| raw.to_owned()))
}
fn host(raw: &str) -> Result<String> {
    let host = canonical_host(raw).map_err(|_| Error::Invalid("invalid DNS hostname"))?;
    if host.parse::<IpAddr>().is_ok() || host.bytes().all(|b| b.is_ascii_digit() || b == b'.') {
        return Err(Error::Invalid("use a DNS hostname, not an IP address"));
    }
    Ok(host)
}

// csv intentionally accepts some malformed quoting. Discovery must not guess where a
// record ends: validate strict field quoting before using csv for the actual decoding.
fn csv_quotes(raw: &str) -> Result<()> {
    let mut state = 0;
    for b in raw.bytes() {
        state = match (state, b) {
            (2, b'"') => 3,
            (2, _) => 2,
            (3, b'"') => 2,
            (_, b',' | b'\r' | b'\n') => 0,
            (0, b'"') => 2,
            (1, b'"') | (3, _) => return Err(Error::Invalid("malformed CSV quoting")),
            _ => 1,
        };
    }
    if state == 2 {
        return Err(Error::Invalid("unterminated CSV quote"));
    }
    Ok(())
}

impl DiscoveryBatch {
    /// One hostname per nonempty line. No separators, comments, DNS resolution or wildcards.
    pub fn manual(raw: &str) -> Result<Self> {
        let mut candidates = vec![];
        for line in input(raw)?.lines().filter(|s| !s.trim().is_empty()) {
            if candidates.len() == MAX_DISCOVERY_ROWS {
                return Err(Error::Invalid("discovery exceeds 1000 records"));
            }
            candidates.push(Candidate {
                host: host(line)?,
                name: None,
                site: None,
                directory: None,
            });
        }
        Self::finish(DiscoverySource::Manual, candidates)
    }

    /// UTF-8 CSV, comma delimiter, required `host`, optional `name`/`site` headers.
    /// Unknown/duplicate headers and uneven records fail closed; no credentials accepted.
    pub fn csv(raw: &str) -> Result<Self> {
        let raw = input(raw)?;
        csv_quotes(raw)?;
        let mut reader = csv::ReaderBuilder::new()
            .trim(csv::Trim::All)
            .from_reader(raw.as_bytes());
        let headers = reader
            .headers()
            .map_err(|_| Error::Invalid("invalid CSV header"))?
            .clone();
        let mut columns = BTreeMap::new();
        for (i, header) in headers.iter().enumerate() {
            let key = header.to_ascii_lowercase();
            if !matches!(key.as_str(), "host" | "name" | "site") || columns.insert(key, i).is_some()
            {
                return Err(Error::Invalid("unknown or duplicate CSV header"));
            }
        }
        let host_column = *columns
            .get("host")
            .ok_or(Error::Invalid("CSV requires host"))?;
        let mut candidates = vec![];
        for record in reader.records() {
            if candidates.len() == MAX_DISCOVERY_ROWS {
                return Err(Error::Invalid("discovery exceeds 1000 records"));
            }
            let record = record.map_err(|_| Error::Invalid("invalid CSV record"))?;
            candidates.push(Candidate {
                host: host(&record[host_column])?,
                name: columns
                    .get("name")
                    .map(|i| label(&record[*i]))
                    .transpose()?
                    .flatten(),
                site: columns
                    .get("site")
                    .map(|i| label(&record[*i]))
                    .transpose()?
                    .flatten(),
                directory: None,
            });
        }
        Self::finish(DiscoverySource::Csv, candidates)
    }

    pub fn directory(computers: Vec<DirectoryComputer>) -> Result<Self> {
        if computers.len() > MAX_DISCOVERY_ROWS {
            return Err(Error::Invalid("discovery exceeds 1000 records"));
        }
        let mut candidates = Vec::with_capacity(computers.len());
        for mut computer in computers {
            computer.host = host(&computer.host)?;
            if computer.guid.is_nil()
                || !computer.host.contains('.')
                || computer.ou.len() > 2048
                || computer.ou.chars().any(char::is_control)
            {
                return Err(Error::Invalid("invalid directory computer"));
            }
            label(&computer.os)?;
            candidates.push(Candidate {
                host: computer.host.clone(),
                name: label(&computer.name)?,
                site: None,
                directory: Some(computer),
            });
        }
        Ok(Self {
            source: DiscoverySource::Ad,
            candidates,
        })
    }

    fn finish(source: DiscoverySource, candidates: Vec<Candidate>) -> Result<Self> {
        if candidates.is_empty() {
            return Err(Error::Invalid("empty discovery input"));
        }
        Ok(Self { source, candidates })
    }

    /// Names are exact. A verified directory GUID can identify a rename.
    pub fn preview(&self, snapshot: &InventorySnapshot) -> DiscoveryPreview {
        let mut owners: BTreeMap<String, Vec<&Device>> = BTreeMap::new();
        let mut directory_owners = BTreeMap::new();
        for record in &snapshot.devices {
            if let Some(guid) = record.device.ad_guid {
                directory_owners.insert(guid, &record.device);
            }
            for name in std::iter::once(&record.device.host).chain(&record.device.aliases) {
                let list = owners.entry(name.clone()).or_default();
                if !list.iter().any(|d| d.id == record.device.id) {
                    list.push(&record.device);
                }
            }
        }
        let mut rows: Vec<DiscoveryRow> = vec![];
        let mut writes = vec![];
        let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
        let mut seen_guids = BTreeMap::new();
        let mut touched: BTreeMap<DeviceId, usize> = BTreeMap::new();
        for (index, candidate) in self.candidates.iter().enumerate() {
            let mut row = DiscoveryRow {
                row: index + 1,
                host: candidate.host.clone(),
                device_id: None,
                action: DiscoveryAction::Conflict,
                target_host: None,
                previous_name: None,
                name: candidate.name.clone(),
                previous_site: None,
                site: candidate.site.clone(),
                directory: candidate.directory.clone(),
            };
            if let Some(first) = seen.get(candidate.host.as_str()) {
                if self.candidates[*first] == *candidate {
                    row = rows[*first].clone();
                    row.row = index + 1;
                    row.action = DiscoveryAction::Duplicate;
                    row.device_id = rows[*first].device_id;
                } else {
                    rows[*first].action = DiscoveryAction::Conflict;
                }
                rows.push(row);
                continue;
            }
            seen.insert(&candidate.host, index);
            if let Some(ad) = &candidate.directory {
                if let Some(first) = seen_guids.insert(ad.guid, index) {
                    rows[first].action = DiscoveryAction::Conflict;
                    rows.push(row);
                    continue;
                }
            }
            let owner = owners.get(&candidate.host);
            if owner.is_some_and(|v| v.len() != 1) {
                rows.push(row);
                continue;
            }
            let named = owner.map(|v| v[0]);
            let existing = if let Some(ad) = &candidate.directory {
                let identified = directory_owners.get(&ad.guid).copied();
                if named.is_some_and(|d| {
                    d.ad_guid.is_some_and(|guid| guid != ad.guid)
                        || identified.is_some_and(|a| a.id != d.id)
                        || (d.ad_guid.is_none() && d.host != candidate.host)
                }) {
                    rows.push(row);
                    continue;
                }
                identified.or(named)
            } else {
                named
            };
            let mut device = existing.cloned().unwrap_or_else(|| Device {
                id: DeviceId::new(),
                host: candidate.host.clone(),
                aliases: vec![],
                ad_guid: None,
                name: candidate.host.clone(),
                domain: String::new(),
                ou: String::new(),
                os: String::new(),
                source: format!("{:?}", self.source),
                site: String::new(),
                disabled: false,
                inventory_metadata: serde_json::Value::Null,
            });
            row.device_id = Some(device.id);
            row.target_host = Some(device.host.clone());
            row.previous_name = existing.map(|d| d.name.clone());
            row.previous_site = existing.map(|d| d.site.clone());
            if let Some(first) = touched.insert(device.id, index) {
                // Two different aliases target one UUID: do not choose an update order.
                rows[first].action = DiscoveryAction::Conflict;
                rows.push(row);
                continue;
            }
            if let Some(name) = &candidate.name {
                device.name = name.clone();
            }
            if let Some(site) = &candidate.site {
                device.site = site.clone();
            }
            if let Some(ad) = &candidate.directory {
                device.host.clone_from(&ad.host);
                device.ad_guid = Some(ad.guid);
                device.domain = ad
                    .host
                    .split_once('.')
                    .map_or_else(String::new, |(_, domain)| domain.to_owned());
                device.ou.clone_from(&ad.ou);
                if !ad.os.is_empty() {
                    device.os.clone_from(&ad.os);
                }
                device.disabled |= ad.disabled;
                device.source = "Ad".into();
            }
            row.name = Some(device.name.clone());
            row.site = Some(device.site.clone());
            row.action = match existing {
                None => DiscoveryAction::Add,
                Some(old) if old == &device => DiscoveryAction::Unchanged,
                Some(_) => DiscoveryAction::Update,
            };
            if matches!(row.action, DiscoveryAction::Add | DiscoveryAction::Update) {
                writes.push(device);
            }
            rows.push(row);
        }
        DiscoveryPreview {
            generation: snapshot.generation,
            baseline: snapshot.devices.iter().map(|r| r.device.clone()).collect(),
            rows,
            writes,
        }
    }
}
