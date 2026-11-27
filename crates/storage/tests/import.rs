use orkestr_domain::*;
use orkestr_storage::*;
use rusqlite::Connection;
use serde_json::{json, Value};
use std::path::Path;
use tempfile::tempdir;

fn legacy(host: &str, guid: &str) -> Value {
    json!({"host":host,"name":host,"domain":"example.test","guid":guid,"os":"Windows 11","source":"Fixture",
        "last_scan":"2027-01-15T08:00:00Z","details":json!({"MemoryGB":7.89,"MemoryModules":[{"Capacity":8589934592u64}],
        "Software":[{"Name":"Google Chrome","Publisher":"Google","Version":"2.9"}],"Services":[{"Name":"fixture"}],"Warnings":[]}).to_string()})
}
fn inventory(path: &Path, rows: &[Value], collections: &[Value]) {
    let db = Connection::open(path).unwrap();
    db.execute_batch("CREATE TABLE computers(host TEXT PRIMARY KEY COLLATE NOCASE,guid TEXT NOT NULL,data TEXT NOT NULL);CREATE TABLE collections(id TEXT PRIMARY KEY,data TEXT NOT NULL);").unwrap();
    for row in rows {
        db.execute(
            "INSERT INTO computers VALUES(?1,?2,?3)",
            rusqlite::params![
                row["host"].as_str().unwrap(),
                row["guid"].as_str().unwrap(),
                row.to_string()
            ],
        )
        .unwrap();
    }
    for row in collections {
        db.execute(
            "INSERT INTO collections VALUES(?1,?2)",
            rusqlite::params![row["id"].as_str().unwrap(), row.to_string()],
        )
        .unwrap();
    }
}
fn deploy(path: &Path, bad: bool) {
    std::fs::write(path,json!({"packages":[{"id":7,"name":"Fixture only","version":"1","steps":[{"kind":if bad {"Unsupported"}else{"PowerShell"},"command":"NEVER EXECUTED","file":"never-read.msi"}]}],
        "lists":[],"schedules":[{"enabled":true}],"deployments":[{"output":"SYNTHETIC-SECRET-MUST-NOT-BE-REPORTED"}],"settings":{"token":"SYNTHETIC-TOKEN"}}).to_string()).unwrap();
}

#[test]
fn inventory_dry_run_preserves_copy_and_import_survives_restart_idempotently() {
    let dir = tempdir().unwrap();
    let copy = dir.path().join("copy.sqlite");
    let target = dir.path().join("orkestr.sqlite");
    inventory(
        &copy,
        &[legacy("PC01.example.test", "")],
        &[
            json!({"id":"static","name":"Пилот","dynamic":false,"hosts":["pc01.EXAMPLE.test"]}),
            json!({"id":"dynamic","name":"Windows","dynamic":true,"match_any":false,"rules":[{"field":"OS","operator":"contains","value":"WINDOWS"}]}),
        ],
    );
    let before = std::fs::read(&copy).unwrap();
    let mut s = Store::open(&target).unwrap();
    let plan = s.preview_inventory_copy(&copy).unwrap();
    assert!(plan.report().can_apply());
    assert_eq!(plan.report().devices, 1);
    assert_eq!(plan.report().collections, 2);
    assert_eq!(s.snapshot().unwrap().generation, 0);
    assert_eq!(std::fs::read(&copy).unwrap(), before);
    assert_eq!(s.apply_import(&plan).unwrap(), ImportOutcome::Applied);
    drop(s);
    let mut s = Store::open(&target).unwrap();
    assert_eq!(
        s.apply_import(&plan).unwrap(),
        ImportOutcome::AlreadyImported
    );
    let snap = s.snapshot().unwrap();
    assert_eq!(snap.devices.len(), 1);
    assert_eq!(snap.collections.len(), 2);
    let scan = snap.devices[0].scan.as_ref().unwrap();
    assert_eq!(scan.installed_ram_bytes, Some(8 * GIB));
    assert_eq!(scan.software_state, ProviderState::Partial);
    assert_eq!(scan.details["Services"][0]["Name"], "fixture");
    assert!(!imported_identity_map(&s, &plan.report().source_sha256)
        .unwrap()
        .is_empty());
    assert_eq!(std::fs::read(&copy).unwrap(), before);
    assert!(!copy.with_extension("sqlite-wal").exists());
    assert!(!copy.with_extension("sqlite-shm").exists());
}

