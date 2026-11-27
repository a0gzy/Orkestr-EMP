use chrono::{Duration, TimeZone, Utc};
use orkestr_domain::*;
use orkestr_storage::*;
use rusqlite::Connection;
use std::collections::BTreeSet;
use tempfile::tempdir;

fn device(host: &str) -> Device {
    Device {
        id: DeviceId::new(),
        host: host.into(),
        aliases: vec![],
        ad_guid: None,
        name: host.into(),
        domain: "example.test".into(),
        ou: "".into(),
        os: "Windows".into(),
        source: "Fixture".into(),
        site: "".into(),
        disabled: false,
        inventory_metadata: serde_json::json!({}),
    }
}
fn scan(d: &Device, ram: u64) -> DeviceScan {
    DeviceScan {
        id: ScanId::new(),
        device_id: d.id,
        scanned_at: Utc.timestamp_opt(1_800_000_000, 0).unwrap(),
        installed_ram_bytes: Some(ram),
        software_state: ProviderState::Complete,
        software: vec![],
        details: serde_json::json!({"Services":[{"Name":"fixture"}],"Warnings":[]}),
    }
}
fn group(name: &str, kind: CollectionKind) -> Collection {
    Collection {
        id: CollectionId::new(),
        parent_id: None,
        name: name.into(),
        revision: 0,
        kind,
    }
}
fn ctx() -> EvaluationContext<'static> {
    EvaluationContext {
        now: Utc.timestamp_opt(1_800_000_000, 0).unwrap(),
        max_scan_age: Duration::hours(24),
        library: &[],
    }
}

#[test]
fn empty_schema_reopens_and_rejects_foreign_newer_corrupt_files() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("orkestr.sqlite");
    let mut s = Store::open(&path).unwrap();
    assert_eq!(s.snapshot().unwrap().generation, 0);
    drop(s);
    assert!(Store::open_read_only(&path)
        .unwrap()
        .snapshot()
        .unwrap()
        .devices
        .is_empty());
    let db = Connection::open(&path).unwrap();
    db.execute_batch("PRAGMA user_version=999").unwrap();
    drop(db);
    let before = std::fs::read(&path).unwrap();
    assert!(Store::open(&path).is_err());
    assert_eq!(before, std::fs::read(&path).unwrap());
    let foreign = dir.path().join("foreign.sqlite");
    let db = Connection::open(&foreign).unwrap();
    db.execute_batch("CREATE TABLE computers(data TEXT)")
        .unwrap();
    drop(db);
    assert!(Store::open(&foreign).is_err());
    let corrupt = dir.path().join("corrupt.sqlite");
    std::fs::write(&corrupt, b"not a database").unwrap();
    assert!(Store::open(&corrupt).is_err());
    assert_eq!(std::fs::read(&corrupt).unwrap(), b"not a database");
}

#[test]
fn stable_identity_case_and_ad_rename_keep_static_members_and_details() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test.sqlite");
    let mut s = Store::open(&path).unwrap();
    let mut d = device("OLD.Example.Test");
    d.ad_guid = Some(uuid::Uuid::new_v4());
    s.upsert_device(&d).unwrap();
    let a = scan(&d, 16 * GIB);
    s.append_scan(&a).unwrap();
    let manual = s
        .save_collection(
            &group("Пилот", CollectionKind::Static(BTreeSet::from([d.id]))),
            None,
        )
        .unwrap();
    d.host = "new.example.test".into();
    s.upsert_device(&d).unwrap();
    let other = device("OLD.example.test");
    assert!(s.upsert_device(&other).is_err());
    drop(s);
    let mut s = Store::open(&path).unwrap();
    let snapshot = s.snapshot().unwrap();
    assert_eq!(snapshot.devices[0].device.id, d.id);
    assert_eq!(snapshot.devices[0].device.host, "new.example.test");
    assert_eq!(
        snapshot.devices[0].scan.as_ref().unwrap().details["Services"][0]["Name"],
        "fixture"
    );
    assert_eq!(
        snapshot
            .preview_collection(manual.id, &ctx())
            .unwrap()
            .members
            .len(),
        1
    );
    s.delete_collection(manual.id, manual.revision).unwrap();
    assert_eq!(s.snapshot().unwrap().devices.len(), 1);
}

