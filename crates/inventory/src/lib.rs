//! Offline M3 inventory core. No network, process execution, credentials or auto-scan.
mod discovery;
mod scan;

pub use discovery::*;
pub use scan::*;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    // Never include source rows or adapter exception messages in errors.
    #[error("Invalid inventory input: {0}")]
    Invalid(&'static str),
}
pub type Result<T> = std::result::Result<T, Error>;