#[test]
fn rounded_ram_is_unknown_and_legacy_rules_are_quarantined_without_semantics_change() {
    let dir = tempdir().unwrap();
    let copy = dir.path().join("copy.sqlite");
    let mut row = legacy("pc", "");
    row["details"] = json!({"MemoryGB":8,"Software":[]}).to_string().into();
    inventory(
        &copy,
        &[row],
        &[
            json!({"id":"ram","name":"RAM","dynamic":true,"rules":[{"field":"MemoryGB","operator":"greater than","value":"8"}]}),
        ],
    );
    let mut s = Store::open(&dir.path().join("target.sqlite")).unwrap();
    let p = s.preview_inventory_copy(&copy).unwrap();
    assert!(p.report().can_apply());
    assert_eq!(p.report().review_items, 1);
    s.apply_import(&p).unwrap();
    assert_eq!(s.snapshot().unwrap().devices.len(), 1);
    assert!(s.snapshot().unwrap().collections.is_empty());
    let review = s.import_reviews().unwrap();
    assert!(!review[0].enabled);
    assert_eq!(review[0].data["rules"][0]["field"], "MemoryGB");
}

#[test]
fn conflicting_guids_and_unresolved_members_block_whole_import() {
    let dir = tempdir().unwrap();
    let copy = dir.path().join("copy.sqlite");
    let guid = uuid::Uuid::new_v4().to_string();
    inventory(
        &copy,
        &[legacy("pc01", &guid), legacy("pc02", &guid)],
        &[json!({"id":"static","name":"Unresolved","hosts":["missing"]})],
    );
    let mut s = Store::open(&dir.path().join("target.sqlite")).unwrap();
    let p = s.preview_inventory_copy(&copy).unwrap();
    assert!(p.report().errors.len() >= 2);
    assert!(s.apply_import(&p).is_err());
    assert_eq!(s.snapshot().unwrap().generation, 0);
}

#[test]
fn source_changes_and_target_changes_invalidate_plan() {
    let dir = tempdir().unwrap();
    let copy = dir.path().join("copy.json");
    deploy(&copy, false);
    let mut s = Store::open(&dir.path().join("target.sqlite")).unwrap();
    let p = s.preview_deploy_copy(&copy).unwrap();
    deploy(&copy, true);
    assert!(s.apply_import(&p).is_err());
    deploy(&copy, false);
    let c = Collection {
        id: CollectionId::new(),
        parent_id: None,
        name: "Concurrent edit".into(),
        revision: 0,
        kind: CollectionKind::Folder,
    };
    s.save_collection(&c, None).unwrap();
    assert!(matches!(s.apply_import(&p), Err(Error::Conflict)));
    assert!(s.packages().unwrap().is_empty());
}

#[test]
fn deploy_only_imports_unapproved_definitions_and_never_reports_history_secrets() {
    let dir = tempdir().unwrap();
    let copy = dir.path().join("copy.json");
    deploy(&copy, false);
    let before = std::fs::read(&copy).unwrap();
    let mut s = Store::open(&dir.path().join("target.sqlite")).unwrap();
    let p = s.preview_deploy_copy(&copy).unwrap();
    assert!(p.report().can_apply());
    assert_eq!(p.report().excluded_history, 1);
    assert_eq!(p.report().excluded_schedules, 1);
    let report = serde_json::to_string(p.report()).unwrap();
    assert!(!report.contains("SYNTHETIC-SECRET"));
    assert!(!report.contains("SYNTHETIC-TOKEN"));
    s.apply_import(&p).unwrap();
    let packages = s.packages().unwrap();
    assert_eq!(packages.len(), 1);
    assert!(packages[0].artifact_hashes.is_empty());
    assert_eq!(packages[0].steps[0].definition["command"], "NEVER EXECUTED");
    assert_eq!(std::fs::read(&copy).unwrap(), before);
    deploy(&copy, true);
    let p = s.preview_deploy_copy(&copy).unwrap();
    assert!(!p.report().can_apply());
    assert!(s.apply_import(&p).is_err());
}

