use chrono::{TimeZone, Utc};
use orkestr_scheduler::*;
use uuid::Uuid;
#[test]
fn windows_cover_midnight_dst_and_exclude_closing_minute() {
    let window = TimeWindow {
        timezone: "Europe/Moscow".into(),
        weekdays: vec![0],
        start_minute: 23 * 60,
        end_minute: 60,
    };
    assert!(window.valid());
    assert!(window.contains(Utc.with_ymd_and_hms(2026, 11, 23, 20, 30, 0).unwrap()));
    assert!(window.contains(Utc.with_ymd_and_hms(2026, 11, 23, 21, 30, 0).unwrap()));
    assert!(!window.contains(Utc.with_ymd_and_hms(2026, 11, 23, 22, 0, 0).unwrap()));
    let dst = TimeWindow {
        timezone: "America/New_York".into(),
        weekdays: vec![6],
        start_minute: 60,
        end_minute: 120,
    };
    assert!(dst.contains(Utc.with_ymd_and_hms(2026, 11, 1, 5, 30, 0).unwrap()));
    assert!(dst.contains(Utc.with_ymd_and_hms(2026, 11, 1, 6, 30, 0).unwrap()));
}
#[test]
fn quotas_are_shared_across_jobs_retries_and_default_sites() {
    let quotas = Quotas::default();
    let lease = Lease {
        id: Uuid::new_v4(),
        site: "default".into(),
        phase: Phase::Install,
        acquired_at: Utc::now(),
    };
    assert!(!can_claim(
        &quotas,
        std::slice::from_ref(&lease),
        "  ",
        Phase::Transfer
    ));
    assert!(can_claim(
        &quotas,
        std::slice::from_ref(&lease),
        "branch",
        Phase::Install
    ));
    let strict = Quotas {
        strict_sequential: true,
        ..quotas.clone()
    };
    assert!(!can_claim(
        &strict,
        std::slice::from_ref(&lease),
        "branch",
        Phase::Download
    ));
    let transfer = Quotas {
        transfers: 1,
        ..quotas
    };
    assert!(!can_claim(&transfer, &[lease], "branch", Phase::Install));
    let now = Utc::now();
    let id = Uuid::new_v4();
    assert!(retry_at(id, 2, now, 120) > retry_at(id, 1, now, 0));
}
