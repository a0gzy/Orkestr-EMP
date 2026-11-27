use crate::DomainError;
use std::cmp::Ordering;

/// M1 deliberately supports only numeric segments. SemVer prerelease is Unknown.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct NumericVersion(Vec<u64>);

impl std::fmt::Display for NumericVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (index, segment) in self.0.iter().enumerate() {
            if index > 0 {
                write!(f, ".")?;
            }
            write!(f, "{segment}")?;
        }
        Ok(())
    }
}

impl NumericVersion {
    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        let raw = raw.trim().strip_prefix('v').unwrap_or(raw.trim());
        if raw.len() > 128 {
            return Err(DomainError::InvalidVersion);
        }
        let mut segments = raw
            .split('.')
            .map(|s| {
                if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
                    return Err(DomainError::InvalidVersion);
                }
                s.parse().map_err(|_| DomainError::InvalidVersion)
            })
            .collect::<Result<Vec<u64>, _>>()?;
        if segments.len() > 16 {
            return Err(DomainError::InvalidVersion);
        }
        while segments.len() > 1 && segments.last() == Some(&0) {
            segments.pop();
        }
        Ok(Self(segments))
    }

    pub fn compare(a: &str, b: &str) -> Option<Ordering> {
        Some(Self::parse(a).ok()?.cmp(&Self::parse(b).ok()?))
    }
}
