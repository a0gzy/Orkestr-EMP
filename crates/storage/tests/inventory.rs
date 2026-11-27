use chrono::{Duration, TimeZone, Utc};
use orkestr_domain::*;
use orkestr_inventory::*;
use orkestr_storage::{Error, Store};
use rusqlite::Connection;
use serde_json::json;
use tempfile::tempdir;

fn discover(store: &mut Store, raw: &str) -> Device {
    let batch = DiscoveryBatch::manual(raw).unwrap();
    let plan = store.preview_discovery(&batch).unwrap();
    let device = plan.writes()[0].clone();
    store.apply_discovery(&plan).unwrap();
    device
}
fn report(device: DeviceId, providers: Vec<ProviderReport>) -> InventoryReport {
    let at = Utc.timestamp_opt(1_800_000_000, 0).unwrap();
    InventoryReport {
        id: uuid::Uuid::new_v4(),
        device_id: device,
        started_at: at,
        finished_at: at + Duration::seconds(10),
        providers,
    }
}
fn memory() -> ProviderReport {
    ProviderReport {
        provider: InventoryProvider::MemoryModules,
        state: ProviderState::Complete,
        failure: None,
        rows: vec![json!({"Capacity":16 * GIB})],
    }
}

#[test]
fn discovery_dry_run_backup_apply_reopen_and_noop_do_not_create_scans_or_jobs() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("core.sqlite");
    let mut store = Store::open(&path).unwrap();
    let batch = DiscoveryBatch::manual("PC\npc\nother").unwrap();
    let plan = store.preview_discovery(&batch).unwrap();
    assert!(store.snapshot().unwrap().devices.is_empty());
    let backup = dir.path().join("before.sqlite");
    store.backup(&backup).unwrap();
    assert_eq!(store.apply_discovery(&plan).unwrap(), 2);
    drop(store);
    let mut store = Store::open(&path).unwrap();
    assert!(store
        .snapshot()
        .unwrap()
        .devices
        .iter()
        .all(|r| r.scan.is_none()));
    assert_eq!(store.snapshot().unwrap().generation, 1);
    let plan = store.preview_discovery(&batch).unwrap();
    assert_eq!(store.apply_discovery(&plan).unwrap(), 0);
    assert_eq!(store.snapshot().unwrap().generation, 1);
    assert!(Store::open_read_only(&backup)
        .unwrap()
        .snapshot()
        .unwrap()
        .devices
        .is_empty());
    let db = Connection::open(&path).unwrap();
    for table in ["scans", "server_jobs", "inventory_reports"] {
        assert_eq!(
            db.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}

#[test]
fn stale_conflicting_and_wrong_database_previews_are_atomic() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("core.sqlite");
    let mut a = Store::open(&path).unwrap();
    let mut b = Store::open(&path).unwrap();
    let plan = a
        .preview_discovery(&DiscoveryBatch::manual("pc\nother").unwrap())
        .unwrap();
    discover(&mut b, "concurrent");
    assert!(matches!(a.apply_discovery(&plan), Err(Error::Conflict)));
    assert_eq!(a.snapshot().unwrap().devices.len(), 1);
    let conflict = a
        .preview_discovery(&DiscoveryBatch::csv("host,site\nnew,A\nNEW,B\nother,C").unwrap())
        .unwrap();
    let generation = a.snapshot().unwrap().generation;
    assert!(a.apply_discovery(&conflict).is_err());
    assert_eq!(a.snapshot().unwrap().generation, generation);
    assert_eq!(a.snapshot().unwrap().devices.len(), 1);
    let other_path = dir.path().join("other.sqlite");
    let mut other = Store::open(&other_path).unwrap();
    discover(&mut other, "different");
    let plan = a
        .preview_discovery(&DiscoveryBatch::csv("host,site\nconcurrent,A").unwrap())
        .unwrap();
    assert!(matches!(other.apply_discovery(&plan), Err(Error::Conflict)));
}

#[test]
fn report_scan_and_raw_f21_evidence_survive_restart_without_mutating_old_scans() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("core.sqlite");
    let mut store = Store::open(&path).unwrap();
    let device = discover(&mut store, "fixture");
    let input = report(
        device.id,
        vec![
            memory(),
            ProviderReport {
                provider: InventoryProvider::Environment,
                state: ProviderState::Complete,
                failure: None,
                rows: vec![json!({"Name":"fixture","VariableValue":"PRIVATE_SYNTHETIC_VALUE"})],
            },
        ],
    );
    let normalized = input.normalize().unwrap();
    store.record_inventory(&normalized).unwrap();
    let generation = store.snapshot().unwrap().generation;
    assert!(store.record_inventory(&normalized).is_err());
    assert_eq!(store.snapshot().unwrap().generation, generation);
    drop(store);
    let mut store = Store::open(&path).unwrap();
    let saved = store.inventory_report(input.id).unwrap().unwrap();
    assert_eq!(&saved, normalized.report());
    assert_eq!(
        store.latest_inventory_report(device.id).unwrap().unwrap(),
        saved
    );
    let snapshot = store.snapshot().unwrap();
    let scan = snapshot.devices[0].scan.as_ref().unwrap();
    assert_eq!(scan.id, ScanId(input.id));
    assert_eq!(scan.installed_ram_bytes, Some(16 * GIB));
    assert_eq!(
        scan.details["Environment"][0]["VariableValue"],
        "PRIVATE_SYNTHETIC_VALUE"
    );
    let context = EvaluationContext {
        now: input.finished_at,
        max_scan_age: Duration::hours(24),
        library: &[],
    };
    assert_eq!(
        Expression::Ram {
            op: Compare::Ge,
            bytes: 16 * GIB
        }
        .evaluate(&snapshot.devices[0], &context),
        Truth::Yes
    );
}

