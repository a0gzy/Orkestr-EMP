use crate::{Error, Result};
use chrono::{DateTime, Utc};
use orkestr_domain::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

pub const MAX_REPORT_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_PROVIDER_ROWS: usize = 10_000;

/// F21 providers, including every independent section of the original Inventory.
/// This is a data contract, NOT a list of implemented network adapters.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum InventoryProvider {
    ComputerSystem,
    OperatingSystem,
    Bios,
    Cpus,
    MemoryModules,
    Nics,
    DiskDrives,
    LogicalDisks,
    Displays,
    Hardware,
    HotFixes,
    LocalUsers,
    LocalGroups,
    Printers,
    Processes,
    Services,
    Shares,
    Environment,
    WindowsFeatures,
    Startup,
    TaskSchedules,
    SoftwareMachine,
    SoftwareUsers,
}
impl InventoryProvider {
    pub const ALL: [Self; 23] = [
        Self::ComputerSystem,
        Self::OperatingSystem,
        Self::Bios,
        Self::Cpus,
        Self::MemoryModules,
        Self::Nics,
        Self::DiskDrives,
        Self::LogicalDisks,
        Self::Displays,
        Self::Hardware,
        Self::HotFixes,
        Self::LocalUsers,
        Self::LocalGroups,
        Self::Printers,
        Self::Processes,
        Self::Services,
        Self::Shares,
        Self::Environment,
        Self::WindowsFeatures,
        Self::Startup,
        Self::TaskSchedules,
        Self::SoftwareMachine,
        Self::SoftwareUsers,
    ];
    pub fn section(self) -> &'static str {
        match self {
            Self::ComputerSystem => "ComputerSystem",
            Self::OperatingSystem => "OperatingSystem",
            Self::Bios => "BIOS",
            Self::Cpus => "CPUs",
            Self::MemoryModules => "MemoryModules",
            Self::Nics => "NICs",
            Self::DiskDrives => "DiskDrives",
            Self::LogicalDisks => "Disks",
            Self::Displays => "Displays",
            Self::Hardware => "Hardware",
            Self::HotFixes => "HotFixes",
            Self::LocalUsers => "LocalUsers",
            Self::LocalGroups => "LocalGroups",
            Self::Printers => "Printers",
            Self::Processes => "Processes",
            Self::Services => "Services",
            Self::Shares => "Shares",
            Self::Environment => "Environment",
            Self::WindowsFeatures => "WindowsFeatures",
            Self::Startup => "Startup",
            Self::TaskSchedules => "TaskSchedules",
            Self::SoftwareMachine => "SoftwareMachine",
            Self::SoftwareUsers => "SoftwareUsers",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProviderFailure {
    AccessDenied,
    Timeout,
    Unavailable,
    Unsupported,
    InvalidData,
    Interrupted,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProviderReport {
    pub provider: InventoryProvider,
    pub state: ProviderState,
    pub failure: Option<ProviderFailure>,
    /// Private raw evidence: objects, not scripts to execute or strings to print to logs.
    pub rows: Vec<Value>,
}
impl ProviderReport {
    pub fn not_collected(provider: InventoryProvider) -> Self {
        Self {
            provider,
            state: ProviderState::NotCollected,
            failure: None,
            rows: vec![],
        }
    }
    fn invalid_data(&mut self) {
        self.state = ProviderState::Partial;
        self.failure = Some(ProviderFailure::InvalidData);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InventoryState {
    Complete,
    Partial,
    Failed,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum SoftwareScope {
    Machine,
    User { sid: String },
}

/// No application/library identity is inferred from a display name (matching is M4).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SoftwareEvidence {
    pub name: String,
    pub publisher: String,
    pub version_raw: String,
    pub version_normalized: Option<String>,
    pub architecture: Option<String>,
    pub scope: SoftwareScope,
    pub product_code: Option<Uuid>,
    pub registry_key: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InventoryReport {
    pub id: Uuid,
    pub device_id: DeviceId,
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
    pub providers: Vec<ProviderReport>,
}

/// Constructed by validation, not deserializable into unchecked persistence input.
pub struct NormalizedInventory {
    report: InventoryReport,
    state: InventoryState,
    software: Vec<SoftwareEvidence>,
    scan: Option<DeviceScan>,
}
impl NormalizedInventory {
    pub fn report(&self) -> &InventoryReport {
        &self.report
    }
    pub fn state(&self) -> InventoryState {
        self.state
    }
    pub fn software(&self) -> &[SoftwareEvidence] {
        &self.software
    }
    pub fn scan(&self) -> Option<&DeviceScan> {
        self.scan.as_ref()
    }
}

impl InventoryReport {
    /// Summary of a validated report; missing providers cannot imply completeness.
    pub fn state(&self) -> InventoryState {
        if self.providers.len() == InventoryProvider::ALL.len()
            && self
                .providers
                .iter()
                .all(|p| p.state == ProviderState::Complete)
        {
            InventoryState::Complete
        } else if self
            .providers
            .iter()
            .any(|p| matches!(p.state, ProviderState::Complete | ProviderState::Partial))
        {
            InventoryState::Partial
        } else if self
            .providers
            .iter()
            .any(|p| p.state == ProviderState::Failed)
        {
            InventoryState::Failed
        } else {
            InventoryState::Unknown
        }
    }

    pub fn normalize(&self) -> Result<NormalizedInventory> {
        if self.finished_at < self.started_at || self.providers.len() > InventoryProvider::ALL.len()
        {
            return Err(Error::Invalid("invalid scan times or provider count"));
        }
        if serde_json::to_vec(self)
            .map_err(|_| Error::Invalid("invalid report"))?
            .len()
            > MAX_REPORT_BYTES
        {
            return Err(Error::Invalid("inventory report exceeds 4 MiB"));
        }
        let mut providers = BTreeMap::new();
        for provider in &self.providers {
            if provider.rows.len() > MAX_PROVIDER_ROWS
                || provider.rows.iter().any(|r| !r.is_object())
            {
                return Err(Error::Invalid("invalid provider rows"));
            }
            match provider.state {
                ProviderState::Complete if provider.failure.is_some() => {
                    return Err(Error::Invalid("complete provider has a failure"))
                }
                ProviderState::Failed
                    if provider.failure.is_none() || !provider.rows.is_empty() =>
                {
                    return Err(Error::Invalid(
                        "failed provider must have a code and no rows",
                    ))
                }
                ProviderState::NotCollected
                    if provider.failure.is_some() || !provider.rows.is_empty() =>
                {
                    return Err(Error::Invalid("uncollected provider cannot have results"))
                }
                _ => {}
            }
            if providers
                .insert(provider.provider, provider.clone())
                .is_some()
            {
                return Err(Error::Invalid("duplicate inventory provider"));
            }
        }
        for kind in InventoryProvider::ALL {
            providers
                .entry(kind)
                .or_insert_with(|| ProviderReport::not_collected(kind));
        }
        let memory = providers
            .get_mut(&InventoryProvider::MemoryModules)
            .unwrap();
        let installed_ram_bytes = if memory.state == ProviderState::Complete {
            let sum = memory
                .rows
                .iter()
                .try_fold(0u64, |total, row| {
                    let value = &row["Capacity"];
                    let n = value
                        .as_u64()
                        .or_else(|| value.as_str()?.parse::<u64>().ok())?;
                    if n == 0 {
                        return None;
                    }
                    total.checked_add(n)
                })
                .filter(|n| *n > 0);
            if sum.is_none() {
                memory.invalid_data();
            }
            sum
        } else {
            None
        };
        let mut software = BTreeSet::new();
        for kind in [
            InventoryProvider::SoftwareMachine,
            InventoryProvider::SoftwareUsers,
        ] {
            let provider = providers.get_mut(&kind).unwrap();
            let mut invalid = false;
            for row in &provider.rows {
                if let Some(app) = normalize_software(row, kind) {
                    software.insert(app);
                } else {
                    invalid = true;
                }
            }
            if invalid {
                provider.invalid_data();
            }
        }
        let machine = providers[&InventoryProvider::SoftwareMachine].state;
        let users = providers[&InventoryProvider::SoftwareUsers].state;
        let software_state =
            if machine == ProviderState::Complete && users == ProviderState::Complete {
                ProviderState::Complete
            } else if [machine, users]
                .iter()
                .any(|s| matches!(s, ProviderState::Complete | ProviderState::Partial))
            {
                ProviderState::Partial
            } else if [machine, users].contains(&ProviderState::Failed) {
                ProviderState::Failed
            } else {
                ProviderState::NotCollected
            };
        let state = if providers
            .values()
            .all(|p| p.state == ProviderState::Complete)
        {
            InventoryState::Complete
        } else if providers
            .values()
            .any(|p| matches!(p.state, ProviderState::Complete | ProviderState::Partial))
        {
            InventoryState::Partial
        } else if providers.values().any(|p| p.state == ProviderState::Failed) {
            InventoryState::Failed
        } else {
            InventoryState::Unknown
        };
        let software: Vec<_> = software.into_iter().collect();
        let report = Self {
            providers: providers.into_values().collect(),
            ..self.clone()
        };
        let scan = if matches!(state, InventoryState::Complete | InventoryState::Partial) {
            let mut details = serde_json::Map::new();
            for p in &report.providers {
                // Do not fabricate successful empty sections for Failed/NotCollected.
                if matches!(p.state, ProviderState::Complete | ProviderState::Partial) {
                    details.insert(p.provider.section().into(), Value::Array(p.rows.clone()));
                }
            }
            details.insert(
                "OrkestrInventory".into(),
                serde_json::json!({
                    "schema": 1, "report_id": report.id, "state": state,
                    "providers": report.providers, "software": software,
                    "started_at": report.started_at, "finished_at": report.finished_at,
                }),
            );
            Some(DeviceScan {
                id: ScanId(report.id),
                device_id: report.device_id,
                // Collection freshness starts at observation start, not when a slow job finishes.
                scanned_at: report.started_at,
                installed_ram_bytes,
                software_state,
                software: software
                    .iter()
                    .map(|s| InstalledSoftware {
                        application_id: None,
                        name: s.name.clone(),
                        publisher: s.publisher.clone(),
                        version: s.version_raw.clone(),
                        channel: None,
                        architecture: s.architecture.clone(),
                    })
                    .collect(),
                details: Value::Object(details),
            })
        } else {
            None
        };
        Ok(NormalizedInventory {
            report,
            state,
            software,
            scan,
        })
    }
}

fn text(row: &Value, field: &str) -> Option<String> {
    match row.get(field) {
        None | Some(Value::Null) => Some(String::new()),
        Some(Value::String(s)) if s.len() <= 1024 && !s.chars().any(char::is_control) => {
            Some(s.trim().into())
        }
        _ => None,
    }
}
fn normalize_software(row: &Value, kind: InventoryProvider) -> Option<SoftwareEvidence> {
    let name = text(row, "name")?
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if name.is_empty() {
        return None;
    }
    let publisher = text(row, "publisher")?
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let version_raw = text(row, "version")?;
    let version_normalized = NumericVersion::parse(&version_raw)
        .ok()
        .map(|n| n.to_string());
    let architecture = match text(row, "architecture")?.to_ascii_lowercase().as_str() {
        "x64" | "amd64" | "64-bit" => Some("x64".into()),
        "x86" | "i386" | "32-bit" => Some("x86".into()),
        "arm64" | "aarch64" => Some("arm64".into()),
        _ => None,
    };
    let scope = if kind == InventoryProvider::SoftwareMachine {
        SoftwareScope::Machine
    } else {
        let sid = text(row, "user_sid")?;
        let parts: Vec<_> = sid.split('-').collect();
        if sid.len() > 184
            || !sid.starts_with("S-1-")
            || parts.len() < 4
            || parts[2..]
                .iter()
                .any(|p| p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()))
        {
            return None;
        }
        SoftwareScope::User { sid }
    };
    let product = text(row, "product_code")?;
    let product_code = if product.is_empty() {
        None
    } else {
        Some(Uuid::parse_str(&product).ok()?)
    };
    let key = text(row, "registry_key")?;
    Some(SoftwareEvidence {
        name,
        publisher,
        version_raw,
        version_normalized,
        architecture,
        scope,
        product_code,
        registry_key: (!key.is_empty()).then_some(key),
    })
}
