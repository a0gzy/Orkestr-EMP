use chrono::{Duration, TimeZone, Utc};
use orkestr_domain::*;
use std::collections::BTreeSet;

fn now() -> chrono::DateTime<Utc> {
    Utc.timestamp_opt(1_800_000_000, 0).unwrap()
}
fn context() -> EvaluationContext<'static> {
    EvaluationContext {
        now: now(),
        max_scan_age: Duration::hours(24),
        library: &[],
    }
}
fn record(ram: Option<u64>) -> DeviceRecord {
    let id = DeviceId::new();
    DeviceRecord {
        device: Device {
            id,
            host: "pc01.example.test".into(),
            name: "ПК-01".into(),
            aliases: vec![],
            ad_guid: None,
            domain: "Example".into(),
            ou: "OU=Lab".into(),
            os: "Windows 11".into(),
            source: "Fixture".into(),
            site: "Lab".into(),
            disabled: false,
            inventory_metadata: serde_json::json!({}),
        },
        scan: Some(DeviceScan {
            id: ScanId::new(),
            device_id: id,
            scanned_at: now(),
            installed_ram_bytes: ram,
            software_state: ProviderState::Complete,
            software: vec![],
            details: serde_json::json!({}),
        }),
    }
}
fn ram(op: Compare, gib: u64) -> Expression {
    Expression::Ram {
        op,
        bytes: gib * GIB,
    }
}
fn software(name: &str, publisher: &str, version: &str) -> InstalledSoftware {
    InstalledSoftware {
        application_id: None,
        name: name.into(),
        publisher: publisher.into(),
        version: version.into(),
        channel: None,
        architecture: None,
    }
}
fn filter() -> SoftwareFilter {
    SoftwareFilter {
        application_id: None,
        name: Some("Chrome".into()),
        publisher: Some("Google".into()),
        channel: None,
        architecture: None,
        version: Some((Compare::Lt, VersionOperand::Literal("2.10".into()))),
    }
}
fn collection(name: &str, parent_id: Option<CollectionId>, kind: CollectionKind) -> Collection {
    Collection {
        id: CollectionId::new(),
        name: name.into(),
        parent_id,
        revision: 1,
        kind,
    }
}

#[test]
fn numeric_versions_are_not_strings() {
    use std::cmp::Ordering::*;
    for (a, b, result) in [
        ("2.10", "2.9", Greater),
        ("v2.0.0.0", "2", Equal),
        ("2.0.1", "2", Greater),
        ("1.9", "2", Less),
    ] {
        assert_eq!(NumericVersion::compare(a, b), Some(result));
    }
    for bad in [
        "",
        "2..0",
        "2-beta",
        "1.0.0+build",
        "1.-2",
        "18446744073709551616",
        "v",
    ] {
        assert!(NumericVersion::parse(bad).is_err());
    }
}

#[test]
fn missing_publisher_is_unknown_but_known_different_identity_is_not_a_match() {
    let mut r = record(None);
    r.scan.as_mut().unwrap().software = vec![software("Chrome", "", "2.9")];
    let e = Expression::Software(filter());
    assert_eq!(e.evaluate(&r, &context()), Truth::Unknown);
    r.scan.as_mut().unwrap().software[0].publisher = "Other".into();
    assert_eq!(e.evaluate(&r, &context()), Truth::No);
    r.scan.as_mut().unwrap().software[0].publisher = "Google".into();
    assert_eq!(e.evaluate(&r, &context()), Truth::Yes);
    let expected = ApplicationId::new();
    let mut f = filter();
    f.application_id = Some(expected);
    r.scan.as_mut().unwrap().software[0].publisher = "Other".into();
    assert_eq!(
        Expression::Software(f.clone()).evaluate(&r, &context()),
        Truth::No
    );
    r.scan.as_mut().unwrap().software[0].application_id = Some(ApplicationId::new());
    r.scan.as_mut().unwrap().software[0].publisher.clear();
    assert_eq!(Expression::Software(f).evaluate(&r, &context()), Truth::No);
    assert_eq!(
        NumericVersion::parse("v02.010.0").unwrap().to_string(),
        "2.10"
    );
}

#[test]
fn exact_ram_boundaries_and_unknown() {
    let expressions = [
        ram(Compare::Ge, 16),
        ram(Compare::Ge, 8),
        ram(Compare::Lt, 8),
        Expression::And(vec![ram(Compare::Ge, 8), ram(Compare::Lt, 16)]),
    ];
    for (bytes, expected) in [
        (Some(8 * GIB - 1), [false, false, true, false]),
        (Some(8 * GIB), [false, true, false, true]),
        (Some(16 * GIB - 1), [false, true, false, true]),
        (Some(16 * GIB), [true, true, false, false]),
    ] {
        let r = record(bytes);
        for (e, yes) in expressions.iter().zip(expected) {
            assert_eq!(
                e.evaluate(&r, &context()),
                if yes { Truth::Yes } else { Truth::No }
            );
        }
        assert_eq!(Expression::RamUnknown.evaluate(&r, &context()), Truth::No);
    }
    for e in expressions {
        assert_eq!(e.evaluate(&record(None), &context()), Truth::Unknown);
    }
    assert_eq!(
        Expression::RamUnknown.evaluate(&record(None), &context()),
        Truth::Yes
    );
    for op in [Compare::Eq, Compare::Ge, Compare::Le] {
        assert_eq!(
            ram(op, 8).evaluate(&record(Some(8 * GIB)), &context()),
            Truth::Yes
        );
    }
    for op in [Compare::Ne, Compare::Gt, Compare::Lt] {
        assert_eq!(
            ram(op, 8).evaluate(&record(Some(8 * GIB)), &context()),
            Truth::No
        );
    }
}

