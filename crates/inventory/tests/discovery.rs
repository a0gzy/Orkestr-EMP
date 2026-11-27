use orkestr_domain::*;
use orkestr_inventory::*;

fn snapshot() -> InventorySnapshot {
    InventorySnapshot {
        generation: 7,
        devices: vec![],
        collections: vec![],
    }
}

#[test]
fn directory_guid_renames_the_same_device_and_conflicting_names_block_the_batch() {
    let guid = uuid::Uuid::new_v4();
    let row = DirectoryComputer {
        guid,
        host: "old.example.test".into(),
        name: "Office PC".into(),
        ou: "OU=Office,DC=example,DC=test".into(),
        os: "Windows 11".into(),
        disabled: false,
    };
    let initial = DiscoveryBatch::directory(vec![row.clone()])
        .unwrap()
        .preview(&snapshot());
    let mut device = initial.writes()[0].clone();
    device.site = "Office".into();
    let original_id = device.id;
    let mut existing = snapshot();
    existing.devices.push(DeviceRecord { device, scan: None });
    let renamed = DirectoryComputer {
        host: "new.example.test".into(),
        ..row.clone()
    };
    let preview = DiscoveryBatch::directory(vec![renamed.clone()])
        .unwrap()
        .preview(&existing);
    assert!(preview.can_apply());
    assert_eq!(preview.writes()[0].id, original_id);
    assert_eq!(preview.writes()[0].host, renamed.host);
    assert_eq!(preview.writes()[0].site, "Office");
    let collision = DirectoryComputer {
        guid: uuid::Uuid::new_v4(),
        ..row
    };
    let preview = DiscoveryBatch::directory(vec![renamed, collision])
        .unwrap()
        .preview(&existing);
    assert!(!preview.can_apply());
    assert_eq!(preview.rows()[1].action, DiscoveryAction::Conflict);
}

#[test]
fn manual_normalizes_without_dns_and_previews_duplicates_without_writes() {
    let snapshot = snapshot();
    let plan =
        DiscoveryBatch::manual("\u{feff}PC-01.Example.Test.\r\n\r\npc-01.example.test\npc-02\n")
            .unwrap()
            .preview(&snapshot);
    assert!(plan.can_apply());
    assert_eq!(plan.generation(), 7);
    assert_eq!(
        plan.rows().iter().map(|r| r.action).collect::<Vec<_>>(),
        vec![
            DiscoveryAction::Add,
            DiscoveryAction::Duplicate,
            DiscoveryAction::Add
        ]
    );
    assert_eq!(plan.rows()[0].device_id, plan.rows()[1].device_id);
    assert_eq!(plan.writes().len(), 2);
    assert_eq!(plan.writes()[0].host, "pc-01.example.test");
    assert!(snapshot.devices.is_empty());
}

#[test]
fn csv_handles_bom_quotes_unicode_and_optional_fields_without_erasing() {
    let plan =
        DiscoveryBatch::csv("\u{feff}Host,Name,Site\r\npc-01,\"Приёмная, ПК \"\"A\"\"\",Офис\r\n")
            .unwrap()
            .preview(&snapshot());
    let device = plan.writes()[0].clone();
    assert_eq!(device.name, "Приёмная, ПК \"A\"");
    assert_eq!(device.site, "Офис");
    let mut snapshot = snapshot();
    snapshot.devices.push(DeviceRecord { device, scan: None });
    let plan = DiscoveryBatch::csv("host,name,site\npc-01,,\n")
        .unwrap()
        .preview(&snapshot);
    assert_eq!(plan.rows()[0].action, DiscoveryAction::Unchanged);
    assert!(plan.writes().is_empty());
    let plan = DiscoveryBatch::csv("host,site\npc-01,Филиал\n")
        .unwrap()
        .preview(&snapshot);
    assert_eq!(plan.rows()[0].action, DiscoveryAction::Update);
    assert_eq!(plan.writes()[0].name, snapshot.devices[0].device.name);
    assert_eq!(plan.writes()[0].id, snapshot.devices[0].device.id);
}

