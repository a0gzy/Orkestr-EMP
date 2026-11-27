use super::*;
use orkestr_storage::Store;

fn input(events: Vec<egui::Event>, size: [f32; 2]) -> egui::RawInput {
    egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(size[0], size[1]),
        )),
        events,
        ..Default::default()
    }
}
fn frame(app: &mut Console, ctx: &egui::Context, events: Vec<egui::Event>) -> egui::FullOutput {
    ctx.run(input(events, [1280., 820.]), |ctx| app.render(ctx))
}
fn wait(app: &mut Console, ctx: &egui::Context, predicate: impl Fn(&Console) -> bool) {
    let until = Instant::now() + Duration::from_secs(10);
    while !predicate(app) {
        assert!(
            Instant::now() < until,
            "UI worker timeout, status {:?}",
            app.error
        );
        frame(app, ctx, vec![]);
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn click(app: &mut Console, ctx: &egui::Context, name: &'static str) {
    frame(app, ctx, vec![]);
    let pos = app.probes[name].center();
    frame(
        app,
        ctx,
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: Default::default(),
            },
        ],
    );
    frame(
        app,
        ctx,
        vec![egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: Default::default(),
        }],
    );
}
fn fixture() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("console.sqlite");
    let mut store = Store::open(&path).unwrap();
    for i in 0..3 {
        let id = DeviceId::new();
        store
            .upsert_device(&Device {
                id,
                host: format!("test-{i}.example.test"),
                aliases: vec![],
                ad_guid: None,
                name: format!("Тест {i}"),
                domain: "example.test".into(),
                ou: "OU=Fixture".into(),
                os: "Windows Fixture".into(),
                source: "SYNTHETIC TEST ONLY".into(),
                site: "lab".into(),
                disabled: false,
                inventory_metadata: serde_json::json!({}),
            })
            .unwrap();
        store.append_scan(&DeviceScan{id:ScanId::new(),device_id:id,scanned_at:chrono::Utc::now(),installed_ram_bytes:if i==2{None}else{Some((i+1)*8*GIB)},software_state:ProviderState::Partial,software:vec![InstalledSoftware{application_id:None,name:"Fixture Chrome".into(),publisher:"Fixture publisher".into(),version:"2.9".into(),channel:None,architecture:None}],details:serde_json::json!({"Services":[{"Name":"fixture"}],"Environment":[{"Name":"NO_REAL_SECRETS"}],"Warnings":["Fixture coverage only"]})}).unwrap();
    }
    drop(store);
    (dir, path)
}
#[test]
fn actual_egui_pointer_create_save_cancel_and_restart() {
    let (_dir, path) = fixture();
    let ctx = egui::Context::default();
    theme::install(&ctx);
    let mut app = Console::default();
    app.connect(path.clone(), false, &ctx);
    wait(&mut app, &ctx, |app| app.preview.is_some());
    click(&mut app, &ctx, "new_group");
    assert!(app.editor.is_some());
    wait(&mut app, &ctx, |app| app.preview.is_some());
    assert_eq!(app.preview.as_ref().unwrap().members.len(), 2);
    assert_eq!(app.preview.as_ref().unwrap().unknown.len(), 1);
    click(&mut app, &ctx, "save_group");
    wait(&mut app, &ctx, |app| app.editor.is_none());
    let mut reopened = Store::open_read_only(&path).unwrap();
    let snapshot = reopened.snapshot().unwrap();
    assert_eq!(snapshot.collections.len(), 1);
    assert_eq!(snapshot.collections[0].revision, 1);
    click(&mut app, &ctx, "new_group");
    click(&mut app, &ctx, "cancel_group");
    assert!(app.editor.is_none());
    assert_eq!(reopened.snapshot().unwrap().collections.len(), 1);
}

