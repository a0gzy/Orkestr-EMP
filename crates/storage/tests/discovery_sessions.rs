use chrono::{Duration, Utc};
use orkestr_domain::*;
use orkestr_inventory::{
    DiscoveryBatch, InventoryProvider, InventoryReport, ProviderFailure, ProviderReport,
};
use orkestr_protocol::*;
use orkestr_storage::{Error, Store};
use serde_json::json;
use uuid::Uuid;

fn input(text: &str) -> PrepareDiscovery {
    PrepareDiscovery {
        format: DiscoverySource::Manual,
        text: text.into(),
    }
}

#[test]
fn apply_receipt_and_backup_survive_restart_without_repeating_writes_or_audit() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db.sqlite");
    let owner = Uuid::new_v4();
    let mut store = Store::open(&path).unwrap();
    let p = store
        .prepare_discovery_session(owner, &input("pc-01\nPC-01\npc-02"))
        .unwrap();
    assert!(store.snapshot().unwrap().devices.is_empty());
    let request = ApplyDiscovery {
        preview_id: p.id,
        confirmed: true,
    };
    let first = store.apply_discovery_session(owner, &request).unwrap();
    assert_eq!(first.changes, 2);
    assert_eq!(first.generation, 1);
    let backup = dir.path().join(first.backup.as_ref().unwrap());
    assert!(Store::open_read_only(&backup)
        .unwrap()
        .snapshot()
        .unwrap()
        .devices
        .is_empty());
    assert_eq!(
        store.apply_discovery_session(owner, &request).unwrap(),
        first
    );
    drop(store);
    let mut store = Store::open(&path).unwrap();
    assert_eq!(
        store.apply_discovery_session(owner, &request).unwrap(),
        first
    );
    assert_eq!(store.snapshot().unwrap().generation, 1);
    assert!(store
        .snapshot()
        .unwrap()
        .devices
        .iter()
        .all(|r| r.scan.is_none()));
    assert!(matches!(
        store.apply_discovery_session(Uuid::new_v4(), &request),
        Err(Error::PreviewUnavailable)
    ));
    let events = store.events(0).unwrap();
    assert_eq!(
        events
            .events
            .iter()
            .filter(|e| e.verb == "devices.discover")
            .count(),
        1
    );
    let p = store
        .prepare_discovery_session(owner, &input("pc-01\npc-02"))
        .unwrap();
    let noop = store
        .apply_discovery_session(
            owner,
            &ApplyDiscovery {
                preview_id: p.id,
                confirmed: true,
            },
        )
        .unwrap();
    assert_eq!(noop.changes, 0);
    assert!(noop.backup.is_none());
    assert_eq!(noop.generation, 1);
    assert_eq!(
        std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e
                .file_name()
                .to_string_lossy()
                .starts_with("before-discovery-"))
            .count(),
        1
    );
}

