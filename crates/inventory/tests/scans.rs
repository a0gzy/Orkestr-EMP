use chrono::{Duration, TimeZone, Utc};
use orkestr_domain::*;
use orkestr_inventory::*;
use serde_json::json;

fn report(providers: Vec<ProviderReport>) -> InventoryReport {
    let started = Utc.timestamp_opt(1_800_000_000, 0).unwrap();
    InventoryReport {
        id: uuid::Uuid::new_v4(),
        device_id: DeviceId::new(),
        started_at: started,
        finished_at: started + Duration::seconds(5),
        providers,
    }
}
fn complete(kind: InventoryProvider, rows: Vec<serde_json::Value>) -> ProviderReport {
    ProviderReport {
        provider: kind,
        state: ProviderState::Complete,
        failure: None,
        rows,
    }
}
fn app() -> serde_json::Value {
    json!({"name":"  Atlas   Editor ","publisher":"Example  Publisher", "version":" v02.010.0 ","architecture":"AMD64",
        "product_code":"{802d76c0-34af-477a-9c52-96d4697c1ad4}","registry_key":"Uninstall/fixture"})
}

#[test]
fn missing_providers_are_unknown_not_successful_empty_results() {
    let normalized = report(vec![]).normalize().unwrap();
    assert_eq!(normalized.state(), InventoryState::Unknown);
    assert_eq!(
        normalized.report().providers.len(),
        InventoryProvider::ALL.len()
    );
    assert!(normalized
        .report()
        .providers
        .iter()
        .all(|p| p.state == ProviderState::NotCollected));
    assert!(normalized.scan().is_none());
}

#[test]
fn all_failed_reports_do_not_create_a_scan() {
    let normalized = report(vec![ProviderReport {
        provider: InventoryProvider::OperatingSystem,
        state: ProviderState::Failed,
        failure: Some(ProviderFailure::Timeout),
        rows: vec![],
    }])
    .normalize()
    .unwrap();
    assert_eq!(normalized.state(), InventoryState::Failed);
    assert!(normalized.scan().is_none());
}

#[test]
fn partial_providers_keep_evidence_and_failure_codes_without_fabricating_sections() {
    let normalized = report(vec![
        complete(InventoryProvider::Services, vec![json!({"Name":"fixture"})]),
        ProviderReport {
            provider: InventoryProvider::Processes,
            state: ProviderState::Failed,
            failure: Some(ProviderFailure::AccessDenied),
            rows: vec![],
        },
    ])
    .normalize()
    .unwrap();
    assert_eq!(normalized.state(), InventoryState::Partial);
    let scan = normalized.scan().unwrap();
    assert_eq!(scan.details["Services"][0]["Name"], "fixture");
    assert!(scan.details.get("Processes").is_none());
    assert!(scan.details.get("TaskSchedules").is_none());
    assert_eq!(scan.software_state, ProviderState::NotCollected);
    assert_eq!(scan.installed_ram_bytes, None);
    assert_eq!(scan.scanned_at, normalized.report().started_at);
}

#[test]
fn full_provider_contract_covers_f21_and_can_be_complete_only_with_evidence() {
    let providers = InventoryProvider::ALL
        .into_iter()
        .map(|kind| {
            complete(
                kind,
                if kind == InventoryProvider::MemoryModules {
                    vec![json!({"Capacity":8 * GIB})]
                } else {
                    vec![]
                },
            )
        })
        .collect();
    let normalized = report(providers).normalize().unwrap();
    assert_eq!(normalized.state(), InventoryState::Complete);
    assert_eq!(
        normalized.scan().unwrap().software_state,
        ProviderState::Complete
    );
    assert_eq!(
        normalized.scan().unwrap().installed_ram_bytes,
        Some(8 * GIB)
    );
    for kind in InventoryProvider::ALL {
        assert!(normalized
            .scan()
            .unwrap()
            .details
            .get(kind.section())
            .is_some());
    }
}

#[test]
fn installed_ram_requires_complete_nonempty_positive_modules_without_overflow() {
    let normalized = report(vec![complete(
        InventoryProvider::MemoryModules,
        vec![
            json!({"Capacity":"8589934592"}),
            json!({"Capacity":8 * GIB}),
        ],
    )])
    .normalize()
    .unwrap();
    assert_eq!(
        normalized.scan().unwrap().installed_ram_bytes,
        Some(16 * GIB)
    );
    for rows in [
        vec![],
        vec![json!({"Capacity":0})],
        vec![json!({"Capacity":8 * GIB}), json!({})],
        vec![json!({"Capacity":u64::MAX}), json!({"Capacity":1})],
        vec![json!({"Capacity":8.5})],
    ] {
        let normalized = report(vec![complete(InventoryProvider::MemoryModules, rows)])
            .normalize()
            .unwrap();
        assert_eq!(normalized.scan().unwrap().installed_ram_bytes, None);
        let p = normalized
            .report()
            .providers
            .iter()
            .find(|p| p.provider == InventoryProvider::MemoryModules)
            .unwrap();
        assert_eq!(p.state, ProviderState::Partial);
        assert_eq!(p.failure, Some(ProviderFailure::InvalidData));
    }
    let mut p = complete(
        InventoryProvider::MemoryModules,
        vec![json!({"Capacity":16 * GIB})],
    );
    p.state = ProviderState::Partial;
    assert_eq!(
        report(vec![p])
            .normalize()
            .unwrap()
            .scan()
            .unwrap()
            .installed_ram_bytes,
        None
    );
}

