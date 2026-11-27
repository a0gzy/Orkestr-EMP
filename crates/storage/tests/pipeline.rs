use chrono::Utc;
use orkestr_domain::*;
use orkestr_protocol::*;
use orkestr_storage::*;
use uuid::Uuid;
fn application() -> LibraryApplication {
    serde_json::from_value(serde_json::json!({"id":ApplicationId::new(),"revision":0,"name":"Fixture","channel":"stable","architecture":"x64","source":{"kind":"Manual","version":"2.0"},"download":{"kind":"Manual","url":"https://repo.example.test/file.exe"},"network":{"allowed_hosts":["repo.example.test"],"max_bytes":1048576},"verification":{"kind":"Hash","expected_sha256":"a".repeat(64)},"software_match":{"name":"Fixture","publisher":"Publisher","product_code":null}})).unwrap()
}
#[test]
fn known_approved_and_software_matching_recalculate_without_mutating_scans_or_starting_jobs() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pipeline.sqlite");
    let mut store = Store::open(&path).unwrap();
    let actor = Uuid::new_v4();
    let app = store
        .save_application(
            actor,
            &SaveApplication {
                application: application(),
                expected: None,
            },
        )
        .unwrap();
    let discovery = store
        .preview_discovery(&orkestr_inventory::DiscoveryBatch::manual("fixture.invalid").unwrap())
        .unwrap();
    store.apply_discovery(&discovery).unwrap();
    let device = store.snapshot().unwrap().devices[0].device.id;
    let scan = DeviceScan {
        id: ScanId::new(),
        device_id: device,
        scanned_at: Utc::now(),
        installed_ram_bytes: None,
        software_state: ProviderState::Complete,
        software: vec![InstalledSoftware {
            application_id: None,
            name: "Fixture".into(),
            publisher: "Publisher".into(),
            version: "1.0".into(),
            channel: None,
            architecture: Some("x64".into()),
        }],
        details: serde_json::Value::Null,
    };
    store.append_scan(&scan).unwrap();
    let preview = orkestr_library::preview_source(&app, &[], None, vec![], None).unwrap();
    let mut release = store.add_release(actor, &app, preview).unwrap();
    let collection = Collection {
        id: CollectionId::new(),
        parent_id: None,
        name: "Old fixture".into(),
        revision: 0,
        kind: CollectionKind::Dynamic(Expression::Software(SoftwareFilter {
            application_id: Some(app.id),
            name: None,
            publisher: None,
            channel: Some("stable".into()),
            architecture: Some("x64".into()),
            version: Some((
                Compare::Lt,
                VersionOperand::Library {
                    application_id: app.id,
                    channel: "stable".into(),
                    architecture: "x64".into(),
                    approved: false,
                },
            )),
        })),
    };
    store.save_collection(&collection, None).unwrap();
    let snapshot = store.snapshot().unwrap();
    let refs = store.library_references().unwrap();
    assert_eq!(refs.len(), 1);
    assert!(!refs[0].approved);
    let context = EvaluationContext {
        now: Utc::now(),
        max_scan_age: chrono::Duration::hours(24),
        library: &refs,
    };
    assert_eq!(
        snapshot
            .preview_collection(collection.id, &context)
            .unwrap()
            .members
            .len(),
        1
    );
    release.state = ReleaseState::Quarantined;
    release.sha256 = Some("a".repeat(64));
    release.bytes = Some(20);
    store.update_release(actor, &release).unwrap();
    assert!(store
        .approve_release(
            actor,
            &ChangeRelease {
                id: release.id,
                application_revision: app.revision
            },
            true
        )
        .is_err());
    release.state = ReleaseState::Verified;
    store.update_release(actor, &release).unwrap();
    store
        .approve_release(
            actor,
            &ChangeRelease {
                id: release.id,
                application_revision: app.revision,
            },
            true,
        )
        .unwrap();
    assert_eq!(store.library_references().unwrap().len(), 2);
    assert!(store.update_release(actor, &release).is_err()); // stale verification cannot undo approval
    assert!(store.library_jobs().unwrap().is_empty());
    assert!(store.deployments().unwrap().is_empty());
    let revised = store
        .save_application(
            actor,
            &SaveApplication {
                application: app.clone(),
                expected: Some(app.revision),
            },
        )
        .unwrap();
    assert_eq!(revised.revision, 2);
    assert!(store.library_references().unwrap().is_empty());
    drop(store);
    let db = rusqlite::Connection::open(&path).unwrap();
    let data: String = db
        .query_row(
            "SELECT data FROM scans WHERE id=?1",
            [scan.id.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(serde_json::from_str::<DeviceScan>(&data).unwrap(), scan);
}
#[test]
fn concurrent_reservations_cannot_bypass_global_quota_and_recovery_is_explicit() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("quotas.sqlite");
    let mut store = Store::open(&path).unwrap();
    store
        .save_quotas(
            Uuid::new_v4(),
            0,
            &Quotas {
                global: 1,
                ..Quotas::default()
            },
        )
        .unwrap();
    drop(store);
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let threads: Vec<_> = (0..2)
        .map(|index| {
            let path = path.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let mut store = Store::open(&path).unwrap();
                barrier.wait();
                store
                    .acquire_resource(Uuid::new_v4(), &format!("site-{index}"), Phase::Install)
                    .unwrap()
            })
        })
        .collect();
    assert_eq!(
        threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .filter(|claimed| *claimed)
            .count(),
        1
    );
    let mut store = Store::open(&path).unwrap();
    assert!(!store
        .acquire_resource(Uuid::new_v4(), "third", Phase::Download)
        .unwrap());
    store.recover_resources().unwrap();
    assert!(store
        .acquire_resource(Uuid::new_v4(), "third", Phase::Download)
        .unwrap());
}
#[test]
fn workflow_nested_steps_are_pinned_and_invalid_paths_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("workflow.sqlite")).unwrap();
    let actor = Uuid::new_v4();
    let nested = WorkflowDefinition {
        name: "Child".into(),
        version: "1".into(),
        steps: vec![WorkflowStep::Sleep { seconds: 0 }],
    };
    let child = store.register_workflow(actor, &nested).unwrap();
    let parent = WorkflowDefinition {
        name: "Parent".into(),
        version: "1".into(),
        steps: vec![
            WorkflowStep::NestedPackage {
                revision_id: child.id,
            },
            WorkflowStep::SystemScan {},
        ],
    };
    let parent = store.register_workflow(actor, &parent).unwrap();
    let flattened = store.workflow_definition(parent.id).unwrap().unwrap();
    assert_eq!(flattened.steps.len(), 2);
    assert!(matches!(flattened.steps[0], WorkflowStep::Sleep { .. }));
    for path in [
        r"\\server\share\x",
        r"C:\target\..\windows\x",
        r"C:\target\x:stream",
        r"C:\target\nul.",
    ] {
        assert!(!valid_remote_path(path), "{path}");
    }
    assert!(valid_remote_path(r"C:\OrkestrPilot\payload.bin"));
}

