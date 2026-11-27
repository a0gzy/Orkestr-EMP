use crate::DomainError;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

macro_rules! id {
    ($name:ident) => {
        #[derive(
            Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(pub Uuid);
        impl $name {
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }
        }
        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }
        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.0.fmt(f)
            }
        }
    };
}
id!(DeviceId);
id!(ScanId);
id!(CollectionId);
id!(PackageId);
id!(PackageRevisionId);
id!(ApplicationId);

/// Adapted from Open Inventory's MIT `valid_host`; rejects malformed DNS labels too.
/// IP addresses are allowed as labels, but are never used to merge identities.
pub fn canonical_host(host: &str) -> Result<String, DomainError> {
    let host = host.trim().trim_end_matches('.');
    if host.is_empty()
        || host.len() > 253
        || host.split('.').any(|label| {
            label.is_empty()
                || label.len() > 63
                || label.starts_with('-')
                || label.ends_with('-')
                || !label
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        })
    {
        return Err(DomainError::InvalidHost);
    }
    Ok(host.to_ascii_lowercase())
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Device {
    pub id: DeviceId,
    pub host: String,
    pub aliases: Vec<String>,
    pub ad_guid: Option<Uuid>,
    pub name: String,
    pub domain: String,
    pub ou: String,
    pub os: String,
    pub source: String,
    pub site: String,
    pub disabled: bool,
    /// Private directory/source metadata, including discovery records without a scan.
    #[serde(default)]
    pub inventory_metadata: serde_json::Value,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProviderState {
    Complete,
    Partial,
    Failed,
    NotCollected,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledSoftware {
    pub application_id: Option<ApplicationId>,
    pub name: String,
    pub publisher: String,
    pub version: String,
    pub channel: Option<String>,
    pub architecture: Option<String>,
}

/// Installed physical bytes = sum of Win32_PhysicalMemory.Capacity, not usable/free RAM.
/// GiB is exactly 2^30 bytes. Missing/partial module data must stay None.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DeviceScan {
    pub id: ScanId,
    pub device_id: DeviceId,
    pub scanned_at: DateTime<Utc>,
    pub installed_ram_bytes: Option<u64>,
    pub software_state: ProviderState,
    pub software: Vec<InstalledSoftware>,
    /// Private inventory details, including all legacy F21 sections. Not a safe public export.
    pub details: serde_json::Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DeviceRecord {
    pub device: Device,
    pub scan: Option<DeviceScan>,
}

/// Names adapted from Open Deploy's MIT KINDS; no execution/defaults are transferred.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum StepKind {
    Install,
    Command,
    PowerShell,
    #[serde(rename = "File Copy")]
    FileCopy,
    Sleep,
    Message,
    Reboot,
    Logoff,
    #[serde(rename = "Nested Package")]
    NestedPackage,
    #[serde(rename = "System Scan")]
    SystemScan,
}

/// Opaque legacy definition for review only. Never executable in M1.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PackageStep {
    pub kind: StepKind,
    pub definition: serde_json::Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PackageRevision {
    pub id: PackageRevisionId,
    pub package_id: PackageId,
    pub name: String,
    pub version: String,
    pub steps: Vec<PackageStep>,
    /// Empty after legacy import: no installer is read, trusted, downloaded or approved.
    pub artifact_hashes: Vec<String>,
    /// Original descriptive metadata; never interpreted as execution policy.
    #[serde(default)]
    pub metadata: serde_json::Value,
}