#[test]
fn actual_egui_discovery_csv_preview_confirmation_apply_and_backup() {
    let (_dir, path) = fixture();
    let ctx = egui::Context::default();
    theme::install(&ctx);
    let mut app = Console::default();
    app.connect(path.clone(), false, &ctx);
    wait(&mut app, &ctx, |a| a.preview.is_some());
    click(&mut app, &ctx, "add_devices");
    assert!(app.discovery.open);
    frame(
        &mut app,
        &ctx,
        vec![egui::Event::Key {
            key: egui::Key::N,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::CTRL,
        }],
    );
    assert!(app.editor.is_none());
    click(&mut app, &ctx, "discovery_csv");
    click(&mut app, &ctx, "discovery_text");
    frame(
        &mut app,
        &ctx,
        vec![egui::Event::Text(
            "host,name,site\nnew-ui,Учебный ПК,Офис\nNEW-UI,Учебный ПК,Офис".into(),
        )],
    );
    click(&mut app, &ctx, "discovery_prepare");
    wait(&mut app, &ctx, |a| {
        a.discovery.preview.is_some() || a.discovery.error.is_some()
    });
    assert!(app.discovery.error.is_none(), "{:?}", app.discovery.error);
    assert_eq!(app.discovery.preview.as_ref().unwrap().changes, 1);
    assert_eq!(
        Store::open_read_only(&path)
            .unwrap()
            .snapshot()
            .unwrap()
            .devices
            .len(),
        3
    );
    for _ in 0..20 {
        frame(&mut app, &ctx, vec![]);
    }
    click(&mut app, &ctx, "discovery_apply");
    assert!(!app.discovery.busy);
    assert!(app.discovery.result.is_none());
    click(&mut app, &ctx, "discovery_approve");
    assert!(app.discovery.approve);
    click(&mut app, &ctx, "discovery_apply");
    wait(&mut app, &ctx, |a| {
        a.discovery.result.is_some() || a.discovery.error.is_some()
    });
    assert!(app.discovery.error.is_none(), "{:?}", app.discovery.error);
    let receipt = app.discovery.result.as_ref().unwrap();
    assert_eq!(receipt.changes, 1);
    let backup = path
        .parent()
        .unwrap()
        .join(receipt.backup.as_ref().unwrap());
    assert!(backup.is_file());
    assert_eq!(
        Store::open_read_only(&backup)
            .unwrap()
            .snapshot()
            .unwrap()
            .devices
            .len(),
        3
    );
    assert_eq!(
        Store::open_read_only(&path)
            .unwrap()
            .snapshot()
            .unwrap()
            .devices
            .len(),
        4
    );
    click(&mut app, &ctx, "discovery_close");
    assert!(!app.discovery.open);
}

#[test]
fn discovery_rejects_stale_responses_and_preserves_ambiguous_apply_for_retry() {
    use orkestr_protocol::*;
    let mut app = Console::default();
    app.discovery.token = 5;
    app.discovery.open = true;
    let prepared = PreparedDiscovery {
        id: uuid::Uuid::new_v4(),
        generation: 1,
        expires_at: chrono::Utc::now() + chrono::Duration::minutes(5),
        rows: vec![],
        changes: 1,
        can_apply: true,
    };
    app.accept(Event::DiscoveryPrepared {
        token: 4,
        preview: prepared.clone(),
    });
    assert!(app.discovery.preview.is_none());
    app.accept(Event::DiscoveryPrepared {
        token: 5,
        preview: prepared,
    });
    app.discovery.apply_attempted = true;
    app.discovery.approve = true;
    app.discovery.busy = true;
    app.accept(Event::Error {
        operation: "discovery apply",
        token: Some(5),
        message: "HTTPS_CONNECTION_FAILED".into(),
    });
    assert!(!app.discovery.busy);
    assert!(app.discovery.apply_attempted);
    assert!(app.discovery.preview.is_some());
    app.accept(Event::Error {
        operation: "discovery apply",
        token: Some(5),
        message: "CONFLICT".into(),
    });
    assert!(!app.discovery.apply_attempted);
    assert!(app.discovery.preview.is_none());
    assert!(!app.discovery.approve);
    let device = DeviceId::new();
    app.selected_device = Some(device);
    app.inventory_status_token = 10;
    app.accept(Event::InventoryStatus {
        token: 9,
        status: DeviceInventoryStatus {
            device_id: device,
            generation: 0,
            latest: None,
            head: None,
        },
    });
    assert!(app.inventory_status.is_none());
    let (_dir, path) = fixture();
    let mut store = Store::open(&path).unwrap();
    let w = Arc::new(orkestr_application::workspace(&mut store).unwrap());
    app.selected_device = Some(w.inventory.devices[0].device.id);
    app.workspace = Some(w.clone());
    app.accept(Event::InventoryStatus {
        token: 10,
        status: DeviceInventoryStatus {
            device_id: app.selected_device.unwrap(),
            generation: w.inventory.generation + 1,
            latest: None,
            head: None,
        },
    });
    assert!(app.inventory_status.is_none());
    assert!(app.inventory_status_error.is_some());
}