#[test]
fn persisted_dispatch_cursor_reaches_old_backlogs_larger_than_a_batch() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fair.sqlite");
    drop(Store::open(&path).unwrap());
    let mut db = rusqlite::Connection::open(&path).unwrap();
    let tx = db.transaction().unwrap();
    let actor = Uuid::new_v4();
    let app = ApplicationId::new();
    // Simulate a recovered mixed backlog without depending on the API admission cap.
    for index in 1..=300u128 {
        let job = LibraryJob {
            id: Uuid::from_u128(index),
            actor_id: actor,
            task: LibraryTask::Check {
                application_id: app,
                revision: 1,
            },
            state: LibraryJobState::Queued,
            code: "QUEUED".into(),
            created_at: Utc::now(),
            release_id: None,
            attempts: 0,
            retry_at: None,
        };
        tx.execute(
            "INSERT INTO library_jobs VALUES(?1,?2,?3,'Queued',?4)",
            rusqlite::params![
                job.id.to_string(),
                actor.to_string(),
                index.to_string(),
                serde_json::to_string(&job).unwrap()
            ],
        )
        .unwrap();
    }
    tx.commit().unwrap();
    drop(db);
    let mut store = Store::open(&path).unwrap();
    let first = store.pending_server_work().unwrap();
    assert_eq!(first.len(), 256);
    store.record_dispatch(*first.last().unwrap()).unwrap();
    drop(store);
    let mut store = Store::open(&path).unwrap();
    let next = store.pending_server_work().unwrap();
    assert_eq!(next.len(), 44);
    assert!(next.iter().all(|id| !first.contains(id)));
    store.record_dispatch(*next.last().unwrap()).unwrap();
    assert_eq!(store.pending_server_work().unwrap(), first);
}