#[test]
fn software_normalization_preserves_identity_scope_raw_version_and_unknowns() {
    let mut user = app();
    user["user_sid"] = json!("S-1-5-21-123-456-789-1001");
    let mut other_publisher = app();
    other_publisher["publisher"] = json!("Different publisher");
    let mut unknown = app();
    unknown["version"] = json!("2026-preview");
    unknown["architecture"] = json!("unknown");
    let normalized = report(vec![
        complete(
            InventoryProvider::SoftwareMachine,
            vec![app(), app(), other_publisher, unknown],
        ),
        complete(InventoryProvider::SoftwareUsers, vec![user]),
    ])
    .normalize()
    .unwrap();
    assert_eq!(normalized.software().len(), 4); // duplicates only; publisher/scope/version stay distinct
    assert!(normalized
        .software()
        .iter()
        .any(|s| s.version_normalized.as_deref() == Some("2.10")
            && s.architecture.as_deref() == Some("x64")));
    assert!(normalized
        .software()
        .iter()
        .any(|s| s.version_raw == "2026-preview"
            && s.version_normalized.is_none()
            && s.architecture.is_none()));
    assert!(normalized
        .software()
        .iter()
        .any(|s| matches!(s.scope, SoftwareScope::User { .. })));
    assert!(normalized
        .scan()
        .unwrap()
        .software
        .iter()
        .all(|s| s.application_id.is_none() && s.channel.is_none()));
    assert_eq!(
        normalized.scan().unwrap().software_state,
        ProviderState::Complete
    );
}

#[test]
fn machine_only_or_malformed_software_never_proves_absence_or_compliance() {
    let normalized = report(vec![complete(
        InventoryProvider::SoftwareMachine,
        vec![app()],
    )])
    .normalize()
    .unwrap();
    assert_eq!(
        normalized.scan().unwrap().software_state,
        ProviderState::Partial
    );
    assert_eq!(normalized.software().len(), 1);
    let normalized = report(vec![
        complete(
            InventoryProvider::SoftwareMachine,
            vec![app(), json!({"name":""})],
        ),
        complete(InventoryProvider::SoftwareUsers, vec![app()]),
    ])
    .normalize()
    .unwrap();
    assert_eq!(normalized.software().len(), 1); // user row missing SID is not reassigned to machine
    assert_eq!(
        normalized.scan().unwrap().software_state,
        ProviderState::Partial
    );
    assert!(normalized
        .report()
        .providers
        .iter()
        .filter(|p| p.provider == InventoryProvider::SoftwareUsers
            || p.provider == InventoryProvider::SoftwareMachine)
        .all(|p| p.state == ProviderState::Partial
            && p.failure == Some(ProviderFailure::InvalidData)));
}

#[test]
fn contradictory_duplicate_oversized_and_invalid_reports_fail_closed() {
    let p = complete(InventoryProvider::Services, vec![]);
    assert!(report(vec![p.clone(), p.clone()]).normalize().is_err());
    let mut bad = p.clone();
    bad.failure = Some(ProviderFailure::Timeout);
    assert!(report(vec![bad]).normalize().is_err());
    let mut bad = p.clone();
    bad.state = ProviderState::Failed;
    assert!(report(vec![bad]).normalize().is_err());
    let mut bad = p.clone();
    bad.state = ProviderState::NotCollected;
    bad.rows = vec![json!({})];
    assert!(report(vec![bad]).normalize().is_err());
    let mut bad = p.clone();
    bad.rows = vec![json!("SYNTHETIC_SECRET")];
    let error = report(vec![bad]).normalize().err().unwrap();
    assert!(!error.to_string().contains("SYNTHETIC_SECRET"));
    let mut bad = p.clone();
    bad.rows = vec![json!({}); MAX_PROVIDER_ROWS + 1];
    assert!(report(vec![bad]).normalize().is_err());
    let mut bad = p;
    bad.rows = vec![json!({"value":"x".repeat(MAX_REPORT_BYTES)})];
    assert!(report(vec![bad]).normalize().is_err());
    let mut bad = report(vec![]);
    bad.finished_at = bad.started_at - Duration::seconds(1);
    assert!(bad.normalize().is_err());
}

#[test]
fn normalized_partial_scan_keeps_existing_collection_unknown_semantics() {
    let mut snapshot = InventorySnapshot {
        generation: 0,
        devices: vec![],
        collections: vec![],
    };
    let plan = DiscoveryBatch::manual("fixture")
        .unwrap()
        .preview(&snapshot);
    let device = plan.writes()[0].clone();
    let mut input = report(vec![complete(
        InventoryProvider::SoftwareMachine,
        vec![app()],
    )]);
    input.device_id = device.id;
    let normalized = input.normalize().unwrap();
    let record = DeviceRecord {
        device,
        scan: normalized.scan().cloned(),
    };
    let context = EvaluationContext {
        now: input.finished_at,
        max_scan_age: Duration::hours(24),
        library: &[],
    };
    let rule = Expression::Software(SoftwareFilter {
        application_id: None,
        name: Some("Atlas Editor".into()),
        publisher: None,
        channel: None,
        architecture: None,
        version: None,
    });
    assert_eq!(rule.evaluate(&record, &context), Truth::Unknown);
    assert_eq!(
        Expression::RamUnknown.evaluate(&record, &context),
        Truth::Yes
    );
    snapshot.devices.push(record);
    assert_eq!(
        snapshot
            .preview_expression(&rule, &context)
            .unwrap()
            .unknown
            .len(),
        1
    );
}
