//! Release discovery and bounded HTTP. Downloading never grants approval.
mod http;
mod source;
use chrono::{DateTime, Utc};
pub use http::*;
use orkestr_domain::ApplicationId;
use serde::{Deserialize, Serialize};
pub use source::*;
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Invalid(&'static str),
    #[error("LIBRARY_IO_FAILED")]
    Io(#[from] std::io::Error),
}
pub type Result<T> = std::result::Result<T, Error>;
pub(crate) fn invalid(code: &'static str) -> Error {
    Error::Invalid(code)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SoftwareMatch {
    pub name: String,
    pub publisher: String,
    pub product_code: Option<Uuid>,
    #[serde(default)]
    pub version_components: Option<u8>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LibraryApplication {
    pub id: ApplicationId,
    pub revision: u64,
    pub name: String,
    pub channel: String,
    pub architecture: String,
    pub source: VersionSource,
    pub download: DownloadSource,
    pub network: NetworkPolicy,
    pub verification: VerificationPolicy,
    pub software_match: SoftwareMatch,
}
impl LibraryApplication {
    pub fn normalized(&self) -> Result<Self> {
        let mut app = self.clone();
        app.name = app.name.trim().to_owned();
        app.channel = app.channel.trim().to_ascii_lowercase();
        app.architecture = match app.architecture.trim().to_ascii_lowercase().as_str() {
            "amd64" | "x86_64" | "x64" => "x64",
            "i386" | "win32" | "x86" => "x86",
            "aarch64" | "arm64" => "arm64",
            "any" | "neutral" => "neutral",
            _ => return Err(invalid("INVALID_ARCHITECTURE")),
        }
        .into();
        app.software_match.name = app.software_match.name.trim().to_owned();
        app.software_match.publisher = app.software_match.publisher.trim().to_owned();
        let host = |value: &str| {
            url::Host::parse(value.trim())
                .map(|host| host.to_string().to_ascii_lowercase())
                .map_err(|_| invalid("EXACT_HOST_REQUIRED"))
        };
        app.network.allowed_hosts = app
            .network
            .allowed_hosts
            .iter()
            .map(|h| host(h))
            .collect::<Result<_>>()?;
        app.network.allowed_hosts.sort();
        app.network.allowed_hosts.dedup();
        app.network.internal_hosts = app
            .network
            .internal_hosts
            .into_iter()
            .map(|(name, nets)| Ok((host(&name)?, nets)))
            .collect::<Result<_>>()?;
        app.validate()?;
        Ok(app)
    }
    pub fn validate(&self) -> Result<()> {
        if self
            .software_match
            .version_components
            .is_some_and(|count| !(1..=16).contains(&count))
        {
            return Err(invalid("INVALID_VERSION_PRECISION"));
        }
        for value in [
            &self.name,
            &self.channel,
            &self.architecture,
            &self.software_match.name,
            &self.software_match.publisher,
        ] {
            if value.trim().is_empty() || value.len() > 128 || value.chars().any(char::is_control) {
                return Err(invalid("INVALID_APPLICATION"));
            }
        }
        if !matches!(
            self.architecture.as_str(),
            "x86" | "x64" | "arm64" | "neutral"
        ) {
            return Err(invalid("INVALID_ARCHITECTURE"));
        }
        self.network.validate()?;
        self.source.validate(&self.network)?;
        self.download.validate(&self.network)?;
        self.verification.validate()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum VerificationPolicy {
    Hash { expected_sha256: String },
    Authenticode { publisher: String },
}
impl VerificationPolicy {
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Hash { expected_sha256 } if valid_hash(expected_sha256) => Ok(()),
            Self::Authenticode { publisher }
                if !publisher.trim().is_empty()
                    && publisher.len() <= 512
                    && !publisher.chars().any(char::is_control) =>
            {
                Ok(())
            }
            _ => Err(invalid("INVALID_VERIFICATION_POLICY")),
        }
    }
}
pub fn valid_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReleaseState {
    Candidate,
    Quarantined,
    Verified,
    Approved,
    Rejected,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReleaseCandidate {
    #[serde(default)]
    pub artifact_metadata: Option<ArtifactMetadata>,
    pub id: Uuid,
    pub application_id: ApplicationId,
    pub application_revision: u64,
    pub channel: String,
    pub architecture: String,
    pub version_raw: String,
    pub version: String,
    pub url: String,
    pub evidence: SourceEvidence,
    pub state: ReleaseState,
    pub sha256: Option<String>,
    pub bytes: Option<u64>,
    pub approved_by: Option<Uuid>,
    pub approved_at: Option<DateTime<Utc>>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ArtifactMetadata {
    pub name: String,
    pub publisher: String,
    pub version: String,
    pub product_code: Option<Uuid>,
    #[serde(default)]
    pub architecture: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SourceEvidence {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download_page_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download_page_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub download_page_redirects: Vec<String>,
    pub checked_at: DateTime<Utc>,
    pub response_sha256: String,
    pub final_url: Option<String>,
    pub redirects: Vec<String>,
    pub matches: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SourcePreview {
    pub version_raw: String,
    pub version: String,
    pub url: String,
    pub evidence: SourceEvidence,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum LibraryTask {
    Check {
        application_id: ApplicationId,
        revision: u64,
    },
    Download {
        release_id: Uuid,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LibraryJobState {
    Queued,
    Running,
    Succeeded,
    Failed,
    Interrupted,
    Cancelled,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LibraryJob {
    #[serde(default)]
    pub attempts: u8,
    #[serde(default)]
    pub retry_at: Option<DateTime<Utc>>,
    pub id: Uuid,
    pub actor_id: Uuid,
    pub task: LibraryTask,
    pub state: LibraryJobState,
    pub code: String,
    pub created_at: DateTime<Utc>,
    pub release_id: Option<Uuid>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveApplication {
    pub application: LibraryApplication,
    pub expected: Option<u64>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChangeRelease {
    pub id: Uuid,
    pub application_revision: u64,
}
