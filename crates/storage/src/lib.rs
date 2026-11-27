//! Local repositories, owned by the backend/service, never accessed over SMB.
mod deployment;
mod discovery;
mod import;
mod inventory;
mod library;
mod review;
mod rollout;
mod scheduling;
mod server;
mod store;
mod workflow;

pub use import::*;
pub use review::*;
pub use store::*;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    // Deliberately no SQLite/JSON error text: may contain hostile imported data/secrets.
    #[error("SQLite operation failed; source data has not been replaced")]
    Sql(#[from] rusqlite::Error),
    #[error("Invalid stored/source JSON")]
    Json(#[from] serde_json::Error),
    #[error("File operation failed")]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Domain(#[from] orkestr_domain::DomainError),
    #[error(transparent)]
    Inventory(#[from] orkestr_inventory::Error),
    #[error("Edit conflict: reload the current revision before saving")]
    Conflict,
    #[error("Discovery preview unavailable or expired; prepare a new preview")]
    PreviewUnavailable,
    #[error("Discovery preview capacity reached; retry later")]
    DiscoveryBusy,
    #[error("A deployment already holds this device")]
    DeploymentDeviceBusy,
    #[error("Unsupported database schema version")]
    Schema,
    #[error("Invalid data: {0}")]
    Invalid(&'static str),
}
pub type Result<T> = std::result::Result<T, Error>;
