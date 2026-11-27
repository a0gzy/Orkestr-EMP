use crate::*;
use chrono::{DateTime, Utc};
use orkestr_domain::{DeviceId, PackageRevisionId};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RolloutState {
    WaitingApproval,
    Pilot,
    Soaking,
    Production,
    Paused,
    Succeeded,
    Cancelled,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RolloutTarget {
    pub device_id: DeviceId,
    pub connection_id: Uuid,
    pub connection_revision: u64,
    pub deployment_id: Option<Uuid>,
    pub retry_at: Option<DateTime<Utc>>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Rollout {
    pub id: Uuid,
    pub revision: u64,
    pub actor_id: Uuid,
    pub policy: Policy,
    pub package_revision_id: PackageRevisionId,
    pub targets: Vec<RolloutTarget>,
    pub pilot_count: usize,
    pub state: RolloutState,
    pub code: String,
    pub soak_until: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanRollout {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_revision_id: Option<PackageRevisionId>,
    pub policy_id: Uuid,
    pub policy_revision: u64,
    pub generation: u64,
}