#[test]
fn owner_confirmation_stale_draft_replacement_and_restart_fail_closed() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db.sqlite");
    let owner = Uuid::new_v4();
    let mut store = Store::open(&path).unwrap();
    let p = store
        .prepare_discovery_session(owner, &input("pc"))
        .unwrap();
    assert!(store
        .apply_discovery_session(
            owner,
            &ApplyDiscovery {
                preview_id: p.id,
                confirmed: false
            }
        )
        .is_err());
    assert!(matches!(
        store.apply_discovery_session(
            Uuid::new_v4(),
            &ApplyDiscovery {
                preview_id: p.id,
                confirmed: true
            }
        ),
        Err(Error::PreviewUnavailable)
    ));
    let changed = DiscoveryBatch::manual("concurrent").unwrap();
    let plan = store.preview_discovery(&changed).unwrap();
    store.apply_discovery(&plan).unwrap();
    assert!(matches!(
        store.apply_discovery_session(
            owner,
            &ApplyDiscovery {
                preview_id: p.id,
                confirmed: true
            }
        ),
        Err(Error::Conflict)
    ));
    let newer = store
        .prepare_discovery_session(owner, &input("other"))
        .unwrap();
    assert!(matches!(
        store.apply_discovery_session(
            owner,
            &ApplyDiscovery {
                preview_id: p.id,
                confirmed: true
            }
        ),
        Err(Error::PreviewUnavailable)
    ));
    assert!(store
        .prepare_discovery_session(owner, &input("https://SYNTHETIC_SECRET"))
        .is_err());
    assert!(matches!(
        store.apply_discovery_session(
            owner,
            &ApplyDiscovery {
                preview_id: newer.id,
                confirmed: true
            }
        ),
        Err(Error::PreviewUnavailable)
    ));
    let p = store
        .prepare_discovery_session(owner, &input("pc"))
        .unwrap();
    drop(store);
    let mut store = Store::open(&path).unwrap();
    assert!(matches!(
        store.apply_discovery_session(
            owner,
            &ApplyDiscovery {
                preview_id: p.id,
                confirmed: true
            }
        ),
        Err(Error::PreviewUnavailable)
    ));
    assert_eq!(store.snapshot().unwrap().devices.len(), 1);
}

#[test]
fn conflicting_input_blocks_apply_and_existing_backup_is_never_overwritten() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("db.sqlite")).unwrap();
    let owner = Uuid::new_v4();
    let p = store
        .prepare_discovery_session(
            owner,
            &PrepareDiscovery {
                format: DiscoverySource::Csv,
                text: "host,site\npc,A\nPC,B\nother,C".into(),
            },
        )
        .unwrap();
    assert!(!p.can_apply);
    assert!(store
        .apply_discovery_session(
            owner,
            &ApplyDiscovery {
                preview_id: p.id,
                confirmed: true
            }
        )
        .is_err());
    assert!(store.snapshot().unwrap().devices.is_empty());
    let p = store
        .prepare_discovery_session(owner, &input("pc"))
        .unwrap();
    let protected = dir.path().join(format!("before-discovery-{}.sqlite", p.id));
    std::fs::write(&protected, b"SYNTHETIC_EXISTING_FILE").unwrap();
    let result = store
        .apply_discovery_session(
            owner,
            &ApplyDiscovery {
                preview_id: p.id,
                confirmed: true,
            },
        )
        .unwrap();
    assert_ne!(
        result.backup.as_deref(),
        protected.file_name().unwrap().to_str()
    );
    assert_eq!(
        std::fs::read(&protected).unwrap(),
        b"SYNTHETIC_EXISTING_FILE"
    );
}

#[test]
fn inventory_status_is_safe_and_separates_latest_failure_from_old_head() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("db.sqlite")).unwrap();
    let plan = store
        .preview_discovery(&DiscoveryBatch::manual("pc").unwrap())
        .unwrap();
    let device = plan.writes()[0].clone();
    store.apply_discovery(&plan).unwrap();
    assert!(store
        .device_inventory_status(device.id)
        .unwrap()
        .latest
        .is_none());
    let at = Utc::now();
    let good = InventoryReport {
        id: Uuid::new_v4(),
        device_id: device.id,
        started_at: at,
        finished_at: at + Duration::seconds(1),
        providers: vec![ProviderReport {
            provider: InventoryProvider::Environment,
            state: ProviderState::Complete,
            failure: None,
            rows: vec![json!({"VariableValue":"SYNTHETIC_PRIVATE_EVIDENCE"})],
        }],
    };
    store.record_inventory(&good.normalize().unwrap()).unwrap();
    let failed = InventoryReport {
        id: Uuid::new_v4(),
        device_id: device.id,
        started_at: at + Duration::minutes(1),
        finished_at: at + Duration::minutes(2),
        providers: vec![ProviderReport {
            provider: InventoryProvider::OperatingSystem,
            state: ProviderState::Failed,
            failure: Some(ProviderFailure::AccessDenied),
            rows: vec![],
        }],
    };
    store
        .record_inventory(&failed.normalize().unwrap())
        .unwrap();
    let status = store.device_inventory_status(device.id).unwrap();
    assert_eq!(status.latest.as_ref().unwrap().id, failed.id);
    assert_eq!(
        status.latest.as_ref().unwrap().state,
        InventoryState::Failed
    );
    assert_eq!(status.head.as_ref().unwrap().id, good.id);
    assert_eq!(status.head.as_ref().unwrap().state, InventoryState::Partial);
    assert!(!serde_json::to_string(&status)
        .unwrap()
        .contains("SYNTHETIC_PRIVATE_EVIDENCE"));
    assert!(store.device_inventory_status(DeviceId::new()).is_err());
}

