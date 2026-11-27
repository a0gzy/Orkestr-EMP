use super::*;
use std::time::{Duration, Instant};

fn next(service: &Service) -> Event {
    service
        .receiver
        .recv_timeout(Duration::from_secs(10))
        .expect("bounded worker response")
}

#[test]
fn discovery_worker_reads_local_content_previews_then_applies_and_replays_receipt() {
    use orkestr_protocol::DiscoverySource;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db.sqlite");
    let file = dir.path().join("devices.csv");
    let raw = "host,name,site\npc-worker,Учебный ПК,Офис";
    std::fs::write(&file, raw).unwrap();
    let service = start(path.clone());
    next(&service);
    send(
        &service,
        Command::ReadDiscoveryFile {
            token: 1,
            path: file,
        },
    );
    let text = match next(&service) {
        Event::DiscoveryText { token: 1, text } => text,
        _ => panic!("file load failed"),
    };
    assert_eq!(text, raw);
    send(
        &service,
        Command::PrepareDiscovery {
            token: 2,
            input: PrepareDiscovery {
                format: DiscoverySource::Csv,
                text,
            },
        },
    );
    let p = match next(&service) {
        Event::DiscoveryPrepared { token: 2, preview } => preview,
        _ => panic!("preview failed"),
    };
    assert!(Store::open_read_only(&path)
        .unwrap()
        .snapshot()
        .unwrap()
        .devices
        .is_empty());
    let request = ApplyDiscovery {
        preview_id: p.id,
        confirmed: true,
    };
    send(
        &service,
        Command::ApplyDiscovery {
            token: 2,
            input: request.clone(),
        },
    );
    let receipt = match next(&service) {
        Event::DiscoveryApplied { token: 2, result } => result,
        _ => panic!("apply failed"),
    };
    assert_eq!(receipt.changes, 1);
    send(
        &service,
        Command::ApplyDiscovery {
            token: 2,
            input: request,
        },
    );
    match next(&service) {
        Event::DiscoveryApplied { token: 2, result } => assert_eq!(result, receipt),
        _ => panic!("replay failed"),
    };
    send(&service, Command::Refresh);
    let w = match next(&service) {
        Event::Loaded(w) => w,
        _ => panic!("refresh failed"),
    };
    send(
        &service,
        Command::InventoryStatus {
            token: 3,
            device_id: w.inventory.devices[0].device.id,
        },
    );
    assert!(
        matches!(next(&service),Event::InventoryStatus{token:3,status} if status.latest.is_none())
    );
}

#[test]
fn discovery_file_reader_rejects_network_paths_bad_utf8_and_oversized_content() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("bad.csv");
    std::fs::write(&file, [0xff, 0xfe]).unwrap();
    assert!(read_discovery_file(&file).is_err());
    std::fs::write(&file, vec![b'x'; DISCOVERY_TEXT_LIMIT + 1]).unwrap();
    assert!(read_discovery_file(&file).is_err());
    assert!(read_discovery_file(dir.path()).is_err());
    for path in [
        "\\\\no-network-fixture\\share\\x.csv",
        "https://no-network-fixture/x",
    ] {
        assert!(read_discovery_file(std::path::Path::new(path)).is_err());
    }
}
fn start(path: PathBuf) -> Service {
    Service::start(path, true, Arc::new(|| {}))
}
fn send(service: &Service, command: Command) {
    assert!(service.submit(command).is_ok());
}
fn group() -> Collection {
    Collection {
        id: CollectionId::new(),
        parent_id: None,
        name: "Test group".into(),
        revision: 0,
        kind: CollectionKind::Dynamic(Expression::RamUnknown),
    }
}

#[test]
fn local_service_creates_only_explicitly_and_keeps_edit_conflicts() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("new.sqlite");
    let missing = Service::start(path.clone(), false, Arc::new(|| {}));
    assert!(matches!(
        next(&missing),
        Event::Error {
            operation: "open",
            ..
        }
    ));
    assert!(!path.exists());
    let service = start(path.clone());
    assert!(matches!(next(&service), Event::Loaded(_)));
    let c = group();
    send(
        &service,
        Command::Save {
            collection: Box::new(c.clone()),
            expected: None,
        },
    );
    match next(&service) {
        Event::Saved { workspace, .. } => {
            assert_eq!(workspace.inventory.collections[0].revision, 1)
        }
        _ => panic!("save failed"),
    }
    send(
        &service,
        Command::Save {
            collection: Box::new(c),
            expected: None,
        },
    );
    assert!(matches!(
        next(&service),
        Event::Error {
            operation: "save",
            ..
        }
    ));
    let duplicate = start(path);
    assert!(matches!(
        next(&duplicate),
        Event::Error {
            operation: "open",
            ..
        }
    ));
}