#[test]
fn three_valued_and_or_and_scan_freshness() {
    let mut r = record(Some(16 * GIB));
    r.scan.as_mut().unwrap().scanned_at = now() - Duration::hours(25);
    let yes = Expression::Text {
        field: TextField::Domain,
        op: TextOp::Equals,
        value: "EXAMPLE".into(),
    };
    let no = Expression::Text {
        field: TextField::Domain,
        op: TextOp::Equals,
        value: "other".into(),
    };
    assert_eq!(
        Expression::And(vec![yes.clone(), ram(Compare::Ge, 8)]).evaluate(&r, &context()),
        Truth::Unknown
    );
    assert_eq!(
        Expression::And(vec![no.clone(), ram(Compare::Ge, 8)]).evaluate(&r, &context()),
        Truth::No
    );
    assert_eq!(
        Expression::Or(vec![yes, ram(Compare::Ge, 8)]).evaluate(&r, &context()),
        Truth::Yes
    );
    assert_eq!(
        Expression::Or(vec![no, ram(Compare::Ge, 8)]).evaluate(&r, &context()),
        Truth::Unknown
    );
    r.scan.as_mut().unwrap().scanned_at = now() + Duration::seconds(1);
    assert_eq!(ram(Compare::Ge, 8).evaluate(&r, &context()), Truth::Unknown);
    r.scan = None;
    assert_eq!(
        Expression::IsUnknown(Box::new(ram(Compare::Ge, 8))).evaluate(&r, &context()),
        Truth::Yes
    );
}

#[test]
fn software_conditions_cannot_cross_rows_and_deduplicate_devices() {
    let mut r = record(Some(8 * GIB));
    r.scan.as_mut().unwrap().software = vec![
        software("Chrome", "Other", "2.9"),
        software("Other", "Google", "2.9"),
    ];
    let e = Expression::Software(filter());
    assert_eq!(e.evaluate(&r, &context()), Truth::No);
    r.scan.as_mut().unwrap().software.extend([
        software("CHROME", "GOOGLE", "2.9"),
        software("Chrome", "Google", "2.9"),
    ]);
    let snapshot = InventorySnapshot {
        generation: 7,
        devices: vec![r],
        collections: vec![],
    };
    let p = snapshot.preview_expression(&e, &context()).unwrap();
    assert_eq!(p.members.len(), 1);
    assert_eq!(p.members[0].scan.as_ref().unwrap().software.len(), 4);
    assert_eq!(p.generation, 7);
}

#[test]
fn software_unknown_is_not_absent_or_outdated() {
    let mut r = record(None);
    let e = Expression::Software(filter());
    assert_eq!(e.evaluate(&r, &context()), Truth::No); // authoritative empty inventory
    r.scan.as_mut().unwrap().software = vec![software("Chrome", "Google", "unrecognized")];
    assert_eq!(e.evaluate(&r, &context()), Truth::Unknown);
    r.scan.as_mut().unwrap().software[0].version = "2.9".into();
    r.scan.as_mut().unwrap().software_state = ProviderState::Partial;
    assert_eq!(e.evaluate(&r, &context()), Truth::Unknown);
}

#[test]
fn library_reference_is_late_bound_and_not_approval() {
    let app = ApplicationId::new();
    let mut r = record(None);
    let mut installed = software("Chrome", "Google", "2.9");
    installed.application_id = Some(app);
    installed.channel = Some("Stable".into());
    installed.architecture = Some("x64".into());
    r.scan.as_mut().unwrap().software = vec![installed];
    let mut f = filter();
    f.application_id = Some(app);
    f.channel = Some("Stable".into());
    f.architecture = Some("x64".into());
    f.version = Some((
        Compare::Eq,
        VersionOperand::Library {
            application_id: app,
            channel: "Stable".into(),
            architecture: "x64".into(),
            approved: false,
        },
    ));
    let e = Expression::Software(f);
    e.validate().unwrap();
    assert_eq!(e.evaluate(&r, &context()), Truth::Unknown);
    let mut refs = vec![LibraryReference {
        installed_version_components: None,
        application_id: app,
        channel: "Stable".into(),
        architecture: "x64".into(),
        approved: false,
        version: "2.9".into(),
    }];
    assert_eq!(
        e.evaluate(
            &r,
            &EvaluationContext {
                library: &refs,
                ..context()
            }
        ),
        Truth::Yes
    );
    refs[0].version = "2.10".into();
    assert_eq!(
        e.evaluate(
            &r,
            &EvaluationContext {
                library: &refs,
                ..context()
            }
        ),
        Truth::No
    );
    refs[0].approved = true;
    assert_eq!(
        e.evaluate(
            &r,
            &EvaluationContext {
                library: &refs,
                ..context()
            }
        ),
        Truth::Unknown
    );
}

