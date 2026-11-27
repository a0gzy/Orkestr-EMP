//! Pure M1 models and collection evaluation. No network, processes or storage.
mod collections;
mod model;
mod version;

pub use collections::*;
pub use model::*;
pub use version::*;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DomainError {
    #[error("Invalid hostname")]
    InvalidHost,
    #[error("Invalid numeric version")]
    InvalidVersion,
    #[error("Invalid collection: {0}")]
    InvalidCollection(&'static str),
}
