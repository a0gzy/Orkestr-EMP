//! M2 wire contract. Secrets intentionally have no Debug or Clone.
use chrono::{DateTime, Utc};
use orkestr_domain::*;
pub use orkestr_inventory::{
    DirectoryComputer, DiscoveryAction, DiscoveryRow, DiscoverySource, InventoryProvider,
    InventoryState, ProviderFailure,
};
mod deployment;
pub use deployment::*;
mod workflow;
pub use workflow::*;
mod rollout;
pub use orkestr_library::{
    ArtifactMetadata, ChangeRelease, DownloadSource, LibraryApplication, LibraryJob,
    LibraryJobState, LibraryTask, NetworkPolicy, ReleaseCandidate, ReleaseState, SaveApplication,
    SoftwareMatch, SourceEvidence, SourcePreview, VerificationPolicy, VersionSource,
};
pub use orkestr_scheduler::*;
pub use rollout::*;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use zeroize::Zeroizing;

pub const API_BODY_LIMIT: usize = 64 * 1024;
pub const EVENT_LIMIT: usize = 128;

/// UI/API content limit, intentionally below the 64 KiB escaped JSON body limit.
/// Offline CLI still supports the larger M3.1 core input limit.
pub const DISCOVERY_TEXT_LIMIT: usize = 24 * 1024;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrepareDiscovery {
    pub format: DiscoverySource,
    pub text: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PreparedDiscovery {
    pub id: Uuid,
    pub generation: u64,
    pub expires_at: DateTime<Utc>,
    pub rows: Vec<DiscoveryRow>,
    pub changes: usize,
    pub can_apply: bool,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplyDiscovery {
    pub preview_id: Uuid,
    pub confirmed: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppliedDiscovery {
    pub preview_id: Uuid,
    pub changes: usize,
    pub generation: u64,
    /// Generated basename only; no client-selected server filesystem path.
    pub backup: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProviderStatus {
    pub provider: InventoryProvider,
    pub state: ProviderState,
    pub failure: Option<ProviderFailure>,
    pub rows: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InventoryRunStatus {
    pub id: Uuid,
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
    pub state: InventoryState,
    pub providers: Vec<ProviderStatus>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeviceInventoryStatus {
    pub device_id: DeviceId,
    pub generation: u64,
    pub latest: Option<InventoryRunStatus>,
    /// Provider states of the current scan head, which may predate the latest failure.
    pub head: Option<InventoryRunStatus>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    Admin,
    Operator,
    Viewer,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Actor {
    pub id: Uuid,
    pub label: String,
    pub role: Role,
    pub expires_at: DateTime<Utc>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IssueToken {
    pub label: String,
    pub role: Role,
    pub days: u16,
}
#[derive(Serialize, Deserialize)]
pub struct IssuedToken {
    pub actor: Actor,
    pub token: Zeroizing<String>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProfileKind {
    AdRead,
    Execution,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum WinRmAuthentication {
    #[default]
    Negotiate,
    Kerberos,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CredentialProfile {
    pub id: Uuid,
    pub revision: u64,
    pub label: String,
    pub username: String,
    pub kind: ProfileKind,
    pub target: String,
    pub owner_sid: String,
    pub updated_at: DateTime<Utc>,
}
impl CredentialProfile {
    pub fn is_domain_account(&self, domain: &str) -> bool {
        self.target
            .split_once('.')
            .is_some_and(|(_, suffix)| suffix.eq_ignore_ascii_case(domain))
            && self
                .username
                .rsplit_once('@')
                .is_some_and(|(user, suffix)| {
                    !user.is_empty()
                        && !user.contains(['@', '\\'])
                        && suffix.eq_ignore_ascii_case(domain)
                })
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveCredential {
    pub id: Uuid,
    pub expected: Option<u64>,
    pub label: String,
    pub username: String,
    pub kind: ProfileKind,
    pub target: String,
    pub password: Zeroizing<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConnectionProfile {
    pub id: Uuid,
    pub revision: u64,
    pub label: String,
    pub kind: ProfileKind,
    pub host: String,
    pub port: u16,
    pub credential_id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub winrm_authentication: Option<WinRmAuthentication>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveConnection {
    pub profile: ConnectionProfile,
    pub expected: Option<u64>,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "source", deny_unknown_fields)]
pub enum ConnectionAccount {
    /// Explicitly use the Windows server process identity, Kerberos only.
    ServerIdentity {},
    Password {
        username: String,
        password: Zeroizing<String>,
    },
    DomainAccount {
        credential_id: Uuid,
        expected_revision: u64,
    },
    SavedTarget {
        credential_id: Uuid,
        expected_revision: u64,
    },
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigureConnection {
    pub id: Uuid,
    pub host: String,
    #[serde(default)]
    pub locate_domain_controller: bool,
    pub kind: ProfileKind,
    pub winrm_authentication: Option<WinRmAuthentication>,
    pub account: ConnectionAccount,
}

#[derive(Serialize, Deserialize)]
pub struct ConfiguredConnection {
    pub credential: CredentialProfile,
    pub connection: ConnectionProfile,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveCollection {
    pub collection: Collection,
    pub expected: Option<u64>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeleteCollection {
    pub id: CollectionId,
    pub expected: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Job {
    pub id: Uuid,
    pub actor_id: Uuid,
    pub connection_id: Uuid,
    pub state: JobState,
    pub code: String,
    pub created_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "InventoryTask::is_connection_test")]
    pub task: InventoryTask,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discovery: Option<PreparedDiscovery>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum InventoryTask {
    #[default]
    TestConnection,
    DiscoverOu {
        base_dn: String,
    },
    ScanDevice {
        device_id: DeviceId,
    },
}
impl InventoryTask {
    fn is_connection_test(&self) -> bool {
        *self == Self::TestConnection
    }
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartInventoryJob {
    pub connection_id: Uuid,
    pub expected_revision: u64,
    pub task: InventoryTask,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum JobState {
    Queued,
    Running,
    Succeeded,
    Failed,
    Interrupted,
    Cancelled,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestConnection {
    pub connection_id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_revision: Option<u64>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuditEvent {
    pub cursor: u64,
    pub at: DateTime<Utc>,
    pub actor: String,
    pub verb: String,
    pub target: String,
    pub code: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Events {
    pub events: Vec<AuditEvent>,
    pub next: u64,
    pub resync: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ApiError {
    pub code: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Health {
    pub status: String,
    pub service_sid: String,
    pub jobs_workers: usize,
    pub jobs_capacity: usize,
}