#[test]
fn v4_upgrade_preserves_inventory_reports_and_makes_a_v4_backup() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v4.sqlite");
    let mut fixture = Store::open(&path).unwrap();
    let plan = fixture
        .preview_discovery(&DiscoveryBatch::manual("preserved").unwrap())
        .unwrap();
    let device = plan.writes()[0].clone();
    fixture.apply_discovery(&plan).unwrap();
    let report = InventoryReport {
        id: Uuid::new_v4(),
        device_id: device.id,
        started_at: Utc::now(),
        finished_at: Utc::now(),
        providers: vec![],
    };
    fixture
        .record_inventory(&report.normalize().unwrap())
        .unwrap();
    drop(fixture);
    // Remove tables added after v4 from this synthetic fixture.
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch(include_str!("common/drop_pipeline.sql"))
        .unwrap();
    db.execute_batch("DROP TABLE deployments;DROP TABLE managed_installers;DROP TABLE discovery_receipts;PRAGMA user_version=4;UPDATE metadata SET generation=24 WHERE id=1;").unwrap();
    drop(db);
    let mut store = Store::open(&path).unwrap();
    assert_eq!(store.snapshot().unwrap().generation, 24);
    drop(store);
    let backup = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .find(|e| e.file_name().to_string_lossy().contains("before-v5-"))
        .unwrap()
        .path();
    let db = rusqlite::Connection::open(&backup).unwrap();
    assert_eq!(
        db.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        4
    );
    assert_eq!(
        Store::open_read_only(&backup)
            .unwrap()
            .snapshot()
            .unwrap()
            .generation,
        24
    );
    for file in [&path, &backup] {
        let store = Store::open_read_only(file).unwrap();
        assert_eq!(
            store
                .inventory_report(report.id)
                .unwrap()
                .unwrap()
                .device_id,
            device.id
        );
    }
}

#[test]
fn receipt_failure_rolls_back_devices_generation_and_audit_and_retry_is_safe() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db.sqlite");
    let mut store = Store::open(&path).unwrap();
    let owner = Uuid::new_v4();
    let p = store
        .prepare_discovery_session(owner, &input("pc\nother"))
        .unwrap();
    let request = ApplyDiscovery {
        preview_id: p.id,
        confirmed: true,
    };
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TRIGGER reject_receipt BEFORE INSERT ON discovery_receipts BEGIN SELECT RAISE(ABORT,'SYNTHETIC_PRIVATE_FAILURE'); END;").unwrap();
    let error = store.apply_discovery_session(owner, &request).unwrap_err();
    assert!(!error.to_string().contains("SYNTHETIC_PRIVATE_FAILURE"));
    assert_eq!(store.snapshot().unwrap().generation, 0);
    assert!(store.snapshot().unwrap().devices.is_empty());
    assert!(store.events(0).unwrap().events.is_empty());
    db.execute_batch("DROP TRIGGER reject_receipt;").unwrap();
    drop(db);
    let receipt = store.apply_discovery_session(owner, &request).unwrap();
    assert_eq!(receipt.changes, 2);
    assert_eq!(
        store.apply_discovery_session(owner, &request).unwrap(),
        receipt
    );
}