#[test]
fn discovery_and_provider_states_render_both_themes_and_all_zoom_levels() {
    use orkestr_protocol::*;
    let (_dir, path) = fixture();
    let mut store = Store::open(&path).unwrap();
    let w = Arc::new(orkestr_application::workspace(&mut store).unwrap());
    let p = orkestr_application::preview(&mut store, &Selection::All).unwrap();
    let device = w.inventory.devices[0].device.id;
    let ctx = egui::Context::default();
    theme::install(&ctx);
    for theme in [theme::ThemeChoice::Light, theme::ThemeChoice::Dark] {
        for zoom in [1., 1.25, 1.5, 2.] {
            let mut app = Console {
                workspace: Some(w.clone()),
                preview: Some(p.clone()),
                selected_device: Some(device),
                device_tab: 9,
                ..Default::default()
            };
            app.preferences.theme = theme;
            app.preferences.zoom = zoom;
            app.inventory_status_for = Some((device, w.inventory.generation));
            app.inventory_status = Some(DeviceInventoryStatus {
                device_id: device,
                generation: w.inventory.generation,
                head: None,
                latest: Some(InventoryRunStatus {
                    id: uuid::Uuid::new_v4(),
                    started_at: chrono::Utc::now(),
                    finished_at: chrono::Utc::now(),
                    state: InventoryState::Failed,
                    providers: vec![ProviderStatus {
                        provider: InventoryProvider::Services,
                        state: ProviderState::Failed,
                        failure: Some(ProviderFailure::AccessDenied),
                        rows: 0,
                    }],
                }),
            });
            assert!(!ctx
                .run(input(vec![], [1280. / zoom, 820. / zoom]), |ctx| app
                    .render(ctx))
                .shapes
                .is_empty());
            app.discovery.open = true;
            app.discovery.text = "host,name\npc,Учебный ПК".into();
            app.discovery.format = DiscoverySource::Csv;
            for prepared in [false, true] {
                if prepared {
                    app.discovery.preview = Some(
                        store
                            .prepare_discovery_session(
                                uuid::Uuid::new_v4(),
                                &PrepareDiscovery {
                                    format: DiscoverySource::Csv,
                                    text: app.discovery.text.clone(),
                                },
                            )
                            .unwrap(),
                    );
                }
                let out = ctx.run(input(vec![], [1280. / zoom, 820. / zoom]), |ctx| {
                    app.render(ctx)
                });
                assert!(!out.shapes.is_empty());
                assert!(app.probes["discovery_close"].is_finite());
                assert!(app.probes["discovery_text"].is_finite());
            }
        }
    }
}
#[test]
fn stale_preview_never_replaces_a_newer_editor_request() {
    let (_dir, path) = fixture();
    let mut store = Store::open(&path).unwrap();
    let w = orkestr_application::workspace(&mut store).unwrap();
    let p = orkestr_application::preview(&mut store, &Selection::All).unwrap();
    let mut app = Console {
        workspace: Some(Arc::new(w)),
        ..Default::default()
    };
    app.changed_preview();
    let old = app.preview_token;
    app.changed_preview();
    app.accept(Event::Preview {
        token: old,
        preview: p.clone(),
    });
    assert!(app.preview.is_none());
    app.accept(Event::Preview {
        token: app.preview_token,
        preview: p,
    });
    assert!(app.preview.is_some());
}
#[test]
fn native_pages_and_editor_render_at_supported_zoom_factors() {
    let (_dir, path) = fixture();
    let mut store = Store::open(&path).unwrap();
    let w = Arc::new(orkestr_application::workspace(&mut store).unwrap());
    let p = orkestr_application::preview(&mut store, &Selection::All).unwrap();
    let ctx = egui::Context::default();
    theme::install(&ctx);
    for dark in [false, true] {
        for zoom in [1., 1.25, 1.5, 2.] {
            let mut app = Console {
                workspace: Some(w.clone()),
                preview: Some(p.clone()),
                ..Default::default()
            };
            app.preferences.theme = if dark {
                theme::ThemeChoice::Dark
            } else {
                theme::ThemeChoice::Light
            };
            app.preferences.zoom = zoom;
            for page in [
                Page::Computers,
                Page::Packages,
                Page::Import,
                Page::Review,
                Page::Settings,
                Page::Help,
            ] {
                app.page = page;
                let out = ctx.run(input(vec![], [1280. / zoom, 820. / zoom]), |ctx| {
                    app.render(ctx)
                });
                assert!(!out.shapes.is_empty());
            }
            app.page = Page::Computers;
            app.selected_device = Some(w.inventory.devices[0].device.id);
            for tab in 0..details::TABS.len() {
                app.device_tab = tab;
                assert!(!ctx
                    .run(input(vec![], [1280. / zoom, 820. / zoom]), |ctx| app
                        .render(ctx))
                    .shapes
                    .is_empty());
            }
            app.editor = Some(GroupEditor::new(None));
            let _ = ctx.run(input(vec![], [1280. / zoom, 820. / zoom]), |ctx| {
                app.render(ctx)
            });
            assert!(app.probes["save_group"].is_finite());
        }
    }
}
#[test]
fn parent_choices_exclude_self_and_descendants_and_preferences_roundtrip() {
    let a = Collection {
        id: CollectionId::new(),
        parent_id: None,
        name: "Root".into(),
        revision: 1,
        kind: CollectionKind::Folder,
    };
    let mut b = a.clone();
    b.id = CollectionId::new();
    b.parent_id = Some(a.id);
    let snapshot = InventorySnapshot {
        generation: 0,
        devices: vec![],
        collections: vec![a.clone(), b.clone()],
    };
    assert!(editor::valid_parents(&snapshot, a.id).is_empty());
    assert_eq!(editor::valid_parents(&snapshot, b.id)[0].0, a.id);
    let prefs = Preferences {
        theme: theme::ThemeChoice::Dark,
        zoom: 1.5,
    };
    let json = serde_json::to_string(&prefs).unwrap();
    let read: Preferences = serde_json::from_str(&json).unwrap();
    assert_eq!(read.theme, prefs.theme);
    assert_eq!(read.zoom, 1.5);
}

