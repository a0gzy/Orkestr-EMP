use orkestr_storage::*;
use rusqlite::Connection;
use serde_json::json;
use tempfile::tempdir;

#[test]
fn v7_upgrade_preserves_msi_and_exe_execution_is_durable_without_rewriting_revisions() {
    use orkestr_protocol::{InstallerExecution, ManagedInstaller};
    let dir = tempdir().unwrap();
    let path = dir.path().join("v7.sqlite");
    let actor = uuid::Uuid::new_v4();
    let legacy = json!({"name":"Legacy MSI", "version":"1", "sha256":"a".repeat(64), "product_code":uuid::Uuid::new_v4()});
    let installer: ManagedInstaller = serde_json::from_value(legacy.clone()).unwrap();
    let mut store = Store::open(&path).unwrap();
    let msi = store.register_installer(actor, &installer).unwrap();
    drop(store);
    let db = Connection::open(&path).unwrap();
    db.execute_batch(include_str!("common/drop_pipeline.sql"))
        .unwrap();
    db.execute_batch("PRAGMA user_version=7;").unwrap();
    drop(db);
    let mut store = Store::open(&path).unwrap();
    assert_eq!(
        serde_json::to_value(store.managed_installer(msi.id).unwrap().unwrap()).unwrap(),
        legacy
    );
    let backup = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(std::result::Result::ok)
        .find(|entry| entry.file_name().to_string_lossy().contains("before-v8-"))
        .unwrap()
        .path();
    assert_eq!(
        Connection::open(&backup)
            .unwrap()
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        7
    );
    assert!(Store::open_read_only(&backup)
        .unwrap()
        .managed_installer(msi.id)
        .unwrap()
        .unwrap()
        .execution
        .is_msi());
    let exe_installer = ManagedInstaller {
        name: "EXE fixture".into(),
        execution: InstallerExecution::Exe {
            arguments: "/S".into(),
        },
        ..installer
    };
    let exe = store.register_installer(actor, &exe_installer).unwrap();
    drop(store);
    let store = Store::open(&path).unwrap();
    assert_eq!(
        store.managed_installer(exe.id).unwrap().unwrap(),
        exe_installer
    );
    assert!(store
        .managed_installer(msi.id)
        .unwrap()
        .unwrap()
        .execution
        .is_msi());
    assert_eq!(
        Connection::open(&path)
            .unwrap()
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        10
    );
}

#[test]
fn v5_upgrade_backups_preserve_legacy_winrm_profiles() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("v5.sqlite");
    drop(Store::open(&path).unwrap());
    let db = Connection::open(&path).unwrap();
    db.execute_batch(include_str!("common/drop_pipeline.sql"))
        .unwrap();
    db.execute_batch("DROP TABLE deployments;DROP TABLE managed_installers;PRAGMA user_version=5;")
        .unwrap();
    let id = uuid::Uuid::new_v4();
    let legacy = json!({
        "id":id,"revision":4,"label":"Workgroup Windows PC","kind":"Execution",
        "host":"test.invalid","port":5986,"credential_id":uuid::Uuid::new_v4()
    });
    db.execute(
        "INSERT INTO connection_profiles VALUES(?1,4,?2)",
        [id.to_string(), legacy.to_string()],
    )
    .unwrap();
    drop(db);

    drop(Store::open(&path).unwrap());
    let backup = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(std::result::Result::ok)
        .find(|e| e.file_name().to_string_lossy().contains("before-v6-"))
        .unwrap()
        .path();
    for (file, version) in [(&path, 10), (&backup, 5)] {
        let profiles = Store::open_read_only(file).unwrap().connections().unwrap();
        assert_eq!(serde_json::to_value(&profiles[0]).unwrap(), legacy);
        assert_eq!(
            Connection::open(file)
                .unwrap()
                .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            version
        );
    }
}

#[test]
fn v1_upgrade_is_additive_and_leaves_a_readable_consistent_backup() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("old.sqlite");
    let db = Connection::open(&path).unwrap();
    db.execute_batch(include_str!("../src/migrations/001_core.sql"))
        .unwrap();
    db.execute_batch("PRAGMA application_id=1330793285;PRAGMA user_version=1;")
        .unwrap();
    drop(db);
    assert!(Store::open_read_only(&path)
        .unwrap()
        .import_reviews()
        .unwrap()
        .is_empty());
    let mut upgraded = Store::open(&path).unwrap();
    assert_eq!(upgraded.snapshot().unwrap().generation, 0);
    drop(upgraded);
    let backups: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().contains("before-v5-"))
        .collect();
    assert_eq!(backups.len(), 1);
    let backup = Connection::open(backups[0].path()).unwrap();
    let version: i64 = backup
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, 1);
    assert!(Store::open_read_only(&backups[0].path())
        .unwrap()
        .packages()
        .unwrap()
        .is_empty());
    let mut again = Store::open(&path).unwrap();
    assert_eq!(again.snapshot().unwrap().generation, 0);
}