#[test]
fn folders_union_without_inheritance_or_double_counts() {
    let a = record(Some(8 * GIB));
    let b = record(Some(16 * GIB));
    let folder = collection("Оборудование", None, CollectionKind::Folder);
    let sub = collection("RAM", Some(folder.id), CollectionKind::Folder);
    let g8 = collection(
        ">=8",
        Some(sub.id),
        CollectionKind::Dynamic(ram(Compare::Ge, 8)),
    );
    let g16 = collection(
        ">=16",
        Some(sub.id),
        CollectionKind::Dynamic(ram(Compare::Ge, 16)),
    );
    let manual = collection(
        "Pilot",
        Some(folder.id),
        CollectionKind::Static(BTreeSet::from([b.device.id])),
    );
    let mut s = InventorySnapshot {
        generation: 10,
        devices: vec![a, b],
        collections: vec![folder.clone(), sub.clone(), g8, g16, manual.clone()],
    };
    assert_eq!(
        s.preview_collection(folder.id, &context())
            .unwrap()
            .members
            .len(),
        2
    );
    assert!(s.paths().unwrap()[&manual.id].starts_with("Оборудование / "));
    let mut cycle = folder.clone();
    cycle.parent_id = Some(sub.id);
    assert!(s.validate_collection(&cycle).is_err());
    let mut converted = folder;
    converted.kind = CollectionKind::Static(BTreeSet::new());
    assert!(s.validate_collection(&converted).is_err());
    s.devices[1].scan.as_mut().unwrap().installed_ram_bytes = Some(2 * GIB);
    assert_eq!(
        s.preview_collection(manual.id, &context())
            .unwrap()
            .members
            .len(),
        1
    );
}

#[test]
fn invalid_expression_and_membership_are_rejected() {
    assert!(Expression::And(vec![]).validate().is_err());
    assert!(Expression::Or(vec![ram(Compare::Ge, 8); 257])
        .validate()
        .is_err());
    let mut e = ram(Compare::Ge, 8);
    for _ in 0..18 {
        e = Expression::IsUnknown(Box::new(e));
    }
    assert!(e.validate().is_err());
    let s = InventorySnapshot {
        generation: 0,
        devices: vec![],
        collections: vec![],
    };
    assert!(s
        .validate_collection(&collection(
            "Bad",
            None,
            CollectionKind::Static(BTreeSet::from([DeviceId::new()]))
        ))
        .is_err());
}

#[test]
fn canonical_names_do_not_depend_on_case_or_ip_identity() {
    assert_eq!(
        canonical_host(" PC-01.Example.Test. ").unwrap(),
        "pc-01.example.test"
    );
    for bad in [
        "",
        "-pc",
        "pc-",
        "a..b",
        "pc'; shutdown",
        "https://pc",
        "a/b",
    ] {
        assert!(canonical_host(bad).is_err());
    }
}

#[test]
fn unknown_union_and_nested_folders_remain_consistent() {
    let r = record(None);
    let folder = collection("Root", None, CollectionKind::Folder);
    let dynamic = collection(
        "RAM",
        Some(folder.id),
        CollectionKind::Dynamic(ram(Compare::Ge, 8)),
    );
    let manual = collection(
        "Manual",
        Some(folder.id),
        CollectionKind::Static(BTreeSet::from([r.device.id])),
    );
    let mut s = InventorySnapshot {
        generation: 1,
        devices: vec![r],
        collections: vec![folder.clone(), dynamic, manual],
    };
    let p = s.preview_collection(folder.id, &context()).unwrap();
    assert_eq!(p.members.len(), 1);
    assert!(p.unknown.is_empty());
    let mut parent = folder.id;
    for _ in 0..1024 {
        let child = collection("Nested", Some(parent), CollectionKind::Folder);
        parent = child.id;
        s.collections.push(child);
    }
    assert_eq!(
        s.preview_collection(folder.id, &context())
            .unwrap()
            .members
            .len(),
        1
    );
}

#[test]
fn unmapped_application_is_unknown_and_names_are_case_insensitive() {
    let mut r = record(None);
    r.scan.as_mut().unwrap().software = vec![software("РЕДАКТОР", "ИЗДАТЕЛЬ", "2.9")];
    let mut f = filter();
    f.name = Some("редактор".into());
    f.publisher = Some("издатель".into());
    assert_eq!(
        Expression::Software(f.clone()).evaluate(&r, &context()),
        Truth::Yes
    );
    f.application_id = Some(ApplicationId::new());
    assert_eq!(
        Expression::Software(f).evaluate(&r, &context()),
        Truth::Unknown
    );
}