#[test]
fn actual_egui_import_dry_run_confirmation_backup_and_apply() {
    let (_dir, path) = fixture();
    let copy = path.with_extension("copy.json");
    let backup = path.with_extension("backup.sqlite");
    std::fs::write(
        &copy,
        r#"{"packages":[{"id":11,"name":"UI import fixture","steps":[]}]}"#,
    )
    .unwrap();
    let before = std::fs::read(&copy).unwrap();
    let ctx = egui::Context::default();
    theme::install(&ctx);
    let mut app = Console::default();
    app.connect(path.clone(), false, &ctx);
    wait(&mut app, &ctx, |app| app.workspace.is_some());
    app.page = Page::Import;
    click(&mut app, &ctx, "import_deploy");
    click(&mut app, &ctx, "import_path");
    frame(
        &mut app,
        &ctx,
        vec![egui::Event::Text(copy.to_string_lossy().into_owned())],
    );
    click(&mut app, &ctx, "import_copy_confirm");
    click(&mut app, &ctx, "import_prepare");
    wait(&mut app, &ctx, |app| app.import.report.is_some());
    assert!(Store::open_read_only(&path)
        .unwrap()
        .packages()
        .unwrap()
        .is_empty());
    click(&mut app, &ctx, "import_approve");
    click(&mut app, &ctx, "import_backup");
    frame(
        &mut app,
        &ctx,
        vec![egui::Event::Text(backup.to_string_lossy().into_owned())],
    );
    click(&mut app, &ctx, "import_apply");
    wait(&mut app, &ctx, |app| {
        app.workspace
            .as_ref()
            .is_some_and(|w| w.packages.len() == 1)
    });
    assert!(Store::open_read_only(&backup)
        .unwrap()
        .packages()
        .unwrap()
        .is_empty());
    assert_eq!(std::fs::read(copy).unwrap(), before);
}

#[test]
fn keyboard_shortcuts_open_cancel_focus_and_help() {
    let (_dir, path) = fixture();
    let ctx = egui::Context::default();
    theme::install(&ctx);
    let mut app = Console::default();
    app.connect(path, false, &ctx);
    wait(&mut app, &ctx, |a| a.preview.is_some());
    let key = |key, modifiers| egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    };
    frame(
        &mut app,
        &ctx,
        vec![key(egui::Key::N, egui::Modifiers::CTRL)],
    );
    assert!(app.editor.is_some());
    frame(
        &mut app,
        &ctx,
        vec![key(egui::Key::Escape, egui::Modifiers::NONE)],
    );
    assert!(app.editor.is_none());
    frame(
        &mut app,
        &ctx,
        vec![key(egui::Key::F, egui::Modifiers::CTRL)],
    );
    assert!(ctx.memory(|m| m.focused()).is_some());
    frame(
        &mut app,
        &ctx,
        vec![key(egui::Key::F1, egui::Modifiers::NONE)],
    );
    assert!(app.page == Page::Help);
}
