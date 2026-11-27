//! Deterministic scheduling decisions. SQLite owns durable claims.
use chrono::{DateTime, Datelike, Timelike, Utc};
use orkestr_domain::{ApplicationId, CollectionId, PackageRevisionId};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimeWindow {
    pub timezone: String,
    pub weekdays: Vec<u32>,
    pub start_minute: u32,
    pub end_minute: u32,
}
impl TimeWindow {
    pub fn valid(&self) -> bool {
        self.timezone.parse::<chrono_tz::Tz>().is_ok()
            && !self.weekdays.is_empty()
            && self.weekdays.len() <= 7
            && self.weekdays.iter().all(|day| *day < 7)
            && self.start_minute < 1440
            && self.end_minute < 1440
            && self.start_minute != self.end_minute
    }
    pub fn contains(&self, now: DateTime<Utc>) -> bool {
        let Ok(tz) = self.timezone.parse::<chrono_tz::Tz>() else {
            return false;
        };
        let local = now.with_timezone(&tz);
        let minute = local.hour() * 60 + local.minute();
        let today = local.weekday().num_days_from_monday();
        if self.start_minute < self.end_minute {
            self.weekdays.contains(&today)
                && minute >= self.start_minute
                && minute < self.end_minute
        } else if minute >= self.start_minute {
            self.weekdays.contains(&today)
        } else {
            minute < self.end_minute && self.weekdays.contains(&((today + 6) % 7))
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Quotas {
    pub global: u16,
    pub per_site: u16,
    pub downloads: u16,
    pub transfers: u16,
    pub installs: u16,
    pub strict_sequential: bool,
    pub download_bytes_per_second: u64,
    pub endpoint_bytes_per_second: u64,
}
impl Default for Quotas {
    fn default() -> Self {
        Self {
            global: 2,
            per_site: 1,
            downloads: 1,
            transfers: 2,
            installs: 2,
            strict_sequential: false,
            download_bytes_per_second: 0,
            endpoint_bytes_per_second: 0,
        }
    }
}
impl Quotas {
    pub fn valid(&self) -> bool {
        [
            self.global,
            self.per_site,
            self.downloads,
            self.transfers,
            self.installs,
        ]
        .iter()
        .all(|value| (1..=32).contains(value))
            && self.download_bytes_per_second <= 1024 * 1024 * 1024
            && self.endpoint_bytes_per_second <= 1024 * 1024 * 1024
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Phase {
    Download,
    Transfer,
    Install,
    ReadOnly,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Automation {
    pub auto_check: bool,
    pub auto_download: bool,
    pub auto_deploy: bool,
    pub unattended: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub id: Uuid,
    pub revision: u64,
    pub name: String,
    pub application_id: ApplicationId,
    pub collection_id: CollectionId,
    pub package_revision_id: PackageRevisionId,
    pub automation: Automation,
    pub interval_seconds: u64,
    pub jitter_seconds: u32,
    pub window: Option<TimeWindow>,
    pub pilot_count: u32,
    pub soak_seconds: u64,
    pub max_failure_percent: u8,
    pub max_attempts: u8,
}
impl Policy {
    pub fn valid(&self) -> bool {
        !self.name.trim().is_empty()
            && self.name.len() <= 128
            && (60..=30 * 86400).contains(&self.interval_seconds)
            && self.jitter_seconds <= 3600
            && self.pilot_count > 0
            && self.pilot_count <= 10000
            && self.soak_seconds <= 30 * 86400
            && self.max_failure_percent <= 100
            && (1..=3).contains(&self.max_attempts)
            && self.window.as_ref().is_none_or(TimeWindow::valid)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavePolicy {
    pub policy: Policy,
    pub expected: Option<u64>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Lease {
    pub id: Uuid,
    pub site: String,
    pub phase: Phase,
    pub acquired_at: DateTime<Utc>,
}
pub fn site_key(site: &str) -> String {
    let site = site.trim().to_ascii_lowercase();
    if site.is_empty() {
        "default".into()
    } else {
        site
    }
}
pub fn can_claim(quotas: &Quotas, leases: &[Lease], site: &str, phase: Phase) -> bool {
    let site = site_key(site);
    quotas.valid()
        && leases.len()
            < (if quotas.strict_sequential {
                1
            } else {
                quotas.global as usize
            })
        && leases.iter().filter(|lease| lease.site == site).count() < quotas.per_site as usize
        && leases
            .iter()
            .filter(|lease| match phase {
                Phase::Transfer | Phase::Install => {
                    matches!(lease.phase, Phase::Transfer | Phase::Install)
                }
                _ => lease.phase == phase,
            })
            .count()
            < match phase {
                Phase::Download => quotas.downloads,
                Phase::Transfer | Phase::Install => quotas.transfers.min(quotas.installs),
                Phase::ReadOnly => quotas.global,
            } as usize
}
pub fn retry_at(id: Uuid, attempt: u8, now: DateTime<Utc>, jitter_seconds: u32) -> DateTime<Utc> {
    let jitter = (id.as_u128() as u64).wrapping_add(attempt as u64) % (jitter_seconds as u64 + 1);
    now + chrono::Duration::seconds((30u64.saturating_mul(1u64 << attempt.min(10)) + jitter) as i64)
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WavePreview {
    pub pilot: Vec<orkestr_domain::DeviceId>,
    pub remaining: Vec<orkestr_domain::DeviceId>,
    pub window_open: bool,
}
pub fn waves(
    policy: &Policy,
    mut targets: Vec<orkestr_domain::DeviceId>,
    now: DateTime<Utc>,
) -> WavePreview {
    targets.sort();
    targets.dedup();
    let remaining = targets.split_off((policy.pilot_count as usize).min(targets.len()));
    WavePreview {
        pilot: targets,
        remaining,
        window_open: policy
            .window
            .as_ref()
            .is_none_or(|window| window.contains(now)),
    }
}