#[test]
fn approved_reference_and_downgrade_guard_outlive_the_bounded_history_list() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.sqlite");
    let mut store = Store::open(&path).unwrap();
    let actor = Uuid::new_v4();
    let app = store
        .save_application(
            actor,
            &SaveApplication {
                application: application(),
                expected: None,
            },
        )
        .unwrap();
    let other = store
        .save_application(
            actor,
            &SaveApplication {
                application: application(),
                expected: None,
            },
        )
        .unwrap();
    let mut release = store
        .add_release(
            actor,
            &app,
            orkestr_library::preview_source(&app, &[], None, vec![], None).unwrap(),
        )
        .unwrap();
    release.state = ReleaseState::Quarantined;
    release.sha256 = Some("a".repeat(64));
    store.update_release(actor, &release).unwrap();
    release.state = ReleaseState::Verified;
    store.update_release(actor, &release).unwrap();
    release = store
        .approve_release(
            actor,
            &ChangeRelease {
                id: release.id,
                application_revision: app.revision,
            },
            true,
        )
        .unwrap();
    drop(store);
    let mut db = rusqlite::Connection::open(&path).unwrap();
    let tx = db.transaction().unwrap();
    for _ in 0..1025 {
        let mut unrelated = release.clone();
        unrelated.id = Uuid::new_v4();
        unrelated.application_id = other.id;
        unrelated.state = ReleaseState::Candidate;
        tx.execute(
            "INSERT INTO library_releases VALUES(?1,?2,?3,'Candidate',?4)",
            rusqlite::params![
                unrelated.id.to_string(),
                other.id.to_string(),
                other.revision,
                serde_json::to_string(&unrelated).unwrap()
            ],
        )
        .unwrap();
    }
    tx.commit().unwrap();
    drop(db);
    let mut store = Store::open(&path).unwrap();
    assert!(!store
        .library_releases()
        .unwrap()
        .iter()
        .any(|r| r.id == release.id));
    assert!(store
        .library_references()
        .unwrap()
        .iter()
        .any(|r| r.application_id == app.id && r.approved && r.version == "2"));
    let mut older = app.clone();
    older.source = VersionSource::Manual {
        version: "1".into(),
    };
    let mut candidate = store
        .add_release(
            actor,
            &app,
            orkestr_library::preview_source(&older, &[], None, vec![], None).unwrap(),
        )
        .unwrap();
    candidate.state = ReleaseState::Quarantined;
    candidate.sha256 = Some("b".repeat(64));
    store.update_release(actor, &candidate).unwrap();
    candidate.state = ReleaseState::Verified;
    store.update_release(actor, &candidate).unwrap();
    assert!(matches!(
        store.approve_release(
            actor,
            &ChangeRelease {
                id: candidate.id,
                application_revision: app.revision
            },
            true
        ),
        Err(Error::Invalid("release downgrade"))
    ));
}