#[test]
fn preview_ticket_and_generation_do_not_mutate_the_database() {
    let dir = tempfile::tempdir().unwrap();
    let service = start(dir.path().join("db.sqlite"));
    let initial = match next(&service) {
        Event::Loaded(w) => w,
        _ => panic!("load failed"),
    };
    send(
        &service,
        Command::Preview {
            token: 123,
            selection: Selection::Draft(Box::new(group())),
        },
    );
    match next(&service) {
        Event::Preview { token, preview } => {
            assert_eq!(token, 123);
            assert_eq!(preview.generation, initial.inventory.generation);
            assert!(preview.members.is_empty());
        }
        _ => panic!("preview failed"),
    }
    send(&service, Command::Refresh);
    match next(&service) {
        Event::Loaded(w) => assert!(w.inventory.collections.is_empty()),
        _ => panic!("refresh failed"),
    }
}

#[test]
fn import_requires_current_ticket_and_creates_rollback_before_writing() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.json");
    let path = dir.path().join("db.sqlite");
    let backup = dir.path().join("rollback.sqlite");
    std::fs::write(&source,r#"{"packages":[{"id":1,"name":"Application fixture","steps":[]}],"schedules":[{"id":1,"enabled":true,"next":123,"recurrence":"weekly"}]}"#).unwrap();
    let service = start(path.clone());
    next(&service);
    send(
        &service,
        Command::PrepareImport {
            token: 4,
            format: ImportFormat::Deploy,
            path: source.clone(),
            origin: Some("test-server".into()),
        },
    );
    match next(&service) {
        Event::ImportPrepared { token, report } => {
            assert_eq!(token, 4);
            assert!(report.can_apply());
            assert_eq!(report.review_items, 1);
        }
        _ => panic!("prepare failed"),
    }
    send(
        &service,
        Command::ApplyImport {
            token: 3,
            backup: backup.clone(),
        },
    );
    assert!(matches!(
        next(&service),
        Event::Error {
            operation: "import apply",
            ..
        }
    ));
    assert!(!backup.exists());
    send(
        &service,
        Command::PrepareImport {
            token: 5,
            format: ImportFormat::Deploy,
            path: source,
            origin: Some("test-server".into()),
        },
    );
    next(&service);
    send(
        &service,
        Command::ApplyImport {
            token: 5,
            backup: backup.clone(),
        },
    );
    match next(&service) {
        Event::Imported { workspace, .. } => {
            assert_eq!(workspace.packages.len(), 1);
            assert!(!workspace.reviews[0].enabled);
        }
        _ => panic!("apply failed"),
    }
    assert!(Store::open_read_only(&backup)
        .unwrap()
        .packages()
        .unwrap()
        .is_empty());
    assert_eq!(
        Store::open_read_only(&path)
            .unwrap()
            .packages()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn fifty_computers_preview_and_tree_counters_share_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bench.sqlite");
    let mut store = Store::open(&path).unwrap();
    for i in 0..50 {
        let id = DeviceId::new();
        store
            .upsert_device(&Device {
                id,
                host: format!("fixture-{i:02}.example.test"),
                aliases: vec![],
                ad_guid: None,
                name: format!("Fixture {i}"),
                domain: "example.test".into(),
                ou: "OU=Fixture".into(),
                os: "Windows fixture".into(),
                source: "TEST ONLY".into(),
                site: "test".into(),
                disabled: false,
                inventory_metadata: serde_json::json!({}),
            })
            .unwrap();
        store
            .append_scan(&DeviceScan {
                id: ScanId::new(),
                device_id: id,
                scanned_at: Utc::now(),
                installed_ram_bytes: Some(if i % 2 == 0 { 16 * GIB } else { 8 * GIB }),
                software_state: ProviderState::Complete,
                software: vec![],
                details: serde_json::json!({"fixture":true}),
            })
            .unwrap();
    }
    let mut c = group();
    c.kind = CollectionKind::Dynamic(Expression::And(vec![
        Expression::Ram {
            op: Compare::Ge,
            bytes: 8 * GIB,
        },
        Expression::Ram {
            op: Compare::Lt,
            bytes: 16 * GIB,
        },
    ]));
    store.save_collection(&c, None).unwrap();
    drop(store);
    let service = Service::start(path, false, Arc::new(|| {}));
    let loaded = match next(&service) {
        Event::Loaded(w) => w,
        _ => panic!("load failed"),
    };
    let started = Instant::now();
    send(
        &service,
        Command::Preview {
            token: 1,
            selection: Selection::Group(c.id),
        },
    );
    match next(&service) {
        Event::Preview { preview, .. } => {
            assert_eq!(preview.members.len(), 25);
            assert_eq!(loaded.counts[&c.id], (25, 0));
            assert_eq!(preview.generation, loaded.inventory.generation);
            assert_eq!(preview.evaluated_at, loaded.evaluated_at);
        }
        _ => panic!("preview failed"),
    }
    println!(
        "50 fixture devices / worker preview round-trip: {:?} (debug, local only)",
        started.elapsed()
    );
}

#[test]
fn bounded_worker_rejects_backpressure_without_blocking_submitter() {
    let dir = tempfile::tempdir().unwrap();
    let service = start(dir.path().join("bounded.sqlite"));
    next(&service);
    let mut busy = 0;
    for token in 0..100 {
        if service.submit(Command::Preview {
            token,
            selection: Selection::All,
        }) == Err(SubmitError::Busy)
        {
            busy += 1;
        }
    }
    assert!(
        busy > 0,
        "bounded command and event queues must apply backpressure"
    );
}