#[test]
fn v2_upgrade_preserves_registry_and_creates_v2_backup_before_server_tables() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("v2.sqlite");
    let db = Connection::open(&path).unwrap();
    db.execute_batch(include_str!("../src/migrations/001_core.sql"))
        .unwrap();
    db.execute_batch(include_str!("../src/migrations/002_review.sql"))
        .unwrap();
    db.execute_batch("PRAGMA application_id=1330793285;PRAGMA user_version=2;UPDATE metadata SET generation=42 WHERE id=1;").unwrap();
    drop(db);
    let mut store = Store::open(&path).unwrap();
    assert_eq!(store.snapshot().unwrap().generation, 42);
    assert!(store.credentials().unwrap().is_empty());
    drop(store);
    let backup = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .find(|e| e.file_name().to_string_lossy().contains("before-v5-"))
        .unwrap();
    let db = Connection::open(backup.path()).unwrap();
    let version: i64 = db
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, 2);
    assert_eq!(
        Store::open_read_only(&backup.path())
            .unwrap()
            .snapshot()
            .unwrap()
            .generation,
        42
    );
}
#[test]
fn explicit_origin_keeps_package_identity_across_different_backups_not_servers() {
    let dir = tempdir().unwrap();
    let a = dir.path().join("a.json");
    let b = dir.path().join("b.json");
    std::fs::write(&a,json!({"packages":[{"id":7,"name":"App","version":"1","steps":[],"description":"Keep description","folder":"Fixtures"}]}).to_string()).unwrap();
    std::fs::write(
        &b,
        json!({"packages":[{"id":7,"name":"App","version":"2","steps":[]}]}).to_string(),
    )
    .unwrap();
    let mut s = Store::open(&dir.path().join("db.sqlite")).unwrap();
    let first = s
        .preview_deploy_copy_with_origin(&a, Some("server-a"))
        .unwrap();
    s.apply_import(&first).unwrap();
    let second = s
        .preview_deploy_copy_with_origin(&b, Some("server-a"))
        .unwrap();
    s.apply_import(&second).unwrap();
    let packages = s.packages().unwrap();
    assert_eq!(packages.len(), 2);
    assert_eq!(packages[0].package_id, packages[1].package_id);
    assert_ne!(packages[0].id, packages[1].id);
    assert_eq!(packages[0].metadata["description"], "Keep description");
    let c = dir.path().join("c.json");
    std::fs::write(
        &c,
        json!({"packages":[{"id":7,"name":"Unrelated","version":"3","steps":[]}]}).to_string(),
    )
    .unwrap();
    let third = s
        .preview_deploy_copy_with_origin(&c, Some("server-b"))
        .unwrap();
    s.apply_import(&third).unwrap();
    assert_ne!(s.packages().unwrap()[2].package_id, packages[0].package_id);
}
#[test]
fn review_preserves_recurrence_and_targets_but_no_outputs_tokens_or_execution() {
    let dir = tempdir().unwrap();
    let copy = dir.path().join("copy.json");
    std::fs::write(&copy,json!({"packages":[],"settings":{"token":"SYNTHETIC_TOKEN"},
        "lists":[{"name":"Pilot","computers":"pc1;pc2"}],
        "schedules":[{"id":1,"name":"Weekly","targets":"pc1","next":123,"recurrence":"weekly","enabled":true,"password":"SYNTHETIC_PASSWORD"}],
        "deployments":[{"id":9,"package_name":"Legacy history","created":100,"finished":true,"targets":[{"computer":"pc1","status":"Successful","output":"SYNTHETIC_OUTPUT_SECRET","elapsed":2}]}]}).to_string()).unwrap();
    let mut s = Store::open(&dir.path().join("db.sqlite")).unwrap();
    let p = s.preview_deploy_copy(&copy).unwrap();
    assert_eq!(p.report().review_items, 3);
    s.apply_import(&p).unwrap();
    let reviews = s.import_reviews().unwrap();
    assert!(reviews.iter().all(|r| !r.enabled));
    let text = serde_json::to_string(&reviews).unwrap();
    for secret in [
        "SYNTHETIC_TOKEN",
        "SYNTHETIC_PASSWORD",
        "SYNTHETIC_OUTPUT_SECRET",
    ] {
        assert!(!text.contains(secret));
    }
    let schedule = reviews.iter().find(|r| r.kind == "schedule").unwrap();
    assert_eq!(schedule.data["recurrence"], "weekly");
    assert_eq!(schedule.data["next"], 123);
    assert_eq!(schedule.data["enabled"], false);
    assert_eq!(schedule.data["was_enabled"], true);
}