#[test]
fn invalid_inputs_and_limits_fail_without_echoing_source() {
    for raw in [
        "",
        "  \n",
        "127.0.0.1",
        "127.1",
        "::1",
        "https://pc",
        "pc;evil",
        "pc/path",
        "pc *",
        "-pc",
        "pc..example",
    ] {
        assert!(DiscoveryBatch::manual(raw).is_err(), "{raw}");
    }
    for raw in [
        "host,password\npc,SYNTHETIC_SECRET",
        "host,HOST\npc,pc",
        "name\npc",
        "host,site\npc",
        "host\npc,other",
        "host,name\npc,\"unclosed",
        "host,name\npc,ab\"cd",
        "host,name\npc,\"ok\"junk",
        "host,name\npc,\"line\nbreak\"",
    ] {
        let error = DiscoveryBatch::csv(raw).err().expect(raw);
        assert!(!error.to_string().contains("SYNTHETIC_SECRET"));
    }
    assert!(DiscoveryBatch::manual(&"x".repeat(MAX_DISCOVERY_BYTES + 1)).is_err());
    let limit = (0..MAX_DISCOVERY_ROWS)
        .map(|n| format!("pc-{n}\n"))
        .collect::<String>();
    assert!(DiscoveryBatch::manual(&limit).is_ok());
    assert!(DiscoveryBatch::manual(&(limit + "pc-extra\n")).is_err());
}

#[test]
fn conflicting_rows_block_entire_preview_but_identical_duplicates_do_not() {
    let plan = DiscoveryBatch::csv("host,site\npc,A\nPC,B\npc,A\nother,C")
        .unwrap()
        .preview(&snapshot());
    assert!(!plan.can_apply());
    assert_eq!(plan.rows()[0].action, DiscoveryAction::Conflict);
    assert_eq!(plan.rows()[1].action, DiscoveryAction::Conflict);
    assert_eq!(plan.rows()[2].action, DiscoveryAction::Duplicate);
    let plan = DiscoveryBatch::csv("host,name\npc,Same\nPC,Same")
        .unwrap()
        .preview(&snapshot());
    assert!(plan.can_apply());
    assert_eq!(plan.writes().len(), 1);
}

#[test]
fn aliases_keep_uuid_and_ad_metadata_but_two_aliases_cannot_update_one_device() {
    let new = DiscoveryBatch::manual("new.example.test")
        .unwrap()
        .preview(&snapshot());
    let mut device = new.writes()[0].clone();
    device.aliases = vec!["old.example.test".into()];
    device.ad_guid = Some(uuid::Uuid::new_v4());
    device.disabled = true;
    device.domain = "example.test".into();
    device.ou = "OU=Fixture".into();
    device.inventory_metadata = serde_json::json!({"preserved":true});
    let snapshot = InventorySnapshot {
        devices: vec![DeviceRecord {
            device: device.clone(),
            scan: None,
        }],
        ..snapshot()
    };
    let plan = DiscoveryBatch::csv("host,site\nold.example.test,Pilot")
        .unwrap()
        .preview(&snapshot);
    let updated = &plan.writes()[0];
    assert_eq!(updated.host, "new.example.test");
    assert_eq!(updated.id, device.id);
    assert_eq!(updated.ad_guid, device.ad_guid);
    assert!(updated.disabled);
    assert_eq!(updated.inventory_metadata, device.inventory_metadata);
    assert_eq!(updated.ou, device.ou);
    assert_eq!(updated.domain, device.domain);
    let plan = DiscoveryBatch::manual("new.example.test\nold.example.test")
        .unwrap()
        .preview(&snapshot);
    assert!(!plan.can_apply());
    assert!(plan
        .rows()
        .iter()
        .all(|r| r.action == DiscoveryAction::Conflict));
}

#[test]
fn short_names_are_not_merged_with_fqdns_and_ambiguous_aliases_block() {
    let plan = DiscoveryBatch::manual("pc\npc.example.test")
        .unwrap()
        .preview(&snapshot());
    assert_eq!(plan.writes().len(), 2);
    let mut a = plan.writes()[0].clone();
    let mut b = plan.writes()[1].clone();
    a.aliases.push("ambiguous".into());
    b.aliases.push("ambiguous".into());
    let snapshot = InventorySnapshot {
        devices: vec![
            DeviceRecord {
                device: a,
                scan: None,
            },
            DeviceRecord {
                device: b,
                scan: None,
            },
        ],
        ..snapshot()
    };
    let plan = DiscoveryBatch::manual("ambiguous")
        .unwrap()
        .preview(&snapshot);
    assert!(!plan.can_apply());
    assert!(plan.writes().is_empty());
}