#[test]
fn failed_and_unknown_runs_preserve_previous_scan_and_its_timestamp() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("core.sqlite");
    let mut store = Store::open(&path).unwrap();
    let device = discover(&mut store, "fixture");
    let good = report(device.id, vec![memory()]);
    store.record_inventory(&good.normalize().unwrap()).unwrap();
    let mut failed = report(
        device.id,
        vec![ProviderReport {
            provider: InventoryProvider::OperatingSystem,
            state: ProviderState::Failed,
            failure: Some(ProviderFailure::Unavailable),
            rows: vec![],
        }],
    );
    failed.started_at += Duration::hours(1);
    failed.finished_at += Duration::hours(1);
    store
        .record_inventory(&failed.normalize().unwrap())
        .unwrap();
    let mut unknown = report(device.id, vec![]);
    unknown.started_at += Duration::hours(2);
    unknown.finished_at += Duration::hours(2);
    store
        .record_inventory(&unknown.normalize().unwrap())
        .unwrap();
    let snapshot = store.snapshot().unwrap();
    let head = snapshot.devices[0].scan.as_ref().unwrap();
    assert_eq!(head.id, ScanId(good.id));
    assert_eq!(head.scanned_at, good.started_at);
    assert_eq!(
        store
            .latest_inventory_report(device.id)
            .unwrap()
            .unwrap()
            .id,
        unknown.id
    );
    assert_eq!(
        store
            .inventory_report(failed.id)
            .unwrap()
            .unwrap()
            .normalize()
            .unwrap()
            .state(),
        InventoryState::Failed
    );
}

#[test]
fn late_reports_do_not_replace_newer_head_and_subsecond_order_is_numeric() {
    let dir = tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("core.sqlite")).unwrap();
    let device = discover(&mut store, "fixture");
    let mut older = report(device.id, vec![memory()]);
    older.started_at += Duration::nanoseconds(1);
    older.finished_at += Duration::nanoseconds(1);
    let mut newer = report(device.id, vec![memory()]);
    newer.started_at += Duration::nanoseconds(10);
    newer.finished_at += Duration::nanoseconds(10);
    store.record_inventory(&newer.normalize().unwrap()).unwrap();
    store.record_inventory(&older.normalize().unwrap()).unwrap();
    assert_eq!(
        store.snapshot().unwrap().devices[0]
            .scan
            .as_ref()
            .unwrap()
            .id,
        ScanId(newer.id)
    );
    assert_eq!(
        store
            .latest_inventory_report(device.id)
            .unwrap()
            .unwrap()
            .id,
        newer.id
    );
}

#[test]
fn report_failure_rolls_back_new_scan_head_software_rows_and_generation() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("core.sqlite");
    let mut store = Store::open(&path).unwrap();
    let device = discover(&mut store, "fixture");
    let empty = report(device.id, vec![]);
    store.record_inventory(&empty.normalize().unwrap()).unwrap();
    let generation = store.snapshot().unwrap().generation;
    let mut duplicate = empty.clone();
    duplicate.providers = vec![memory()];
    assert!(store
        .record_inventory(&duplicate.normalize().unwrap())
        .is_err());
    assert!(store.snapshot().unwrap().devices[0].scan.is_none());
    let unknown_device = report(DeviceId::new(), vec![memory()]);
    assert!(store
        .record_inventory(&unknown_device.normalize().unwrap())
        .is_err());
    assert_eq!(store.snapshot().unwrap().generation, generation);
    let db = Connection::open(&path).unwrap();
    for table in ["scans", "installed_software", "scan_heads"] {
        assert_eq!(
            db.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}

#[test]
fn v3_upgrade_preserves_generation_and_security_blobs_with_consistent_backup() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("v3.sqlite");
    let db = Connection::open(&path).unwrap();
    db.execute_batch(include_str!("../src/migrations/001_core.sql"))
        .unwrap();
    db.execute_batch(include_str!("../src/migrations/002_review.sql"))
        .unwrap();
    db.execute_batch(include_str!("../src/migrations/003_server.sql"))
        .unwrap();
    db.execute_batch("PRAGMA application_id=1330793285;PRAGMA user_version=3;UPDATE metadata SET generation=42 WHERE id=1;INSERT INTO server_identity VALUES(1,'SYNTHETIC-SID',X'010203');INSERT INTO credential_profiles VALUES('fixture',1,'{}',X'040506');").unwrap();
    drop(db);
    let mut store = Store::open(&path).unwrap();
    assert_eq!(store.snapshot().unwrap().generation, 42);
    drop(store);
    let backup = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .find(|e| e.file_name().to_string_lossy().contains("before-v5-"))
        .unwrap()
        .path();
    for (file, version) in [(&path, 10), (&backup, 3)] {
        let db = Connection::open(file).unwrap();
        assert_eq!(
            db.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            version
        );
        assert_eq!(
            db.query_row("SELECT ciphertext FROM credential_profiles", [], |r| r
                .get::<_, Vec<u8>>(0))
                .unwrap(),
            vec![4, 5, 6]
        );
        assert_eq!(
            db.query_row("SELECT canary FROM server_identity", [], |r| r
                .get::<_, Vec<u8>>(0))
                .unwrap(),
            vec![1, 2, 3]
        );
        assert_eq!(
            Store::open_read_only(file)
                .unwrap()
                .snapshot()
                .unwrap()
                .generation,
            42
        );
    }
    drop(Store::open(&path).unwrap());
    assert_eq!(
        std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains("before-v5-"))
            .count(),
        1
    );
}
