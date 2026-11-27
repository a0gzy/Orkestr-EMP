use chrono::{DateTime, Utc};
use orkestr_domain::{DeviceId, PackageRevisionId};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum InstallerExecution {
    Msi {},
    /// Vendor-supplied silent command line, passed directly to the executable.
    Exe {
        arguments: String,
    },
}
impl Default for InstallerExecution {
    fn default() -> Self {
        Self::Msi {}
    }
}
impl InstallerExecution {
    pub fn is_msi(&self) -> bool {
        matches!(self, Self::Msi {})
    }

    pub fn extension(&self) -> &'static str {
        match self {
            Self::Msi {} => "msi",
            Self::Exe { .. } => "exe",
        }
    }

    pub fn valid(&self) -> bool {
        match self {
            Self::Msi {} => true,
            Self::Exe { arguments } => {
                !arguments.trim().is_empty()
                    && arguments.len() <= 2048
                    && !arguments.chars().any(char::is_control)
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedInstaller {
    #[serde(default, skip_serializing_if = "is_false")]
    pub allow_upgrade: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version_components: Option<u8>,
    pub name: String,
    pub version: String,
    pub sha256: String,
    pub product_code: Uuid,
    #[serde(default, skip_serializing_if = "InstallerExecution::is_msi")]
    pub execution: InstallerExecution,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registry_key: Option<String>,
}
fn is_false(value: &bool) -> bool {
    !*value
}
impl ManagedInstaller {
    pub fn valid(&self) -> bool {
        if self
            .version_components
            .is_some_and(|n| !(1..=16).contains(&n))
            || ((self.allow_upgrade || self.version_components.is_some())
                && orkestr_domain::NumericVersion::parse(&self.version).is_err())
        {
            return false;
        }
        !self.name.trim().is_empty()
            && self.name.len() <= 128
            && !self.name.chars().any(char::is_control)
            && !self.version.trim().is_empty()
            && self.version.len() <= 80
            && !self.version.chars().any(char::is_control)
            && self.execution.valid()
            && orkestr_library::valid_hash(&self.sha256)
            && match &self.registry_key {
                Some(key) => {
                    !self.execution.is_msi()
                        && !key.is_empty()
                        && key.len() <= 128
                        && key
                            .bytes()
                            .all(|c| c.is_ascii_alphanumeric() || b"{}_.-".contains(&c))
                }
                None => !self.product_code.is_nil(),
            }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanDeployment {
    pub device_id: DeviceId,
    pub package_revision_id: PackageRevisionId,
    pub connection_id: Uuid,
    pub connection_revision: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeploymentChange {
    pub expected_revision: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeploymentState {
    WaitingApproval,
    Queued,
    Running,
    Succeeded,
    RebootRequired,
    Failed,
    NeedsReconciliation,
    ReconcileQueued,
    Reconciling,
    Cancelled,
}
impl DeploymentState {
    pub fn holds_device(self) -> bool {
        matches!(
            self,
            Self::Queued
                | Self::Running
                | Self::NeedsReconciliation
                | Self::ReconcileQueued
                | Self::Reconciling
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Detection {
    Present,
    Absent,
    DifferentVersion,
    Unknown,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeploymentAttempt {
    #[serde(default)]
    pub transfer_bytes: Option<u64>,
    #[serde(default)]
    pub transfer_ms: Option<u64>,
    pub approved_by: Uuid,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub code: String,
    pub exit_code: Option<i32>,
    pub detection: Detection,
    pub safe_to_retry: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Deployment {
    #[serde(default)]
    pub manual_resolution: Option<ManualResolution>,
    pub id: Uuid,
    pub revision: u64,
    pub plan: PlanDeployment,
    pub installer: ManagedInstaller,
    pub host: String,
    pub created_by: Uuid,
    pub approved_by: Option<Uuid>,
    pub approved_at: Option<DateTime<Utc>>,
    pub action_by: Option<Uuid>,
    pub credential_revision: u64,
    pub state: DeploymentState,
    pub code: String,
    pub created_at: DateTime<Utc>,
    pub attempts: Vec<DeploymentAttempt>,
    pub reconciliation: Option<Detection>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ManualResolution {
    pub actor_id: Uuid,
    pub at: DateTime<Utc>,
    pub evidence: String,
}