#[test]
fn failure_mid_import_rolls_back_ledger_and_prior_records() {
    let dir = tempdir().unwrap();
    let copy = dir.path().join("copy.json");
    deploy(&copy, false);
    let target = dir.path().join("target.sqlite");
    let mut s = Store::open(&target).unwrap();
    let p = s.preview_deploy_copy(&copy).unwrap();
    let db = Connection::open(&target).unwrap();
    db.execute_batch("CREATE TRIGGER fixture_fail BEFORE INSERT ON legacy_ids BEGIN SELECT RAISE(ABORT,'fixture failure'); END;").unwrap();
    assert!(s.apply_import(&p).is_err());
    assert!(s.packages().unwrap().is_empty());
    assert_eq!(s.snapshot().unwrap().generation, 0);
    let count: i64 = db
        .query_row("SELECT count(*) FROM imports", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0);
    db.execute_batch("DROP TRIGGER fixture_fail").unwrap();
    assert_eq!(s.apply_import(&p).unwrap(), ImportOutcome::Applied);
}

#[test]
fn live_wal_and_invalid_formats_are_rejected_without_writing_source() {
    let dir = tempdir().unwrap();
    let copy = dir.path().join("live.sqlite");
    inventory(&copy, &[], &[]);
    let source = Connection::open(&copy).unwrap();
    source
        .execute_batch("PRAGMA journal_mode=WAL; INSERT INTO collections VALUES('fixture','{}');")
        .unwrap();
    let mut s = Store::open(&dir.path().join("target.sqlite")).unwrap();
    assert!(s.preview_inventory_copy(&copy).is_err());
    drop(source);
    let json = dir.path().join("bad.json");
    std::fs::write(&json, "{broken").unwrap();
    assert!(s.preview_deploy_copy(&json).is_err());
    assert_eq!(std::fs::read_to_string(&json).unwrap(), "{broken");
}

#[test]
fn no_module_capacities_does_not_convert_rounded_memory_to_exact_bytes() {
    let dir = tempdir().unwrap();
    let copy = dir.path().join("copy.sqlite");
    let mut row = legacy("pc", "");
    row["details"] = json!({"MemoryGB":16,"Software":[],"Warnings":[]})
        .to_string()
        .into();
    inventory(&copy, &[row], &[]);
    let mut s = Store::open(&dir.path().join("target.sqlite")).unwrap();
    let plan = s.preview_inventory_copy(&copy).unwrap();
    assert!(plan.report().can_apply());
    assert!(plan
        .report()
        .warnings
        .contains("RAM_EXACT_PHYSICAL_BYTES_UNAVAILABLE"));
    s.apply_import(&plan).unwrap();
    assert_eq!(
        s.snapshot().unwrap().devices[0]
            .scan
            .as_ref()
            .unwrap()
            .installed_ram_bytes,
        None
    );
}

#[test]
fn importing_ad_rename_preserves_device_id_static_members_and_directory_metadata() {
    let dir = tempdir().unwrap();
    let first = dir.path().join("first.sqlite");
    let second = dir.path().join("second.sqlite");
    let guid = uuid::Uuid::new_v4().to_string();
    inventory(
        &first,
        &[legacy("old.example.test", &guid)],
        &[json!({"id":"pilot","name":"Pilot","hosts":["OLD.example.test"]})],
    );
    let mut row = legacy("new.example.test", &guid);
    row["details"] = "".into();
    row["ad_details"] = "fixture-directory-details".into();
    inventory(&second, &[row], &[]);
    let mut s = Store::open(&dir.path().join("target.sqlite")).unwrap();
    let plan = s.preview_inventory_copy(&first).unwrap();
    s.apply_import(&plan).unwrap();
    let old = s.snapshot().unwrap();
    let id = old.devices[0].device.id;
    let plan = s.preview_inventory_copy(&second).unwrap();
    assert_eq!(plan.report().updated_devices, 1);
    s.apply_import(&plan).unwrap();
    let new = s.snapshot().unwrap();
    assert_eq!(new.devices.len(), 1);
    assert_eq!(new.devices[0].device.id, id);
    assert_eq!(new.devices[0].device.host, "new.example.test");
    assert_eq!(
        new.devices[0].device.inventory_metadata["ad_details"],
        "fixture-directory-details"
    );
    assert!(new.devices[0].scan.is_some());
    assert_eq!(old.collections, new.collections);
}