#[test]
fn immutable_scans_packages_and_latest_scan_head() {
    let dir = tempdir().unwrap();
    let mut s = Store::open(&dir.path().join("test.sqlite")).unwrap();
    let d = device("pc");
    s.upsert_device(&d).unwrap();
    let a = scan(&d, 8 * GIB);
    s.append_scan(&a).unwrap();
    let generation = s.snapshot().unwrap().generation;
    assert!(s.append_scan(&a).is_err());
    assert_eq!(s.snapshot().unwrap().generation, generation);
    let mut older = a.clone();
    older.id = ScanId::new();
    older.scanned_at -= Duration::hours(1);
    older.installed_ram_bytes = Some(2 * GIB);
    s.append_scan(&older).unwrap();
    assert_eq!(
        s.snapshot().unwrap().devices[0].scan.as_ref().unwrap().id,
        a.id
    );
    let p = PackageRevision {
        id: PackageRevisionId::new(),
        package_id: PackageId::new(),
        name: "Fixture".into(),
        version: "1".into(),
        steps: vec![],
        artifact_hashes: vec![],
        metadata: serde_json::json!({}),
    };
    s.add_package_revision(&p).unwrap();
    let mut edit = p.clone();
    edit.name = "Changed".into();
    assert!(s.add_package_revision(&edit).is_err());
    assert_eq!(s.packages().unwrap(), vec![p]);
}

#[test]
fn two_connections_cannot_lose_collection_edits_or_make_cycles() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test.sqlite");
    let mut a = Store::open(&path).unwrap();
    let mut b = Store::open(&path).unwrap();
    let folder = a
        .save_collection(&group("Hardware", CollectionKind::Folder), None)
        .unwrap();
    let mut child = group("RAM", CollectionKind::Folder);
    child.parent_id = Some(folder.id);
    let child = a.save_collection(&child, None).unwrap();
    let mut edit = folder.clone();
    edit.name = "Оборудование".into();
    let newer = a.save_collection(&edit, Some(1)).unwrap();
    assert!(matches!(
        b.save_collection(&edit, Some(1)),
        Err(Error::Conflict)
    ));
    assert!(a.delete_collection(folder.id, newer.revision).is_err());
    edit.parent_id = Some(child.id);
    assert!(a.save_collection(&edit, Some(newer.revision)).is_err());
    edit.parent_id = None;
    edit.kind = CollectionKind::Static(BTreeSet::new());
    assert!(a.save_collection(&edit, Some(newer.revision)).is_err());
    assert_eq!(b.snapshot().unwrap().collections[0], newer);
}

#[test]
fn scan_recalculates_dynamic_not_static_and_preview_is_one_snapshot() {
    let dir = tempdir().unwrap();
    let mut s = Store::open(&dir.path().join("test.sqlite")).unwrap();
    let d = device("pc");
    s.upsert_device(&d).unwrap();
    let old = scan(&d, 8 * GIB);
    s.append_scan(&old).unwrap();
    let dynamic = s
        .save_collection(
            &group(
                ">=16",
                CollectionKind::Dynamic(Expression::Ram {
                    op: Compare::Ge,
                    bytes: 16 * GIB,
                }),
            ),
            None,
        )
        .unwrap();
    let manual = s
        .save_collection(
            &group("Pilot", CollectionKind::Static(BTreeSet::from([d.id]))),
            None,
        )
        .unwrap();
    let snapshot = s.snapshot().unwrap();
    let mut new = scan(&d, 16 * GIB);
    new.scanned_at += Duration::seconds(1);
    s.append_scan(&new).unwrap();
    let context = EvaluationContext {
        now: new.scanned_at,
        ..ctx()
    };
    assert_eq!(
        snapshot
            .preview_collection(dynamic.id, &context)
            .unwrap()
            .members
            .len(),
        0
    );
    let next = s.snapshot().unwrap();
    assert!(next.generation > snapshot.generation);
    assert_eq!(
        next.preview_collection(dynamic.id, &context)
            .unwrap()
            .members
            .len(),
        1
    );
    assert_eq!(
        next.preview_collection(manual.id, &context)
            .unwrap()
            .members
            .len(),
        1
    );
}

#[test]
fn backup_is_wal_consistent_and_refuses_overwrite() {
    let dir = tempdir().unwrap();
    let mut s = Store::open(&dir.path().join("test.sqlite")).unwrap();
    s.upsert_device(&device("pc")).unwrap();
    let path = dir.path().join("backup.sqlite");
    s.backup(&path).unwrap();
    assert!(s.backup(&path).is_err());
    assert_eq!(
        Store::open_read_only(&path)
            .unwrap()
            .snapshot()
            .unwrap()
            .devices
            .len(),
        1
    );
}
