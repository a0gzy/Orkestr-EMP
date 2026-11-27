use crate::*;
use chrono::{DateTime, Utc};
use orkestr_domain::{PackageRevisionId, StepKind};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum WorkflowStep {
    Install {
        installer: ManagedInstaller,
    },
    Command {
        executable: String,
        arguments: String,
        success_codes: Vec<i32>,
        timeout_seconds: u32,
    },
    PowerShell {
        script: String,
        timeout_seconds: u32,
    },
    FileCopy {
        sha256: String,
        destination: String,
        overwrite: bool,
    },
    Sleep {
        seconds: u32,
    },
    Message {
        session_id: u32,
        text: String,
        seconds: u32,
    },
    Reboot {
        delay_seconds: u32,
    },
    Logoff {
        session_id: u32,
    },
    NestedPackage {
        revision_id: PackageRevisionId,
    },
    SystemScan {},
}
pub fn valid_remote_path(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 4
        && bytes.len() <= 512
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && bytes[2] == b'\\'
        && !path[3..].contains([':', '/', '"', '<', '>', '|', '?', '*'])
        && !path.chars().any(char::is_control)
        && path
            .split('\\')
            .skip(1)
            .all(|p| !p.is_empty() && p != "." && p != ".." && !p.ends_with([' ', '.']))
}
impl WorkflowStep {
    pub fn valid(&self) -> bool {
        match self {
            Self::Install { installer } => installer.valid(),
            Self::Command {
                executable,
                arguments,
                success_codes,
                timeout_seconds,
            } => {
                valid_remote_path(executable)
                    && executable.to_ascii_lowercase().ends_with(".exe")
                    && arguments.len() <= 4096
                    && !arguments.chars().any(char::is_control)
                    && !success_codes.is_empty()
                    && success_codes.len() <= 16
                    && (1..=900).contains(timeout_seconds)
            }
            Self::PowerShell {
                script,
                timeout_seconds,
            } => {
                !script.trim().is_empty()
                    && script.len() <= 16384
                    && !script.contains('\0')
                    && (1..=900).contains(timeout_seconds)
            }
            Self::FileCopy {
                sha256,
                destination,
                ..
            } => orkestr_library::valid_hash(sha256) && valid_remote_path(destination),
            Self::Sleep { seconds } => *seconds <= 3600,
            Self::Message { text, seconds, .. } => {
                !text.trim().is_empty()
                    && text.len() <= 1024
                    && !text.chars().any(char::is_control)
                    && (1..=3600).contains(seconds)
            }
            Self::Reboot { delay_seconds } => *delay_seconds <= 3600,
            Self::Logoff { .. } | Self::NestedPackage { .. } | Self::SystemScan {} => true,
        }
    }
    pub fn step_kind(&self) -> StepKind {
        match self {
            Self::Install { .. } => StepKind::Install,
            Self::Command { .. } => StepKind::Command,
            Self::PowerShell { .. } => StepKind::PowerShell,
            Self::FileCopy { .. } => StepKind::FileCopy,
            Self::Sleep { .. } => StepKind::Sleep,
            Self::Message { .. } => StepKind::Message,
            Self::Reboot { .. } => StepKind::Reboot,
            Self::Logoff { .. } => StepKind::Logoff,
            Self::NestedPackage { .. } => StepKind::NestedPackage,
            Self::SystemScan {} => StepKind::SystemScan,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowDefinition {
    pub name: String,
    pub version: String,
    pub steps: Vec<WorkflowStep>,
}
impl WorkflowDefinition {
    pub fn valid(&self) -> bool {
        !self.name.trim().is_empty()
            && self.name.len() <= 128
            && !self.version.trim().is_empty()
            && self.version.len() <= 80
            && !self.steps.is_empty()
            && self.steps.len() <= 64
            && self.steps.iter().all(WorkflowStep::valid)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StepCheckpoint {
    #[serde(default)]
    pub transfer: Option<TransferStatistics>,
    pub index: usize,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub code: String,
    pub exit_code: Option<i32>,
    pub evidence: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TransferStatistics {
    pub bytes: u64,
    pub elapsed_ms: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkflowRun {
    pub id: Uuid,
    pub revision: u64,
    pub plan: PlanDeployment,
    pub definition: WorkflowDefinition,
    pub host: String,
    pub created_by: Uuid,
    pub action_by: Option<Uuid>,
    pub credential_revision: u64,
    pub state: DeploymentState,
    pub code: String,
    pub created_at: DateTime<Utc>,
    pub next_step: usize,
    pub checkpoints: Vec<StepCheckpoint>,
    pub reboot_required: bool,
    #[serde(default)]
    pub cancel_requested: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublishRelease {
    #[serde(default)]
    pub allow_upgrade: bool,
    #[serde(default)]
    pub installed_version: Option<String>,
    pub release: ChangeRelease,
    pub product_code: Uuid,
    pub execution: InstallerExecution,
    #[serde(default)]
    pub registry_key: Option<String>,
}
