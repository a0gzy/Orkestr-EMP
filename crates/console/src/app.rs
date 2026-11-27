use crate::{
    deployment_view::DeploymentView,
    details,
    discovery_view::DiscoveryView,
    editor::{self, GroupEditor},
    import_view::ImportView,
    remote::{Remote, Window as RemoteWindow},
    theme::{self, Palette, Preferences},
};
use eframe::egui::{self, RichText};
use orkestr_application::{Command, Event, Selection, Service, Workspace};
use orkestr_domain::*;
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Page {
    Computers,
    Packages,
    Import,
    Review,
    Settings,
    Help,
}
pub struct Console {
    pub(crate) workspace: Option<Arc<Workspace>>,
    service: Option<Service>,
    pub(crate) preferences: Preferences,
    pub(crate) page: Page,
    pub(crate) group: Option<CollectionId>,
    pub(crate) editor: Option<GroupEditor>,
    pub(crate) preview: Option<Preview>,
    pub(crate) preview_error: Option<String>,
    preview_token: u64,
    preview_due: Option<Instant>,
    saving: bool,
    pub(crate) status: String,
    pub(crate) error: Option<String>,
    search: String,
    focus_search: bool,
    last_refresh: Instant,
    selected_device: Option<DeviceId>,
    device_tab: usize,
    selected_package: Option<PackageRevisionId>,
    delete_confirm: Option<(CollectionId, u64, String)>,
    pub(crate) import: ImportView,
    pub(crate) discovery: DiscoveryView,
    inventory_status: Option<orkestr_protocol::DeviceInventoryStatus>,
    inventory_status_error: Option<String>,
    inventory_status_token: u64,
    inventory_status_for: Option<(DeviceId, u64)>,
    path: Option<PathBuf>,
    capture: Option<PathBuf>,
    capture_sent: bool,
    capture_mode: bool,
    capture_ready: Option<Instant>,
    pub(crate) capture_view: String,
    pub(crate) probes: BTreeMap<&'static str, egui::Rect>,
    pub(crate) remote: Remote,
    scan_connection: Option<uuid::Uuid>,
    scan_refreshed: Option<uuid::Uuid>,
    deployment: DeploymentView,
}
impl Default for Console {
    fn default() -> Self {
        Self {
            workspace: None,
            service: None,
            preferences: Preferences::default(),
            page: Page::Computers,
            group: None,
            editor: None,
            preview: None,
            preview_error: None,
            preview_token: 0,
            preview_due: None,
            saving: false,
            status: "Выберите локальную базу. Автоматических подключений нет.".into(),
            error: None,
            search: String::new(),
            focus_search: false,
            last_refresh: Instant::now(),
            selected_device: None,
            device_tab: 0,
            selected_package: None,
            delete_confirm: None,
            import: ImportView::default(),
            discovery: DiscoveryView::default(),
            inventory_status: None,
            inventory_status_error: None,
            inventory_status_token: 0,
            inventory_status_for: None,
            path: None,
            capture: None,
            capture_sent: false,
            capture_mode: false,
            capture_ready: None,
            capture_view: String::new(),
            probes: BTreeMap::new(),
            remote: Remote::default(),
            scan_connection: None,
            scan_refreshed: None,
            deployment: DeploymentView::default(),
        }
    }
}
impl Console {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        database: Option<PathBuf>,
        capture: Option<PathBuf>,
    ) -> Self {
        theme::install(&cc.egui_ctx);
        let preferences = cc
            .storage
            .and_then(|s| eframe::get_value(s, "orkestr-preferences"))
            .unwrap_or_default();
        let capture_mode = capture.is_some();
        let mut app = Self {
            preferences,
            capture,
            capture_mode,
            ..Self::default()
        };
        if let Some(path) = database {
            app.connect(path, false, &cc.egui_ctx);
        }
        app
    }
    fn connect(&mut self, path: PathBuf, create: bool, ctx: &egui::Context) {
        self.remote = Remote::default();
        self.selected_device = None;
        self.discovery = DiscoveryView::default();
        self.inventory_status = None;
        self.inventory_status_for = None;
        self.inventory_status_error = None;
        self.inventory_status_token = self.inventory_status_token.wrapping_add(1);
        let repaint = ctx.clone();
        self.service = Some(Service::start(
            path.clone(),
            create,
            Arc::new(move || repaint.request_repaint()),
        ));
        self.path = Some(path);
        self.workspace = None;
        self.editor = None;
        self.preview = None;
        self.error = None;
        self.group = None;
        self.status = "Открытие локальной базы…".into();
    }
    pub(crate) fn submit(&mut self, command: Command) -> bool {
        if !self.can_manage_devices()
            && matches!(
                &command,
                Command::PrepareDiscovery { .. }
                    | Command::ApplyDiscovery { .. }
                    | Command::ReadDiscoveryFile { .. }
            )
        {
            self.error = Some("Изменение реестра компьютеров доступно только Admin.".into());
            return false;
        }
        if self.remote.active
            && !self.remote.can_write()
            && matches!(
                &command,
                Command::Save { .. }
                    | Command::Delete { .. }
                    | Command::PrepareImport { .. }
                    | Command::ApplyImport { .. }
            )
        {
            self.error = Some("Viewer: изменение данных запрещено сервером.".into());
            return false;
        }
        if self
            .service
            .as_ref()
            .is_some_and(|s| s.submit(command).is_ok())
        {
            true
        } else {
            self.error=Some("Локальный обработчик недоступен или очередь занята. Повторите действие; введённые данные сохранены в окне.".into());
            false
        }
    }
    pub(crate) fn can_manage_devices(&self) -> bool {
        !self.remote.active || self.remote.admin()
    }
    pub(crate) fn changed_preview(&mut self) {
        self.preview_token = self.preview_token.wrapping_add(1);
        self.preview = None;
        self.preview_error = None;
        self.preview_due = Some(Instant::now() + Duration::from_millis(180));
    }
    fn poll(&mut self) {
        let mut events = vec![];
        if let Some(service) = &self.service {
            while let Some(event) = service.try_event() {
                events.push(event);
            }
        }
        for event in events {
            self.accept(event);
        }
        let selected = self.selected_device.and_then(|id| {
            self.workspace
                .as_ref()
                .map(|w| (id, w.inventory.generation))
        });
        if selected != self.inventory_status_for {
            self.inventory_status_token = self.inventory_status_token.wrapping_add(1);
            self.inventory_status = None;
            self.inventory_status_error = None;
            if let Some((device_id, _)) = selected {
                if self.submit(Command::InventoryStatus {
                    token: self.inventory_status_token,
                    device_id,
                }) {
                    self.inventory_status_for = selected;
                }
            } else {
                self.inventory_status_for = None;
            }
        }
        if self.preview_due.is_some_and(|due| Instant::now() >= due) && self.workspace.is_some() {
            let selection = if let Some(editor) = &self.editor {
                Selection::Draft(Box::new(editor.draft.clone()))
            } else {
                self.group.map_or(Selection::All, Selection::Group)
            };
            if self.submit(Command::Preview {
                token: self.preview_token,
                selection,
            }) {
                self.preview_due = None;
            }
        }
    }
    pub(crate) fn accept(&mut self, event: Event) {
        match event {
            Event::DiscoveryText { token, text } if token == self.discovery.token => {
                self.discovery.text = text;
                self.discovery.busy = false;
                self.discovery.error = None;
            }
            Event::DiscoveryPrepared { token, preview } if token == self.discovery.token => {
                self.discovery.preview = Some(preview);
                self.discovery.reveal_preview = true;
                self.discovery.busy = false;
                self.discovery.error = None;
            }
            Event::DiscoveryApplied { token, result } if token == self.discovery.token => {
                self.discovery.result = Some(result);
                self.discovery.busy = false;
                self.discovery.error = None;
                self.status = "Реестр обновлён. Сканирование и подключения не запускались.".into();
                self.submit(Command::Refresh);
            }
            Event::InventoryStatus { token, status }
                if token == self.inventory_status_token
                    && Some(status.device_id) == self.selected_device =>
            {
                if self
                    .workspace
                    .as_ref()
                    .is_some_and(|w| w.inventory.generation != status.generation)
                {
                    self.inventory_status = None;
                    self.inventory_status_error=Some("Реестр изменился. Обновление снимка; новые статусы не смешиваются со старыми данными.".into());
                    self.submit(Command::Refresh);
                } else {
                    self.inventory_status = Some(status);
                    self.inventory_status_error = None;
                }
            }
            Event::DiscoveryText { .. }
            | Event::DiscoveryPrepared { .. }
            | Event::DiscoveryApplied { .. }
            | Event::InventoryStatus { .. } => {}
            Event::Loaded(workspace) => {
                self.last_refresh = Instant::now();
                self.workspace = Some(workspace);
                self.status = if self.remote.active {
                    "HTTPS · общий сервер"
                } else {
                    "Локальная база открыта · выполнение отсутствует"
                }
                .into();
                self.changed_preview();
            }
            Event::Preview { token, preview } if token == self.preview_token => {
                if self
                    .workspace
                    .as_ref()
                    .is_some_and(|w| w.inventory.generation != preview.generation)
                {
                    self.submit(Command::Refresh);
                    self.changed_preview();
                } else {
                    self.preview = Some(preview);
                    self.preview_error = None;
                }
            }
            Event::Preview { .. } => {}
            Event::Saved { id, workspace } => {
                self.saving = false;
                self.workspace = Some(workspace);
                self.editor = None;
                self.group = Some(id);
                self.status = "Группа сохранена в SQLite".into();
                self.error = None;
                self.changed_preview();
            }
            Event::Deleted(workspace) => {
                self.workspace = Some(workspace);
                self.group = None;
                self.delete_confirm = None;
                self.status = "Группа удалена. ПК и сканы сохранены.".into();
                self.error = None;
                self.changed_preview();
            }
            Event::ImportPrepared { token, report } if token == self.import.token => {
                self.import.busy = false;
                self.import.report = Some(report);
            }
            Event::ImportPrepared { .. } => {}
            Event::Imported { outcome, workspace } => {
                self.workspace = Some(workspace);
                self.import.busy = false;
                self.import.report = None;
                self.import.approve = false;
                self.error = None;
                self.status =
                    format!("Импорт: {outcome:?}. Автоматика не включена. Backup сохранён.");
                self.changed_preview();
            }
            Event::Error {
                operation,
                token,
                message,
            } => {
                if operation == "preview" {
                    if token == Some(self.preview_token) {
                        self.preview_error = Some(message);
                        self.preview = None;
                    }
                } else if operation.starts_with("discovery") {
                    if token == Some(self.discovery.token) {
                        self.discovery.busy = false;
                        if message.contains("CONFLICT")
                            || message.contains("PREVIEW_UNAVAILABLE")
                            || message.contains("Edit conflict")
                            || message.contains("preview unavailable")
                        {
                            self.discovery.apply_attempted = false;
                            self.discovery.approve = false;
                            self.discovery.preview = None;
                        }
                        self.discovery.error = Some(message);
                    }
                } else if operation == "inventory status" {
                    if token == Some(self.inventory_status_token) {
                        self.inventory_status_error = Some(message);
                    }
                } else if operation.starts_with("import") {
                    if token == Some(self.import.token) {
                        self.import.busy = false;
                        self.error = Some(format!("Импорт: {message}"));
                    }
                } else {
                    self.saving = false;
                    self.error = Some(format!("{operation}: {message}"));
                }
            }
        }
    }
    pub fn render(&mut self, ctx: &egui::Context) {
        self.remote.poll();
        if let Some(profile) = self.remote.configured_scan.take() {
            self.scan_connection = Some(profile);
        }
        if let Some(revision) = self.remote.registered_msi.take() {
            self.selected_package = Some(revision);
            self.submit(Command::Refresh);
        }
        if let Some(job) = self.remote.inventory_job.as_ref() {
            if self.scan_refreshed != Some(job.id)
                && matches!(job.task, orkestr_protocol::InventoryTask::ScanDevice { .. })
                && !matches!(
                    job.state,
                    orkestr_protocol::JobState::Queued | orkestr_protocol::JobState::Running
                )
            {
                self.scan_refreshed = Some(job.id);
                self.inventory_status_for = None;
                self.submit(Command::Refresh);
            }
        }
        if let Some(service) = self.remote.ready.take() {
            self.discovery = DiscoveryView::default();
            self.selected_device = None;
            self.inventory_status = None;
            self.inventory_status_for = None;
            self.inventory_status_token = self.inventory_status_token.wrapping_add(1);
            self.service = Some(service);
            self.workspace = None;
            self.preview = None;
            self.editor = None;
            self.group = None;
            self.path = None;
            self.error = None;
            self.status = "Загрузка реестра по HTTPS…".into();
        }
        ctx.set_theme(self.preferences.theme.preference());
        ctx.set_zoom_factor(self.preferences.zoom.clamp(0.8, 2.));
        self.poll();
        if self.workspace.is_some()
            && self.editor.is_none()
            && !self.import.busy
            && !self.discovery.busy
            && !self.saving
            && self.last_refresh.elapsed() >= Duration::from_secs(60)
            && self.submit(Command::Refresh)
        {
            self.last_refresh = Instant::now();
        }
        self.probes.clear();
        if !self.discovery.open
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::F))
        {
            self.focus_search = true;
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::R))
            && self.workspace.is_some()
        {
            self.submit(Command::Refresh);
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::F1)) {
            self.page = Page::Help;
        }
        if self.editor.is_none()
            && !self.discovery.open
            && self.workspace.is_some()
            && self.remote.can_write()
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::N))
        {
            self.new_editor();
        }
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.small(&self.status);
                if let Some(w) = &self.workspace {
                    ui.small(format!(
                        "Снимок #{} · {} ПК",
                        w.inventory.generation,
                        w.inventory.devices.len()
                    ));
                }
            });
        });
        if self.workspace.is_some() {
            egui::SidePanel::left("navigation")
                .resizable(true)
                .default_width(254.)
                .width_range(220.0..=400.0)
                .show(ctx, |ui| {
                    if ui.available_height() < 650. {
                        egui::ScrollArea::vertical()
                            .id_salt("navigation_scroll")
                            .show(ui, |ui| self.sidebar(ui));
                    } else {
                        self.sidebar(ui);
                    }
                });
        }
        egui::CentralPanel::default()
            .frame(
                egui::Frame::central_panel(&ctx.style())
                    .fill(Palette::get(ctx.style().visuals.dark_mode).bg),
            )
            .show(ctx, |ui| {
                if let Some(error) = &self.error {
                    ui.colored_label(Palette::get(ui.visuals().dark_mode).error, error);
                    ui.separator();
                }
                if self.workspace.is_none() {
                    self.startup(ui, ctx);
                } else {
                    match self.page {
                        Page::Computers => self.computers(ui),
                        Page::Packages => self.packages(ui),
                        Page::Import => {
                            if self.remote.active {
                                ui.heading("Импорт копий");
                                ui.label("Remote import закрыт: сервер не читает клиентские paths. Offline-перенос M1 описан в docs/m2-server.md.");
                            } else { egui::ScrollArea::vertical()
                                .id_salt("import_page")
                                .show(ui, |ui| self.import_page(ui)); }
                        }
                        Page::Review => self.review_page(ui),
                        Page::Settings => {
                            egui::ScrollArea::vertical()
                                .id_salt("settings_scroll")
                                .show(ui, |ui| self.settings(ui));
                        }
                        Page::Help => {
                            egui::ScrollArea::vertical()
                                .id_salt("help_scroll")
                                .show(ui, |ui| self.help(ui));
                        }
                    }
                }
            });
        self.editor_modal(ctx);
        self.delete_modal(ctx);
        self.discovery_modal(ctx);
        self.remote.show(ctx);
        if self.preview_due.is_some()
            || self.import.busy
            || self.discovery.busy
            || self.inventory_status_for.is_some()
                && self.inventory_status.is_none()
                && self.inventory_status_error.is_none()
            || self.saving
            || self.workspace.is_none() && self.service.is_some()
        {
            ctx.request_repaint_after(Duration::from_millis(80));
        }
        self.capture_frame(ctx);
    }
    fn startup(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.add_space(60.);
        ui.horizontal(|ui| {
            theme::mark(ui);
            ui.heading("Orkestr EMP");
        });
        ui.add_space(24.);
        ui.heading("Один реестр. Без автоматического исполнения.");
        ui.label("Локальный реестр M1 / общий сервер M2. Выберите режим подключения.");
        ui.label("Локальный режим M1 или общий сервер M2 по HTTPS. Не открывайте SQLite через сетевую папку.");
        if ui.button("Подключиться к серверу…").clicked() {
            self.remote.window = Some(RemoteWindow::Server);
        }
        ui.add_space(24.);
        ui.horizontal(|ui| {
            if ui.button("Открыть базу…").clicked() {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("Orkestr SQLite", &["sqlite"])
                    .pick_file()
                {
                    self.connect(path, false, ctx);
                }
            }
            if ui.button("Создать пустую базу…").clicked() {
                if let Some(path) = rfd::FileDialog::new()
                    .set_file_name("orkestr.sqlite")
                    .add_filter("SQLite", &["sqlite"])
                    .save_file()
                {
                    self.connect(path, true, ctx);
                }
            }
        });
        ui.add_space(16.);
        ui.small("Учебные данные не загружаются автоматически. Секреты — только в отдельном окне подключённого M2-сервера, не в локальном M1.");
        if self.service.is_some() && self.error.is_none() {
            ui.spinner();
        }
    }
    fn sidebar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            theme::mark(ui);
            ui.label(RichText::new("Orkestr").size(23.).strong());
            theme::muted(ui, "EMP");
        });
        ui.label(
            RichText::new(if self.remote.active {
                "Сервер · HTTPS"
            } else {
                "Локально · SQLite"
            })
            .small()
            .color(Palette::get(ui.visuals().dark_mode).muted),
        );
        ui.add_space(16.);
        for (page, label) in [
            (Page::Computers, "Компьютеры"),
            (Page::Packages, "Пакеты"),
            (Page::Import, "Импорт копий"),
            (Page::Review, "Требует проверки"),
        ] {
            if ui
                .add_sized(
                    [ui.available_width(), 32.],
                    egui::Button::new(label).selected(self.page == page),
                )
                .clicked()
            {
                self.page = page;
            }
        }
        ui.add_space(16.);
        ui.separator();
        ui.horizontal(|ui| {
            ui.strong("Группы компьютеров");
            let r = ui.add_enabled(
                self.remote.can_write(),
                egui::Button::new("+ Создать").small(),
            );
            self.probes.insert("new_group", r.rect);
            if r.clicked() {
                self.new_editor();
            }
        });
        if ui
            .selectable_label(self.group.is_none(), "Все компьютеры")
            .clicked()
        {
            self.group = None;
            self.page = Page::Computers;
            self.changed_preview();
        }
        let w = self.workspace.as_ref().unwrap().clone();
        let paths = w.inventory.paths().unwrap_or_default();
        let mut rows: Vec<_> = w.inventory.collections.iter().collect();
        rows.sort_by_key(|c| paths.get(&c.id).cloned().unwrap_or_default());
        egui::ScrollArea::vertical()
            .id_salt("group_tree")
            .max_height((ui.available_height() - 140.).clamp(80., 380.))
            .show(ui, |ui| {
                for c in rows {
                    let path = paths.get(&c.id).map_or(c.name.as_str(), String::as_str);
                    let depth = path.matches(" / ").count().min(6);
                    ui.horizontal(|ui| {
                        ui.add_space(depth as f32 * 12.);
                        let (count, unknown) = w.counts.get(&c.id).copied().unwrap_or_default();
                        theme::group_icon(ui, &c.kind);
                        let label = format!(
                            "{}  {count}{}",
                            short(&c.name, 24),
                            if unknown > 0 {
                                format!(" · ?{unknown}")
                            } else {
                                String::new()
                            }
                        );
                        if ui
                            .selectable_label(self.group == Some(c.id), label)
                            .on_hover_text(path)
                            .clicked()
                        {
                            self.group = Some(c.id);
                            self.page = Page::Computers;
                            self.selected_device = None;
                            self.changed_preview();
                        }
                    });
                }
                if w.inventory.collections.is_empty() {
                    theme::muted(ui, "Групп пока нет. Создайте ручную, умную или папку.");
                }
            });
        ui.add_space(12.);
        ui.separator();
        ui.horizontal(|ui| {
            if ui.button("Настройки").clicked() {
                self.page = Page::Settings;
            }
            if ui.button("Справка · F1").clicked() {
                self.page = Page::Help;
            }
        });
        if ui.button("Обновить · Ctrl+R").clicked() {
            self.submit(Command::Refresh);
        }
    }
    fn computers(&mut self, ui: &mut egui::Ui) {
        let w = self.workspace.as_ref().unwrap().clone();
        let selected = self
            .group
            .and_then(|id| w.inventory.collections.iter().find(|c| c.id == id));
        let title = selected.map_or("Все компьютеры", |c| c.name.as_str());
        ui.horizontal_wrapped(|ui| {
            ui.heading(title);
            let add = ui.add_enabled(
                self.can_manage_devices(),
                egui::Button::new("Добавить компьютеры…"),
            );
            self.probes.insert("add_devices", add.rect);
            if add.clicked() {
                self.discovery.open = true;
            }
            if let Some(c) = selected {
                let edit = ui.add_enabled(
                    self.remote.can_write(),
                    egui::Button::new("Изменить / перенести"),
                );
                self.probes.insert("edit_group", edit.rect);
                if edit.clicked() {
                    self.editor = Some(GroupEditor::edit(c));
                    self.changed_preview();
                }
                if matches!(c.kind, CollectionKind::Folder)
                    && ui
                        .add_enabled(
                            self.remote.can_write(),
                            egui::Button::new("Создать подгруппу"),
                        )
                        .clicked()
                {
                    self.editor = Some(GroupEditor::new(Some(c.id)));
                    self.changed_preview();
                }
                if ui
                    .add_enabled(self.remote.can_write(), egui::Button::new("Удалить…"))
                    .clicked()
                {
                    self.delete_confirm = Some((c.id, c.revision, c.name.clone()));
                }
            }
        });
        if let Some(c) = selected {
            theme::muted(
                ui,
                w.inventory
                    .paths()
                    .unwrap_or_default()
                    .get(&c.id)
                    .map_or_else(|| c.name.clone(), Clone::clone),
            );
        }
        ui.horizontal_wrapped(|ui| {
            ui.label("Поиск ПК / ПО");
            let response = ui.add(egui::TextEdit::singleline(&mut self.search).desired_width(280.));
            if self.focus_search {
                response.request_focus();
                self.focus_search = false;
            }
            if ui.small_button("Сбросить").clicked() {
                self.search.clear();
            }
        });
        ui.add_space(8.);
        if let Some(message) = &self.preview_error {
            ui.colored_label(Palette::get(ui.visuals().dark_mode).error, message);
            return;
        }
        let Some(preview) = &self.preview else {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Пересчёт снимка…");
            });
            return;
        };
        ui.label(format!(
            "{} ПК · {} · снимок #{} · {}",
            preview.members.len(),
            if self.group.is_none() {
                "все устройства, без фильтра".into()
            } else {
                format!("Unknown условий: {}", preview.unknown.len())
            },
            preview.generation,
            preview.evaluated_at.format("%Y-%m-%d %H:%M:%S UTC")
        ));
        let query = self.search.to_lowercase();
        let rows: Vec<_> = preview
            .members
            .iter()
            .filter(|r| search_record(r, &query))
            .collect();
        if !preview.unknown.is_empty() {
            egui::CollapsingHeader::new(format!(
                "Нет достоверных данных: {} ПК",
                preview.unknown.len()
            ))
            .show(ui, |ui| {
                for id in &preview.unknown {
                    if let Some(r) = w.inventory.devices.iter().find(|r| r.device.id == *id) {
                        if ui.link(&r.device.host).clicked() {
                            self.selected_device = Some(*id);
                        }
                    }
                }
            });
        }
        egui::ScrollArea::both().id_salt("device_list").max_height(if self.selected_device.is_some(){220.}else{ui.available_height()-12.}).auto_shrink([false,false]).show(ui,|ui| {
            egui::Grid::new("devices").num_columns(5).striped(true).spacing([24.,12.]).show(ui,|ui| {
                for label in ["Компьютер","Операционная система","RAM · installed","Скан / полнота ПО","Приложений"]{ui.strong(label);}ui.end_row();
                for r in rows {
                    if ui.selectable_label(self.selected_device==Some(r.device.id),&r.device.host).clicked(){self.selected_device=Some(r.device.id);}
                    ui.label(if r.device.os.is_empty(){"Unknown"}else{&r.device.os});ui.monospace(details::ram_label(r.scan.as_ref()));
                    ui.label(r.scan.as_ref().map_or_else(||"Нет скана".into(),|s|format!("{} · {:?}",s.scanned_at.format("%Y-%m-%d %H:%M"),s.software_state)));
                    ui.label(r.scan.as_ref().map_or_else(||"—".into(),|s|s.software.len().to_string()));ui.end_row();
                }
            });
            if preview.members.is_empty(){ui.add_space(24.);ui.heading("Компьютеров в выборке пока нет");ui.label("Импортируйте свою копию Inventory или проверьте условия и Unknown. Сканирование не запускается.");}
        });
        if let Some(r) = self
            .selected_device
            .and_then(|id| w.inventory.devices.iter().find(|r| r.device.id == id))
        {
            ui.separator();
            ui.horizontal(|ui| {
                ui.strong(format!("Карточка · {}", r.device.host));
                if ui.small_button("Закрыть карточку").clicked() {
                    self.selected_device = None;
                }
                if ui.small_button("Обновить статусы").clicked() {
                    self.inventory_status_for = None;
                }
            });
            if self.remote.admin() && !r.device.disabled {
                self.inventory_scan_controls(ui, &r.device);
            }
            details::device_card(
                ui,
                r,
                &mut self.device_tab,
                self.inventory_status
                    .as_ref()
                    .filter(|s| s.device_id == r.device.id),
                self.inventory_status_error.as_deref(),
            );
        }
    }
    fn inventory_scan_controls(&mut self, ui: &mut egui::Ui, device: &Device) {
        use orkestr_protocol::{InventoryTask, JobState, ProfileKind, StartInventoryJob};
        let profiles: Vec<_> = self
            .remote
            .profiles(ProfileKind::Execution)
            .filter(|p| p.host == device.host)
            .cloned()
            .collect();
        if !profiles.iter().any(|p| Some(p.id) == self.scan_connection) {
            self.scan_connection = (profiles.len() == 1).then(|| profiles[0].id);
        }
        ui.horizontal_wrapped(|ui| {
            if !profiles.is_empty() {
                egui::ComboBox::from_id_salt("inventory_scan_profile")
                    .selected_text(
                        profiles
                            .iter()
                            .find(|p| Some(p.id) == self.scan_connection)
                            .map_or_else(
                                || "Учётная запись WinRM".to_owned(),
                                |p| self.remote.profile_label(p),
                            ),
                    )
                    .show_ui(ui, |ui| {
                        for profile in &profiles {
                            ui.selectable_value(
                                &mut self.scan_connection,
                                Some(profile.id),
                                self.remote.profile_label(profile),
                            );
                        }
                    });
                if ui
                    .add_enabled(
                        self.scan_connection.is_some() && !self.remote.inventory_busy(),
                        egui::Button::new("Сканировать через WinRM"),
                    )
                    .clicked()
                {
                    if let Some(profile) =
                        profiles.iter().find(|p| Some(p.id) == self.scan_connection)
                    {
                        self.remote.start_inventory(StartInventoryJob {
                            connection_id: profile.id,
                            expected_revision: profile.revision,
                            task: InventoryTask::ScanDevice {
                                device_id: device.id,
                            },
                        });
                    }
                }
            }
            if ui.button("Выбрать учётную запись…").clicked() {
                let domain = device
                    .host
                    .split_once('.')
                    .map(|(_, domain)| domain)
                    .filter(|domain| {
                        device.domain.is_empty() || device.domain.eq_ignore_ascii_case(domain)
                    });
                self.remote
                    .open_setup(ProfileKind::Execution, Some((&device.host, domain)));
            }
            if ui.small_button("Обновить профили").clicked() {
                self.remote.reload_profiles();
            }
        });
        if let Some(job) = self.remote.inventory_job.as_ref().filter(|j| {
            j.task
                == (InventoryTask::ScanDevice {
                    device_id: device.id,
                })
        }) {
            ui.label(format!("Скан {} · {:?} · {}", job.id, job.state, job.code));
            if let Some(explanation)=match job.code.as_str() {
                "INVENTORY_WINRM_TLS_FAILED"=>Some("ПК отвечает на 5986, но WinRM HTTPS не прошёл проверку TLS. Проверьте привязку сертификата и доверие к его издателю."),
                "INVENTORY_WINRM_UNREACHABLE"=>Some("Сервер не смог подключиться к порту WinRM HTTPS 5986 на ПК."),
                "INVENTORY_WINRM_ACCESS_DENIED"=>Some("WinRM отказал в доступе. Проверьте логин, пароль и права на этом ПК."),
                "INVENTORY_WINRM_SESSION_FAILED"=>Some("TLS прошёл, но Windows не открыла CIM-сессию. Проверьте режим аутентификации и права на WinRM."),
                _=>None,
            } { ui.label(explanation); }
            if matches!(job.state, JobState::Queued | JobState::Running)
                && ui.small_button("Обновить состояние скана").clicked()
            {
                self.remote.poll_inventory_job();
            }
        }
        if let Some(error) = &self.remote.inventory_error {
            ui.label(error);
        }
    }
    fn packages(&mut self, ui: &mut egui::Ui) {
        ui.heading("Пакеты");
        ui.label("Сохранённые ревизии пакетов.");
        ui.separator();
        let w = self.workspace.as_ref().unwrap().clone();
        egui::ScrollArea::vertical().id_salt("packages").show(ui,|ui|{
            for p in &w.packages {if ui.selectable_label(self.selected_package==Some(p.id),format!("{} · {} · {} шагов",p.name,p.version,p.steps.len())).clicked(){self.selected_package=Some(p.id);}}
            if w.packages.is_empty(){ui.label("Пакетов нет. Импортируйте выбранную копию console.json в разделе «Импорт копий».");}
            if let Some(p)=w.packages.iter().find(|p|Some(p.id)==self.selected_package){ui.separator();ui.heading(&p.name);ui.monospace(format!("PackageId: {}",p.package_id));ui.monospace(format!("Revision: {}",p.id));
                    if p.artifact_hashes.is_empty() { ui.label("Импортированное определение ждёт проверки."); }
                    egui::CollapsingHeader::new("Описание и исходная папка").show(ui,|ui|details::value_ui(ui,&p.metadata,0));
                for (i,step) in p.steps.iter().enumerate(){egui::CollapsingHeader::new(format!("Шаг {} · {:?}",i+1,step.kind)).show(ui,|ui|{details::value_ui(ui,&step.definition,0);});}
            }
            if self.remote.admin() { self.deployment.show(ui,&mut self.remote,&w,self.selected_package); }
        });
    }
    pub(crate) fn new_editor(&mut self) {
        if !self.remote.can_write() {
            return;
        }
        self.page = Page::Computers;
        let parent = self.group.filter(|id| {
            self.workspace.as_ref().is_some_and(|w| {
                w.inventory
                    .collections
                    .iter()
                    .any(|c| c.id == *id && matches!(c.kind, CollectionKind::Folder))
            })
        });
        self.editor = Some(GroupEditor::new(parent));
        self.changed_preview();
    }
    fn editor_modal(&mut self, ctx: &egui::Context) {
        let Some(mut editor) = self.editor.take() else {
            return;
        };
        let Some(w) = self.workspace.clone() else {
            return;
        };
        let before = editor.draft.clone();
        let mut save = false;
        let mut close = false;
        egui::Modal::new(egui::Id::new("group_editor")).show(ctx, |ui| {
            ui.set_width((ctx.screen_rect().width() - 64.).clamp(240., 760.));
            ui.heading(if editor.expected.is_some() {
                "Изменить группу"
            } else {
                "Создать группу"
            });
            egui::ScrollArea::vertical()
                .max_height((ctx.screen_rect().height() - 220.).max(180.))
                .id_salt("editor_scroll")
                .show(ui, |ui| {
                    ui.add_enabled_ui(!self.saving, |ui| {
                        editor::group_form(ui, &mut editor, &w.inventory)
                    });
                    ui.add_space(16.);
                    ui.separator();
                    ui.strong("Предпросмотр до сохранения");
                    if before != editor.draft {
                        ui.label("Правила изменены. Пересчёт…");
                    } else if let Some(error) = &self.preview_error {
                        ui.colored_label(Palette::get(ui.visuals().dark_mode).error, error);
                    } else if let Some(p) = &self.preview {
                        ui.label(format!(
                            "{} ПК · Unknown {} · снимок #{}",
                            p.members.len(),
                            p.unknown.len(),
                            p.generation
                        ));
                        for r in &p.members {
                            ui.collapsing(
                                format!(
                                    "{} · {} · {} записей ПО",
                                    r.device.host,
                                    details::ram_label(r.scan.as_ref()),
                                    r.scan.as_ref().map_or(0, |s| s.software.len())
                                ),
                                |ui| {
                                    if let Some(s) = &r.scan {
                                        details::software_table(ui, &s.software);
                                        ui.small(format!(
                                            "Скан: {} · {:?}",
                                            s.scanned_at, s.software_state
                                        ));
                                    }
                                },
                            );
                        }
                        for id in &p.unknown {
                            if let Some(r) = w.inventory.devices.iter().find(|r| r.device.id == *id)
                            {
                                theme::muted(ui, format!("Unknown · {}", r.device.host));
                            }
                        }
                    } else {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(if self.remote.active {
                                "Вычисление на сервере…"
                            } else {
                                "Вычисление на локальном обработчике…"
                            });
                        });
                    }
                });
            if let Some(error) = &self.error {
                ui.colored_label(Palette::get(ui.visuals().dark_mode).error, error);
            }
            ui.separator();
            ui.horizontal(|ui| {
                let valid = w.inventory.validate_collection(&editor.draft).is_ok()
                    && before == editor.draft
                    && self.preview.is_some()
                    && self.preview_error.is_none();
                let r = ui.add_enabled(
                    !self.saving && valid && self.remote.can_write(),
                    egui::Button::new("Сохранить группу"),
                );
                self.probes.insert("save_group", r.rect);
                if r.clicked() {
                    save = true;
                }
                let r = ui.add_enabled(!self.saving, egui::Button::new("Отмена"));
                self.probes.insert("cancel_group", r.rect);
                if r.clicked() {
                    close = true;
                }
                if self.saving {
                    ui.spinner();
                }
            });
            if !self.saving
                && self
                    .error
                    .as_ref()
                    .is_some_and(|e| e.contains("Edit conflict") || e.contains("CONFLICT"))
                && ui.button("Сохранить правки как новую группу").clicked()
            {
                editor.draft.id = CollectionId::new();
                editor.draft.revision = 0;
                editor.expected = None;
                self.error = None;
            }
        });
        if !self.saving
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            close = true;
        }
        if before != editor.draft {
            self.changed_preview();
        }
        if close {
            self.changed_preview();
            self.error = None;
        } else {
            if save
                && self.submit(Command::Save {
                    collection: Box::new(editor.draft.clone()),
                    expected: editor.expected,
                })
            {
                self.saving = true;
            }
            self.editor = Some(editor);
        }
    }
    fn delete_modal(&mut self, ctx: &egui::Context) {
        if let Some((id, rev, name)) = self.delete_confirm.clone() {
            egui::Modal::new(egui::Id::new("delete_group")).show(ctx,|ui|{
            ui.heading("Удалить группу?");ui.label(name);ui.label("Компьютеры и инвентаризация не удаляются. Папку с детьми сначала нужно освободить.");
                ui.horizontal(|ui| {
                    if ui.button("Удалить группу").clicked() {
                        self.submit(Command::Delete{id,expected:rev});
                        self.delete_confirm=None;
                    }
                    if ui.button("Отмена").clicked() {
                        self.delete_confirm=None;
                    }
                });
        });
        }
    }
    fn settings(&mut self, ui: &mut egui::Ui) {
        if ui.button("Подключение к серверу…").clicked() {
            self.remote.window = Some(RemoteWindow::Server);
        }
        if let Some(actor) = &self.remote.actor {
            ui.label(format!(
                "{} · {:?} · токен до {}",
                actor.label,
                actor.role,
                actor.expires_at.format("%Y-%m-%d")
            ));
        }
        if self.remote.active {
            if ui
                .add_enabled(
                    self.remote.admin(),
                    egui::Button::new("Добавить подключение к домену или ПК…"),
                )
                .clicked()
            {
                self.remote
                    .open_setup(orkestr_protocol::ProfileKind::Execution, None);
            }
            egui::CollapsingHeader::new("Расширенные настройки сервера").show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .add_enabled(self.remote.admin(), egui::Button::new("Учётные данные…"))
                        .clicked()
                    {
                        self.remote.window = Some(RemoteWindow::Credentials);
                    }
                    if ui
                        .add_enabled(
                            self.remote.admin(),
                            egui::Button::new("Подключения к ПК / AD…"),
                        )
                        .clicked()
                    {
                        self.remote.window = Some(RemoteWindow::Connections);
                    }
                    if ui.button("Операторы и аудит…").clicked() {
                        self.remote.window = Some(RemoteWindow::Operators);
                    }
                });
            });
        }
        ui.heading("Настройки консоли");
        ui.add_space(16.);
        ui.horizontal_wrapped(|ui| {
            ui.label("Тема");
            for theme in [
                theme::ThemeChoice::Light,
                theme::ThemeChoice::Dark,
                theme::ThemeChoice::System,
            ] {
                ui.selectable_value(&mut self.preferences.theme, theme, theme.label());
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.label("Масштаб");
            for scale in [1., 1.25, 1.5, 2.] {
                ui.selectable_value(
                    &mut self.preferences.zoom,
                    scale,
                    format!("{}%", (scale * 100.) as u32),
                );
            }
        });
        ui.separator();
        if let Some(path) = &self.path {
            ui.label("Локальная база (не SMB)");
            ui.monospace(path.display().to_string());
        }
    }
    fn help(&self, ui: &mut egui::Ui) {
        ui.heading("Справка · Orkestr EMP");
        ui.label("M2: «Подключиться к серверу…» — HTTPS origin, персональный token и проверенный CA/certificate. Настройки → отдельные окна credentials, connections, operators/audit. Viewer не может изменять данные или создавать задания.");
        ui.label("1. Создайте пустую локальную базу или откройте существующую базу Orkestr.");
        ui.label("2. «Компьютеры → Добавить компьютеры…»: ручной список или UTF-8 CSV (host,name,site). Предпросмотр → подтверждение → apply с backup. Это только реестр, не подключение/скан. В remote mode нужен Admin; путь клиентского файла серверу не отправляется.");
        ui.label("3. В «Импорт копий» выберите закрытую копию Inventory (VACUUM INTO) или Deploy JSON. Проверьте dry-run и сохраните новый rollback backup при применении.");
        ui.label("4. Создавайте папки, ручные или умные группы. RAM измеряется в точных installed bytes; 1 GiB = 2³⁰ байт. Unknown показан отдельно.");
        ui.label("5. Карточка ПК → «Провайдеры»: последняя попытка отдельно от сохранённого снимка. Viewer видит только безопасные состояния/коды, не raw evidence. Все прежние подробные разделы сохранены.");
        ui.add_space(16.);
        ui.label("Ctrl+N — создать группу · Ctrl+F — поиск · Ctrl+R — обновить · Escape — отменить редактор · F1 — справка.");
        ui.separator();
        ui.strong("Ничего не устанавливается и не сканируется");
        ui.label("Deploy/библиотека/одобрения — следующие этапы. M2 запускает только явно запрошенные read-only connection tests с сервера. Импортированные расписания выключены. Живой эталон из Интернета недоступен до M4.");
    }
    fn capture_frame(&mut self, ctx: &egui::Context) {
        if self.capture.is_none() {
            return;
        }
        if self.workspace.is_some() && self.preview.is_some() && self.capture_ready.is_none() {
            match self.capture_view.as_str() {
                "editor" => self.new_editor(),
                "device" => {
                    self.selected_device = self
                        .workspace
                        .as_ref()
                        .unwrap()
                        .inventory
                        .devices
                        .first()
                        .map(|r| r.device.id);
                }
                "packages" => {
                    self.page = Page::Packages;
                    self.selected_package = self
                        .workspace
                        .as_ref()
                        .unwrap()
                        .packages
                        .first()
                        .map(|p| p.id);
                }
                "settings" => self.page = Page::Settings,
                "review" => self.page = Page::Review,
                "group" => {
                    self.group = self
                        .workspace
                        .as_ref()
                        .unwrap()
                        .inventory
                        .collections
                        .iter()
                        .find(|c| matches!(c.kind, CollectionKind::Dynamic(_)))
                        .map(|c| c.id);
                    self.changed_preview();
                }
                "import" => self.page = Page::Import,
                _ => {}
            }
            self.capture_ready = Some(Instant::now() + Duration::from_millis(600));
        }
        if !self.capture_sent && self.capture_ready.is_some_and(|t| Instant::now() >= t) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
            self.capture_sent = true;
        }
        let images = ctx.input(|i| {
            i.events
                .iter()
                .filter_map(|e| {
                    if let egui::Event::Screenshot { image, .. } = e {
                        Some(image.clone())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
        });
        for image in images {
            if let Some(path) = self.capture.take() {
                use image::ImageEncoder;
                let result = (|| -> std::result::Result<(), Box<dyn std::error::Error>> {
                    let file = std::fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(path)?;
                    let rgba: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
                    image::codecs::png::PngEncoder::new(file).write_image(
                        &rgba,
                        image.width() as u32,
                        image.height() as u32,
                        image::ExtendedColorType::Rgba8,
                    )?;
                    Ok(())
                })();
                if result.is_err() {
                    eprintln!("Screenshot could not be saved (no overwrite).");
                }
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
        ctx.request_repaint_after(Duration::from_millis(80));
    }
}
impl eframe::App for Console {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.render(ctx);
    }
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        if !self.capture_mode {
            eframe::set_value(storage, "orkestr-preferences", &self.preferences);
        }
    }
}
pub(crate) fn short(raw: &str, limit: usize) -> String {
    let mut s: String = raw
        .chars()
        .filter(|c| !c.is_control())
        .take(limit)
        .collect();
    if raw.chars().count() > limit {
        s.push('…');
    }
    s
}
fn search_record(r: &DeviceRecord, q: &str) -> bool {
    q.is_empty()
        || [
            &r.device.host,
            &r.device.name,
            &r.device.os,
            &r.device.domain,
        ]
        .iter()
        .any(|v| v.to_lowercase().contains(q))
        || r.scan
            .as_ref()
            .is_some_and(|s| s.software.iter().any(|a| a.name.to_lowercase().contains(q)))
}

#[cfg(test)]
mod tests;
