//! All HTTP and DPAPI operations run off the frame thread. Passwords are not preferences.
use eframe::egui;
use orkestr_application::Service;
use orkestr_client::Client;
use orkestr_protocol::*;
use std::{
    path::PathBuf,
    sync::{
        mpsc::{self, Receiver, SyncSender},
        Arc,
    },
};
use uuid::Uuid;
use zeroize::{Zeroize, Zeroizing};

enum Request {
    LoadProfiles,
    LoadAudit,
    Credential(SaveCredential),
    Connection(SaveConnection),
    Configure(ConfigureConnection),
    Issue(IssueToken),
    Revoke(Uuid),
    Test(TestConnection, String),
    Job(Uuid),
    StartInventory(StartInventoryJob, String),
    PollInventory(Uuid),
    Deployments,
    RegisterMsi(ManagedInstaller),
    PlanDeployment(PlanDeployment, String),
    ChangeDeployment(Uuid, &'static str, DeploymentChange),
    SaveLocalToken(PathBuf, Zeroizing<String>),
}
enum Reply {
    Connected(Actor, Service),
    Profiles(Vec<CredentialProfile>, Vec<ConnectionProfile>),
    Audit(Vec<Actor>, Events),
    CredentialSaved(CredentialProfile),
    Configured(Uuid, ConfiguredConnection),
    Issued(IssuedToken),
    Job(Job),
    Inventory(Result<Job, String>),
    Deployment(Result<DeploymentReply, String>),
    ConnectionSaved(ConnectionProfile),
    Done,
    Error(String),
}
enum DeploymentReply {
    List(Vec<Deployment>),
    Record(Box<Deployment>),
    Registered(orkestr_domain::PackageRevisionId),
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Window {
    Server,
    Setup,
    Credentials,
    Connections,
    Operators,
}
struct SetupDraft {
    id: Uuid,
    host: String,
    kind: ProfileKind,
    fixed_target: bool,
    manual_controller: bool,
    target_domain: Option<String>,
    username: String,
    password: Zeroizing<String>,
    winrm_authentication: WinRmAuthentication,
    saved: bool,
    reuse_credential: Option<Uuid>,
}
impl Default for SetupDraft {
    fn default() -> Self {
        Self {
            id: Uuid::new_v4(),
            host: String::new(),
            kind: ProfileKind::Execution,
            fixed_target: false,
            manual_controller: false,
            target_domain: None,
            username: String::new(),
            password: Zeroizing::new(String::new()),
            winrm_authentication: WinRmAuthentication::Negotiate,
            saved: false,
            reuse_credential: None,
        }
    }
}
pub struct Remote {
    pub active: bool,
    pub actor: Option<Actor>,
    pub window: Option<Window>,
    pub ready: Option<Service>,
    origin: String,
    token: Zeroizing<String>,
    token_file: Option<PathBuf>,
    ca: Option<PathBuf>,
    sender: Option<SyncSender<Request>>,
    receiver: Option<Receiver<Reply>>,
    busy: bool,
    message: String,
    error: Option<String>,
    credentials: Vec<CredentialProfile>,
    connections: Vec<ConnectionProfile>,
    operators: Vec<Actor>,
    events: Vec<AuditEvent>,
    credential_id: Uuid,
    credential_revision: Option<u64>,
    label: String,
    username: String,
    password: Zeroizing<String>,
    target: String,
    kind: ProfileKind,
    connection_id: Uuid,
    connection_revision: Option<u64>,
    connection_label: String,
    connection_host: String,
    winrm_authentication: Option<WinRmAuthentication>,
    selected_credential: Option<Uuid>,
    issued: Option<IssuedToken>,
    issue_label: String,
    issue_role: Role,
    days: u16,
    job: Option<Job>,
    pub(crate) inventory_job: Option<Job>,
    pub(crate) inventory_error: Option<String>,
    inventory_request: Option<(StartInventoryJob, String)>,
    pub(crate) deployments: Vec<Deployment>,
    pub(crate) deployment_error: Option<String>,
    pub(crate) registered_msi: Option<orkestr_domain::PackageRevisionId>,
    pub(crate) planned_deployment: Option<Uuid>,
    deployment_request: Option<(PlanDeployment, String)>,
    test_key: String,
    show_issued: bool,
    setup: SetupDraft,
    pub(crate) configured_scan: Option<Uuid>,
}
impl Default for Remote {
    fn default() -> Self {
        Self {
            active: false,
            actor: None,
            window: None,
            ready: None,
            origin: "https://localhost:7443".into(),
            token: Zeroizing::new(String::new()),
            token_file: None,
            ca: None,
            sender: None,
            receiver: None,
            busy: false,
            message: String::new(),
            error: None,
            credentials: vec![],
            connections: vec![],
            operators: vec![],
            events: vec![],
            credential_id: Uuid::new_v4(),
            credential_revision: None,
            label: String::new(),
            username: String::new(),
            password: Zeroizing::new(String::new()),
            target: String::new(),
            kind: ProfileKind::Execution,
            connection_id: Uuid::new_v4(),
            connection_revision: None,
            connection_label: String::new(),
            connection_host: String::new(),
            winrm_authentication: None,
            selected_credential: None,
            issued: None,
            issue_label: String::new(),
            issue_role: Role::Viewer,
            days: 30,
            job: None,
            inventory_job: None,
            inventory_error: None,
            inventory_request: None,
            deployments: vec![],
            deployment_error: None,
            registered_msi: None,
            planned_deployment: None,
            deployment_request: None,
            test_key: Uuid::new_v4().to_string(),
            show_issued: false,
            setup: SetupDraft::default(),
            configured_scan: None,
        }
    }
}
impl Remote {
    pub fn can_write(&self) -> bool {
        !self.active || self.actor.as_ref().is_some_and(|a| a.role != Role::Viewer)
    }
    pub fn admin(&self) -> bool {
        self.active && self.actor.as_ref().is_some_and(|a| a.role == Role::Admin)
    }
    pub fn poll(&mut self) {
        let mut replies = vec![];
        if let Some(receiver) = &self.receiver {
            while let Ok(reply) = receiver.try_recv() {
                replies.push(reply);
            }
        }
        for reply in replies {
            self.busy = false;
            match reply {
                Reply::Connected(actor, service) => {
                    self.active = true;
                    self.actor = Some(actor);
                    self.ready = Some(service);
                    self.window = None;
                    self.credentials.clear();
                    self.connections.clear();
                    self.operators.clear();
                    self.events.clear();
                    self.credential_id = Uuid::new_v4();
                    self.credential_revision = None;
                    self.label.clear();
                    self.username.clear();
                    self.password.zeroize();
                    self.target.clear();
                    self.connection_id = Uuid::new_v4();
                    self.connection_revision = None;
                    self.connection_label.clear();
                    self.connection_host.clear();
                    self.winrm_authentication = None;
                    self.selected_credential = None;
                    self.issued = None;
                    self.show_issued = false;
                    self.job = None;
                    self.inventory_job = None;
                    self.inventory_error = None;
                    self.inventory_request = None;
                    self.deployments.clear();
                    self.deployment_error = None;
                    self.registered_msi = None;
                    self.planned_deployment = None;
                    self.deployment_request = None;
                    self.test_key = Uuid::new_v4().to_string();
                    self.message = "HTTPS подключён".into();
                    self.setup = SetupDraft::default();
                    self.configured_scan = None;
                    if self.admin() {
                        self.send(Request::LoadProfiles);
                    }
                }
                Reply::Profiles(credentials, connections) => {
                    self.credentials = credentials;
                    self.connections = connections;
                    self.message = "Список учётных данных и подключений обновлён.".into();
                }
                Reply::Audit(operators, events) => {
                    self.operators = operators;
                    self.events = events.events;
                    self.message = if events.resync {
                        "Показаны последние события журнала."
                    } else {
                        "Журнал обновлён."
                    }
                    .into();
                }
                Reply::CredentialSaved(p) => {
                    self.remember_credential(p);
                    self.password.zeroize();
                    self.message = "Учётные данные сохранены.".into();
                }
                Reply::Configured(request_id, configured) => {
                    let current_draft = self.setup.id == request_id;
                    if current_draft {
                        self.setup.host.clone_from(&configured.connection.host);
                        if self.setup.fixed_target
                            && configured.connection.kind == ProfileKind::Execution
                        {
                            self.configured_scan = Some(configured.connection.id);
                        }
                    }
                    self.remember_credential(configured.credential);
                    self.remember_connection(configured.connection);
                    if current_draft {
                        self.setup.password.zeroize();
                        self.setup.saved = true;
                    }
                    self.message = "Подключение сохранено. Можно проверить доступ.".into();
                }
                Reply::Issued(issued) => {
                    self.issued = Some(issued);
                    self.message = "Токен выдан. Значение показывается только в этом окне.".into();
                }
                Reply::Job(job) => {
                    self.message = format!("{:?} · {}", job.state, job.code);
                    self.job = Some(job);
                }
                Reply::Inventory(result) => match result {
                    Ok(job) => {
                        self.inventory_job = Some(job);
                        self.inventory_error = None;
                    }
                    Err(error) => self.inventory_error = Some(error),
                },
                Reply::Deployment(result) => match result {
                    Ok(reply) => {
                        self.deployment_error = None;
                        match reply {
                            DeploymentReply::List(list) => self.deployments = list,
                            DeploymentReply::Record(record) => {
                                if self
                                    .deployment_request
                                    .as_ref()
                                    .is_some_and(|(plan, _)| plan == &record.plan)
                                {
                                    self.deployment_request = None;
                                    self.planned_deployment = Some(record.id);
                                }
                                if let Some(current) =
                                    self.deployments.iter_mut().find(|r| r.id == record.id)
                                {
                                    *current = *record;
                                } else {
                                    self.deployments.insert(0, *record);
                                }
                            }
                            DeploymentReply::Registered(id) => self.registered_msi = Some(id),
                        }
                    }
                    Err(error) => self.deployment_error = Some(error),
                },
                Reply::ConnectionSaved(profile) => {
                    self.remember_connection(profile);
                    self.message = "Подключение сохранено. Можно запустить проверку.".into();
                }
                Reply::Done => {
                    self.message = "Изменение сохранено.".into();
                }
                Reply::Error(error) => {
                    self.error = Some(error);
                }
            }
        }
    }
    fn remember_credential(&mut self, profile: CredentialProfile) {
        self.credential_id = profile.id;
        self.credential_revision = Some(profile.revision);
        self.label.clone_from(&profile.label);
        self.username.clone_from(&profile.username);
        self.target.clone_from(&profile.target);
        self.kind = profile.kind;
        if let Some(entry) = self.credentials.iter_mut().find(|p| p.id == profile.id) {
            *entry = profile;
        } else {
            self.credentials.push(profile);
        }
    }

    fn remember_connection(&mut self, profile: ConnectionProfile) {
        self.connection_id = profile.id;
        self.connection_revision = Some(profile.revision);
        self.connection_host.clone_from(&profile.host);
        self.connection_label.clone_from(&profile.label);
        self.selected_credential = Some(profile.credential_id);
        self.winrm_authentication = profile.winrm_authentication;
        self.test_key = Uuid::new_v4().to_string();
        self.job = None;
        if let Some(entry) = self.connections.iter_mut().find(|p| p.id == profile.id) {
            *entry = profile;
        } else {
            self.connections.push(profile);
        }
    }
    fn send(&mut self, request: Request) {
        if self.busy {
            return;
        }
        if self
            .sender
            .as_ref()
            .is_some_and(|s| s.try_send(request).is_ok())
        {
            self.busy = true;
            self.error = None;
            self.message.clear();
        } else {
            self.error =
                Some("Очередь недоступна. Подключитесь заново; правки не сохранены.".into());
        }
    }

    pub(crate) fn profiles(&self, kind: ProfileKind) -> impl Iterator<Item = &ConnectionProfile> {
        self.connections
            .iter()
            .filter(move |profile| profile.kind == kind)
    }
    pub(crate) fn profile_label(&self, profile: &ConnectionProfile) -> String {
        let account = self
            .credentials
            .iter()
            .find(|c| c.id == profile.credential_id)
            .map_or(profile.label.as_str(), |c| c.username.as_str());
        format!(
            "{account} · {:?}",
            profile.winrm_authentication.unwrap_or_default()
        )
    }

    pub(crate) fn reload_profiles(&mut self) {
        self.send(Request::LoadProfiles);
    }

    pub(crate) fn open_setup(&mut self, kind: ProfileKind, target: Option<(&str, Option<&str>)>) {
        self.window = Some(Window::Setup);
        self.error = None;
        self.message.clear();
        self.job = None;
        self.setup = SetupDraft {
            kind,
            ..SetupDraft::default()
        };
        if let Some((host, domain)) = target {
            self.setup.host = host.to_owned();
            self.setup.fixed_target = true;
            self.setup.target_domain = domain.map(str::to_owned);
        }
    }

    pub(crate) fn inventory_busy(&self) -> bool {
        self.busy
            || self
                .inventory_job
                .as_ref()
                .is_some_and(|j| matches!(j.state, JobState::Queued | JobState::Running))
    }

    pub(crate) fn start_inventory(&mut self, input: StartInventoryJob) -> bool {
        if !self.admin() || self.inventory_busy() {
            return false;
        }
        let key = self
            .inventory_request
            .as_ref()
            .filter(|(previous, _)| self.inventory_job.is_none() && previous == &input)
            .map_or_else(|| Uuid::new_v4().to_string(), |(_, key)| key.clone());
        self.inventory_job = None;
        self.inventory_error = None;
        self.inventory_request = Some((input.clone(), key.clone()));
        self.send(Request::StartInventory(input, key));
        self.busy
    }

    pub(crate) fn poll_inventory_job(&mut self) {
        if let Some(job) = &self.inventory_job {
            self.send(Request::PollInventory(job.id));
        }
    }

    pub(crate) fn busy(&self) -> bool {
        self.busy
    }

    pub(crate) fn load_deployments(&mut self) {
        self.send(Request::Deployments);
    }

    pub(crate) fn register_msi(&mut self, installer: ManagedInstaller) {
        self.send(Request::RegisterMsi(installer));
    }

    pub(crate) fn plan_deployment(&mut self, plan: PlanDeployment) {
        if self.busy {
            return;
        }
        let key = self
            .deployment_request
            .as_ref()
            .filter(|(previous, _)| previous == &plan)
            .map_or_else(|| Uuid::new_v4().to_string(), |(_, key)| key.clone());
        self.deployment_request = Some((plan.clone(), key.clone()));
        self.send(Request::PlanDeployment(plan, key));
    }

    pub(crate) fn change_deployment(
        &mut self,
        id: Uuid,
        action: &'static str,
        expected_revision: u64,
    ) {
        self.send(Request::ChangeDeployment(
            id,
            action,
            DeploymentChange { expected_revision },
        ));
    }
    fn connect(&mut self, ctx: &egui::Context) {
        let origin = self.origin.clone();
        let token = Zeroizing::new(self.token.to_string());
        let token_file = self.token_file.clone();
        let ca = self.ca.clone();
        let (sender, commands) = mpsc::sync_channel(4);
        let (events, receiver) = mpsc::sync_channel(4);
        self.sender = Some(sender);
        self.receiver = Some(receiver);
        self.busy = true;
        self.error = None;
        self.token.zeroize();
        let repaint = ctx.clone();
        std::thread::spawn(move || {
            let send = |reply| {
                let ok = events.send(reply).is_ok();
                repaint.request_repaint();
                ok
            };
            let opened = (|| {
                let token = if let Some(path) = token_file {
                    orkestr_client::load_token(&path)?
                } else {
                    token
                };
                let ca = ca
                    .map(std::fs::read)
                    .transpose()
                    .map_err(|_| "CA_FILE_UNAVAILABLE")?;
                let client =
                    Client::new(&origin, Zeroizing::new(token.to_string()), ca.as_deref())?;
                let actor: Actor = client.get("/v1/me")?;
                let transport = Client::new(&origin, token, ca.as_deref())?;
                let ctx = repaint.clone();
                let service = transport.service(Arc::new(move || ctx.request_repaint()));
                Ok::<_, String>((client, actor, service))
            })();
            let (client, actor, service) = match opened {
                Ok(o) => o,
                Err(e) => {
                    send(Reply::Error(remote_error("Подключение к Orkestr", &e)));
                    return;
                }
            };
            let admin = actor.role == Role::Admin;
            if !send(Reply::Connected(actor, service)) {
                return;
            }
            while let Ok(request) = commands.recv() {
                let context = match &request {
                    Request::LoadProfiles => "Загрузка профилей",
                    Request::LoadAudit => "Загрузка журнала и операторов",
                    Request::Credential(_) => "Сохранение учётных данных",
                    Request::Connection(_) | Request::Configure(_) => "Сохранение подключения",
                    Request::Test(..) | Request::Job(_) => "Проверка доступа",
                    _ => "Запрос к серверу",
                };
                let result = (|| {
                    Ok::<_, String>(match request {
                        Request::LoadProfiles => Reply::Profiles(
                            client.get("/v1/credentials")?,
                            client.get("/v1/connections")?,
                        ),
                        Request::LoadAudit => {
                            let operators = if admin {
                                client.get("/v1/operators")?
                            } else {
                                vec![]
                            };
                            Reply::Audit(operators, client.get("/v1/events?after=0")?)
                        }
                        Request::Credential(p) => {
                            Reply::CredentialSaved(client.post("/v1/credentials", &p, None)?)
                        }
                        Request::Configure(input) => Reply::Configured(
                            input.id,
                            client.post("/v1/connections/setup", &input, None)?,
                        ),
                        Request::Connection(p) => {
                            let path = if p.profile.winrm_authentication.is_some() {
                                "/v1/connections/winrm"
                            } else {
                                "/v1/connections"
                            };
                            Reply::ConnectionSaved(client.post(path, &p, None)?)
                        }
                        Request::Issue(p) => {
                            Reply::Issued(client.post("/v1/operators", &p, None)?)
                        }
                        Request::Revoke(id) => {
                            let _: () =
                                client.post(&format!("/v1/operators/{id}/revoke"), &(), None)?;
                            Reply::Done
                        }
                        Request::Test(profile, key) => {
                            Reply::Job(client.post("/v1/jobs", &profile, Some(&key))?)
                        }
                        Request::Job(id) => Reply::Job(client.get(&format!("/v1/jobs/{id}"))?),
                        Request::StartInventory(input, key) => {
                            Reply::Inventory(client.post("/v1/inventory/jobs", &input, Some(&key)))
                        }
                        Request::PollInventory(id) => {
                            Reply::Inventory(client.get(&format!("/v1/jobs/{id}")))
                        }
                        Request::Deployments => Reply::Deployment(
                            client.get("/v1/deployments").map(DeploymentReply::List),
                        ),
                        Request::RegisterMsi(input) => Reply::Deployment(
                            client
                                .post::<_, orkestr_domain::PackageRevision>(
                                    "/v1/deploy/revisions",
                                    &input,
                                    None,
                                )
                                .map(|r| DeploymentReply::Registered(r.id)),
                        ),
                        Request::PlanDeployment(input, key) => Reply::Deployment(
                            client
                                .post("/v1/deployments", &input, Some(&key))
                                .map(DeploymentReply::Record),
                        ),
                        Request::ChangeDeployment(id, action, change) => Reply::Deployment(
                            client
                                .post(&format!("/v1/deployments/{id}/{action}"), &change, None)
                                .map(DeploymentReply::Record),
                        ),
                        Request::SaveLocalToken(path, token) => {
                            orkestr_client::save_token(&path, &token)?;
                            Reply::Done
                        }
                    })
                })();
                if !send(result.unwrap_or_else(|code| Reply::Error(remote_error(context, &code)))) {
                    break;
                }
            }
        });
    }
    pub fn show(&mut self, ctx: &egui::Context) {
        let Some(window) = self.window else {
            return;
        };
        let title = match window {
            Window::Server => "Подключение к серверу",
            Window::Setup => "Настроить подключение",
            Window::Credentials => "Учётные данные сервера",
            Window::Connections => "Подключения к ПК / AD",
            Window::Operators => "Операторы и аудит",
        };
        let mut open = true;
        egui::Window::new(title)
            .open(&mut open)
            .default_width(620.)
            .resizable(true)
            .vscroll(true)
            .max_height(ctx.screen_rect().height() - 60.)
            .show(ctx, |ui| {
                if let Some(error) = &self.error {
                    ui.colored_label(
                        crate::theme::Palette::get(ui.visuals().dark_mode).error,
                        error,
                    );
                    ui.separator();
                }
                if self.busy {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Сервер обрабатывает запрос…");
                    });
                    ctx.request_repaint_after(std::time::Duration::from_millis(100));
                }
                if !self.message.is_empty() {
                    ui.label(&self.message);
                }
                ui.add_enabled_ui(!self.busy, |ui| match window {
                    Window::Server => self.server_form(ui, ctx),
                    Window::Setup => self.setup_form(ui),
                    Window::Credentials => self.credentials_form(ui),
                    Window::Connections => self.connections_form(ui),
                    Window::Operators => self.operators_form(ui),
                });
            });
        if !open {
            self.window = None;
            self.password.zeroize();
            self.setup.password.zeroize();
            self.token.zeroize();
            self.issued = None;
        }
    }
    fn server_form(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.label("Доступ консоли не зависит от домена. HTTPS с проверкой цепочки и имени; обхода TLS нет.");
        field(ui, "Адрес HTTPS", &mut self.origin);
        ui.label("Персональный токен (не пароль AD)");
        ui.add(
            egui::TextEdit::singleline(&mut *self.token)
                .password(true)
                .desired_width(f32::INFINITY),
        );
        ui.horizontal_wrapped(|ui| {
            if ui.button("Загрузить DPAPI token…").clicked() {
                self.token_file = rfd::FileDialog::new()
                    .add_filter("DPAPI", &["dpapi"])
                    .pick_file();
            }
            if self.token_file.is_some() {
                ui.label("Выбран локальный DPAPI-файл");
                if ui.button("Ввести вручную").clicked() {
                    self.token_file = None;
                }
            }
        });
        ui.horizontal_wrapped(|ui| {
            if ui.button("Доверенный сертификат / CA…").clicked() {
                self.ca = rfd::FileDialog::new()
                    .add_filter("Certificate PEM", &["crt", "pem"])
                    .pick_file();
            }
            if let Some(path) = &self.ca {
                ui.label(path.file_name().unwrap_or_default().to_string_lossy());
            }
        });
        ui.small("Сертификат выбирайте после независимой сверки отпечатка. Без файла используется Windows trust store. Пароль/токен не сохраняется в настройках.");
        if ui.button("Подключиться по HTTPS").clicked() {
            self.connect(ctx);
        }
    }
    fn profile_header(&mut self, ui: &mut egui::Ui) {
        if ui.button("Обновить список").clicked() {
            self.reload_profiles();
        }
        ui.separator();
    }
    fn setup_form(&mut self, ui: &mut egui::Ui) {
        if !self.admin() {
            ui.label("Нужна роль Admin.");
            return;
        }
        if self.setup.saved {
            ui.label(&self.setup.host);
            if let Some(revision) = self.connection_revision {
                if ui.button("Проверить доступ").clicked() {
                    if self.job.as_ref().is_some_and(|job| {
                        !matches!(job.state, JobState::Queued | JobState::Running)
                    }) {
                        self.test_key = Uuid::new_v4().to_string();
                    }
                    self.send(Request::Test(
                        TestConnection {
                            connection_id: self.connection_id,
                            expected_revision: self.winrm_authentication.map(|_| revision),
                        },
                        self.test_key.clone(),
                    ));
                }
            }
            self.show_test_status(ui);
            if ui.button("Готово").clicked() {
                self.window = None;
            }
            return;
        }
        if !self.setup.fixed_target {
            let previous = self.setup.kind;
            ui.horizontal_wrapped(|ui| {
                ui.selectable_value(
                    &mut self.setup.kind,
                    ProfileKind::AdRead,
                    "Найти ПК в домене (AD)",
                );
                ui.selectable_value(
                    &mut self.setup.kind,
                    ProfileKind::Execution,
                    "Сканировать ПК (WinRM)",
                );
            });
            if self.setup.kind != previous {
                self.setup.host.clear();
            }
        }
        if self.setup.kind == ProfileKind::AdRead
            && !self.setup.fixed_target
            && ui
                .checkbox(
                    &mut self.setup.manual_controller,
                    "Указать контроллер вручную",
                )
                .changed()
        {
            self.setup.host.clear();
        }
        let locate_domain_controller =
            self.setup.kind == ProfileKind::AdRead && !self.setup.manual_controller;
        let host_label = if self.setup.kind == ProfileKind::AdRead {
            if self.setup.manual_controller {
                "Полное DNS-имя контроллера"
            } else {
                "DNS-имя домена"
            }
        } else {
            "DNS-имя компьютера"
        };
        let host_valid = if self.setup.fixed_target {
            ui.strong(&self.setup.host);
            true
        } else {
            dns_field(ui, host_label, &mut self.setup.host)
        };
        let valid_domain =
            !locate_domain_controller || self.setup.host.trim().trim_end_matches('.').contains('.');
        if host_valid && !valid_domain {
            required_label(
                ui,
                "Домен",
                Some("укажите полное имя, например corp.example"),
            );
        }
        let target = orkestr_domain::canonical_host(&self.setup.host).ok();
        let eligible: Vec<_> = self
            .credentials
            .iter()
            .filter_map(|p| {
                let same_target = !locate_domain_controller
                    && p.kind == self.setup.kind
                    && target.as_deref() == Some(p.target.as_str());
                let same_domain = self
                    .setup
                    .target_domain
                    .as_deref()
                    .is_some_and(|domain| p.is_domain_account(domain));
                (same_target || same_domain).then_some((p, same_target))
            })
            .collect();
        let previous_account = self.setup.reuse_credential;
        if !eligible.is_empty() {
            if !eligible
                .iter()
                .any(|(p, _)| Some(p.id) == self.setup.reuse_credential)
            {
                self.setup.reuse_credential = None;
            }
            ui.label("Учётная запись");
            egui::ComboBox::from_id_salt("setup_saved_account")
                .selected_text(
                    eligible
                        .iter()
                        .find(|(p, _)| Some(p.id) == self.setup.reuse_credential)
                        .map_or("Ввести новую", |(p, _)| p.username.as_str()),
                )
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.setup.reuse_credential, None, "Ввести новую");
                    for (profile, same_target) in &eligible {
                        ui.selectable_value(
                            &mut self.setup.reuse_credential,
                            Some(profile.id),
                            if *same_target {
                                format!("{} · для этой цели", profile.username)
                            } else {
                                format!("{} · из домена", profile.username)
                            },
                        );
                    }
                });
        }
        let reuse = eligible
            .iter()
            .find(|(p, _)| Some(p.id) == self.setup.reuse_credential);
        if self.setup.reuse_credential != previous_account {
            if let Some((account, true)) = reuse {
                if let Some(connection) = self.connections.iter().find(|p| {
                    p.kind == ProfileKind::Execution
                        && p.credential_id == account.id
                        && p.host == account.target
                }) {
                    self.setup.winrm_authentication =
                        connection.winrm_authentication.unwrap_or_default();
                }
            }
        }
        let domain_reuse = reuse.is_some_and(|(_, same_target)| !*same_target);
        let valid_account = if reuse.is_some() {
            if self.setup.kind == ProfileKind::Execution {
                ui.small("У этой учётной записи должен быть доступ к WinRM выбранного ПК.");
            }
            true
        } else {
            let user_valid = required_text(ui, "Имя пользователя", &mut self.setup.username, 256);
            if self.setup.kind == ProfileKind::Execution {
                ui.small("Для домена можно указать user@domain. Для рабочей группы — HOST\\user.");
            } else {
                ui.small(
                    "Формат user@domain позволяет выбрать эту учётную запись и для сканирования.",
                );
            }
            let password_valid = password_field(ui, "Пароль", &mut self.setup.password);
            user_valid && password_valid
        };
        let mode = if self.setup.kind == ProfileKind::Execution {
            if domain_reuse {
                Some(WinRmAuthentication::Kerberos)
            } else {
                egui::CollapsingHeader::new("Аутентификация WinRM").show(ui, |ui| {
                    ui.radio_value(
                        &mut self.setup.winrm_authentication,
                        WinRmAuthentication::Negotiate,
                        "Negotiate · домен или рабочая группа",
                    );
                    ui.radio_value(
                        &mut self.setup.winrm_authentication,
                        WinRmAuthentication::Kerberos,
                        "Kerberos · домен",
                    );
                });
                (self.setup.winrm_authentication == WinRmAuthentication::Kerberos)
                    .then_some(WinRmAuthentication::Kerberos)
            }
        } else {
            None
        };
        let fully_qualified = mode != Some(WinRmAuthentication::Kerberos)
            || self.setup.host.trim().trim_end_matches('.').contains('.');
        if !fully_qualified && host_valid {
            ui.label("Для Kerberos укажите полное DNS-имя компьютера.");
        }
        if ui
            .add_enabled(
                host_valid && valid_domain && valid_account && fully_qualified,
                egui::Button::new("Сохранить подключение"),
            )
            .clicked()
        {
            let account = reuse.map_or_else(
                || ConnectionAccount::Password {
                    username: self.setup.username.trim().to_owned(),
                    password: Zeroizing::new(self.setup.password.to_string()),
                },
                |(profile, same_target)| {
                    if *same_target {
                        ConnectionAccount::SavedTarget {
                            credential_id: profile.id,
                            expected_revision: profile.revision,
                        }
                    } else {
                        ConnectionAccount::DomainAccount {
                            credential_id: profile.id,
                            expected_revision: profile.revision,
                        }
                    }
                },
            );
            self.send(Request::Configure(ConfigureConnection {
                id: self.setup.id,
                host: self.setup.host.trim().to_owned(),
                locate_domain_controller,
                kind: self.setup.kind,
                winrm_authentication: mode,
                account,
            }));
        }
    }
    fn credentials_form(&mut self, ui: &mut egui::Ui) {
        if !self.admin() {
            ui.label("Нужна роль Admin.");
            return;
        }
        self.profile_header(ui);
        for p in self.credentials.clone() {
            if ui
                .selectable_label(
                    self.credential_id == p.id,
                    format!(
                        "{} · {:?} · {} · r{}",
                        p.label, p.kind, p.target, p.revision
                    ),
                )
                .clicked()
            {
                self.credential_id = p.id;
                self.credential_revision = Some(p.revision);
                self.label = p.label;
                self.username = p.username;
                self.kind = p.kind;
                self.target = p.target;
                self.password.zeroize();
            }
        }
        if ui.button("Новый профиль").clicked() {
            self.credential_id = Uuid::new_v4();
            self.credential_revision = None;
            self.label.clear();
            self.username.clear();
            self.target.clear();
            self.password.zeroize();
        }
        ui.separator();
        let valid_label = required_text(ui, "Название", &mut self.label, 80);
        kind_select(ui, &mut self.kind);
        let valid_target = dns_field(ui, "DNS цели / контроллера домена", &mut self.target);
        let valid_user = required_text(ui, "Имя пользователя", &mut self.username, 256);
        let valid_password = password_field(ui, "Пароль", &mut self.password);
        if ui
            .add_enabled(
                valid_label && valid_target && valid_user && valid_password,
                egui::Button::new("Сохранить на сервере"),
            )
            .clicked()
        {
            self.send(Request::Credential(SaveCredential {
                id: self.credential_id,
                expected: self.credential_revision,
                label: self.label.trim().to_owned(),
                username: self.username.trim().to_owned(),
                target: self.target.trim().to_owned(),
                kind: self.kind,
                password: Zeroizing::new(self.password.to_string()),
            }));
        }
    }
    fn connections_form(&mut self, ui: &mut egui::Ui) {
        if !self.admin() {
            ui.label("Редактор профилей — Admin. Сервер также проверяет роль каждого запроса.");
            return;
        }
        self.profile_header(ui);
        for p in self.connections.clone() {
            ui.horizontal_wrapped(|ui| {
                if ui
                    .selectable_label(
                        self.connection_id == p.id,
                        format!("{} · {}:{} · r{}", p.label, p.host, p.port, p.revision),
                    )
                    .clicked()
                {
                    self.connection_id = p.id;
                    self.connection_revision = Some(p.revision);
                    self.connection_label = p.label;
                    self.connection_host = p.host;
                    self.winrm_authentication = p.winrm_authentication;
                    self.selected_credential = Some(p.credential_id);
                    self.test_key = Uuid::new_v4().to_string();
                    self.job = None;
                }
            });
        }
        if ui.button("Новое подключение").clicked() {
            self.connection_id = Uuid::new_v4();
            self.connection_revision = None;
            self.connection_label.clear();
            self.connection_host.clear();
            self.winrm_authentication = None;
            self.selected_credential = None;
            self.test_key = Uuid::new_v4().to_string();
            self.job = None;
        }
        ui.separator();
        let change_before = self.selected_credential;
        egui::ComboBox::from_id_salt("connection_credential")
            .selected_text(
                self.credentials
                    .iter()
                    .find(|p| Some(p.id) == self.selected_credential)
                    .map_or("Выберите профиль", |p| p.label.as_str()),
            )
            .show_ui(ui, |ui| {
                for p in &self.credentials {
                    ui.selectable_value(
                        &mut self.selected_credential,
                        Some(p.id),
                        format!("{} · {:?} · {}", p.label, p.kind, p.target),
                    );
                }
            });
        if self.selected_credential != change_before {
            if let Some(credential) = self
                .credentials
                .iter()
                .find(|p| Some(p.id) == self.selected_credential)
            {
                self.connection_host.clone_from(&credential.target);
                if self.connection_label.trim().is_empty() {
                    self.connection_label.clone_from(&credential.target);
                }
            }
        }
        let valid_label = required_text(ui, "Название подключения", &mut self.connection_label, 80);
        let valid_host = dns_field(ui, "DNS-имя", &mut self.connection_host);
        let credential = self
            .credentials
            .iter()
            .find(|p| Some(p.id) == self.selected_credential);
        let scope_matches = credential.is_some_and(|c| {
            orkestr_domain::canonical_host(&self.connection_host).is_ok_and(|host| host == c.target)
        });
        if credential.is_none() {
            required_label(ui, "Учётные данные", Some("выберите профиль"));
        } else if valid_host && !scope_matches {
            required_label(
                ui,
                "DNS-имя",
                Some("должно совпадать с DNS в профиле учётных данных"),
            );
        }
        if credential.is_some_and(|c| c.kind == ProfileKind::Execution) {
            ui.label("WinRM HTTPS · порт 5986 · проверка CIM");
            ui.label("Аутентификация WinRM");
            egui::ComboBox::from_id_salt("winrm_authentication")
                .selected_text(match self.winrm_authentication.unwrap_or_default() {
                    WinRmAuthentication::Negotiate => "Negotiate · домен или рабочая группа",
                    WinRmAuthentication::Kerberos => "Kerberos · домен",
                })
                .show_ui(ui, |ui| {
                    let previous = self.winrm_authentication.unwrap_or_default();
                    let mut selected = previous;
                    ui.selectable_value(
                        &mut selected,
                        WinRmAuthentication::Negotiate,
                        "Negotiate · домен или рабочая группа",
                    );
                    ui.selectable_value(
                        &mut selected,
                        WinRmAuthentication::Kerberos,
                        "Kerberos · домен",
                    );
                    if selected != previous {
                        self.winrm_authentication = Some(selected);
                    }
                });
            ui.small(match self.winrm_authentication.unwrap_or_default() {
                WinRmAuthentication::Negotiate => {
                    "Для рабочей группы укажите локальную учётную запись HOST\\user."
                }
                WinRmAuthentication::Kerberos => {
                    "Укажите полное DNS-имя целевого ПК и доменную учётную запись."
                }
            });
        } else if credential.is_some() {
            ui.label("LDAPS · порт 636 · запрос RootDSE");
        }
        if ui
            .add_enabled(
                valid_label && valid_host && scope_matches,
                egui::Button::new("Сохранить подключение"),
            )
            .clicked()
        {
            if let Some(c) = credential {
                self.send(Request::Connection(SaveConnection {
                    expected: self.connection_revision,
                    profile: ConnectionProfile {
                        id: self.connection_id,
                        revision: 0,
                        label: self.connection_label.trim().to_owned(),
                        kind: c.kind,
                        host: self.connection_host.trim().to_owned(),
                        port: if c.kind == ProfileKind::AdRead {
                            636
                        } else {
                            5986
                        },
                        credential_id: c.id,
                        winrm_authentication: if c.kind == ProfileKind::Execution {
                            self.winrm_authentication
                        } else {
                            None
                        },
                    },
                }));
            }
        }
        ui.separator();
        let saved = self.connections.iter().find(|p| p.id == self.connection_id);
        let test_request = saved
            .filter(|p| {
                Some(p.revision) == self.connection_revision
                    && p.label == self.connection_label
                    && p.host == self.connection_host
                    && Some(p.credential_id) == self.selected_credential
                    && p.winrm_authentication == self.winrm_authentication
            })
            .map(|p| TestConnection {
                connection_id: p.id,
                expected_revision: p.winrm_authentication.map(|_| p.revision),
            });
        if saved.is_some() && test_request.is_none() {
            ui.small("Сохраните изменения или заново выберите подключение перед проверкой.");
        }
        if ui
            .add_enabled(
                test_request.is_some(),
                egui::Button::new("Read-only проверка с сервера"),
            )
            .clicked()
        {
            if let Some(request) = test_request {
                self.send(Request::Test(request, self.test_key.clone()));
            }
        }
        ui.small("При повторе после ошибки связи используется тот же Idempotency-Key. Для новой проверки выберите «Новая проверка».");
        if ui.button("Новая проверка (новый ключ)").clicked() {
            self.test_key = Uuid::new_v4().to_string();
            self.job = None;
        }
        self.show_test_status(ui);
    }
    fn show_test_status(&mut self, ui: &mut egui::Ui) {
        if let Some(job) = self.job.clone() {
            let status = match job.state {
                JobState::Queued => "Проверка в очереди",
                JobState::Running => "Сервер проверяет доступ",
                JobState::Succeeded => "Доступ подтверждён",
                JobState::Failed => "Проверка доступа завершилась ошибкой",
                JobState::Interrupted => "Проверка прервана",
                JobState::Cancelled => "Проверка отменена",
            };
            ui.label(format!("{status} · {}", job.code));
            if matches!(job.state, JobState::Queued | JobState::Running)
                && ui.button("Обновить результат проверки").clicked()
            {
                self.send(Request::Job(job.id));
            }
        }
    }
    fn operators_form(&mut self, ui: &mut egui::Ui) {
        if ui.button("Обновить операторов и журнал").clicked() {
            self.send(Request::LoadAudit);
        }
        ui.separator();
        if self.admin() {
            for actor in self.operators.clone() {
                ui.horizontal_wrapped(|ui| {
                    ui.label(format!(
                        "{} · {:?} · до {}",
                        actor.label,
                        actor.role,
                        actor.expires_at.format("%Y-%m-%d")
                    ));
                    if self.actor.as_ref().is_some_and(|a| a.id != actor.id)
                        && ui.button("Отозвать").clicked()
                    {
                        self.send(Request::Revoke(actor.id));
                    }
                });
            }
            ui.separator();
            field(ui, "Имя оператора / консоли", &mut self.issue_label);
            ui.horizontal(|ui| {
                for role in [Role::Viewer, Role::Operator, Role::Admin] {
                    ui.selectable_value(&mut self.issue_role, role, format!("{role:?}"));
                }
                ui.label("Дней");
                ui.add(egui::DragValue::new(&mut self.days).range(1..=365));
            });
            if ui.button("Выдать персональный токен").clicked() {
                self.issued = None;
                self.send(Request::Issue(IssueToken {
                    label: self.issue_label.clone(),
                    role: self.issue_role,
                    days: self.days,
                }));
            }
            let mut save_local = None;
            if let Some(issued) = &mut self.issued {
                ui.label("Однократно. Передайте оператору защищённым каналом. DPAPI-файл другой identity не переносим.");
                ui.checkbox(
                    &mut self.show_issued,
                    "Показать токен для защищённой передачи",
                );
                ui.add(
                    egui::TextEdit::singleline(&mut issued.token.as_str())
                        .password(!self.show_issued)
                        .desired_width(f32::INFINITY),
                );
                if ui
                    .button("Сохранить DPAPI для текущего Windows-пользователя…")
                    .clicked()
                {
                    if let Some(path) = rfd::FileDialog::new()
                        .set_file_name("orkestr-token.dpapi")
                        .save_file()
                    {
                        save_local = Some(Request::SaveLocalToken(
                            path,
                            Zeroizing::new(issued.token.to_string()),
                        ));
                    }
                }
            }
            if let Some(request) = save_local {
                self.send(request);
            }
        }
        ui.separator();
        ui.strong("Последние события сервера (до 128)");
        for event in self.events.iter().rev() {
            ui.label(format!(
                "#{} {} · {} · {}",
                event.cursor, event.verb, event.target, event.code
            ));
        }
    }
}
fn field(ui: &mut egui::Ui, label: &str, text: &mut String) {
    ui.label(label);
    ui.add(egui::TextEdit::singleline(text).desired_width(f32::INFINITY));
}
fn required_label(ui: &mut egui::Ui, label: &str, problem: Option<&str>) -> egui::Id {
    match problem {
        Some(problem) => {
            ui.colored_label(
                crate::theme::Palette::get(ui.visuals().dark_mode).error,
                format!("{label} · {problem}"),
            )
            .id
        }
        None => ui.label(label).id,
    }
}
fn text_problem(value: &str, max: usize) -> Option<&'static str> {
    if value.trim().is_empty() {
        Some("обязательное поле")
    } else if value.trim().len() > max || value.chars().any(char::is_control) {
        Some("слишком длинное или содержит недопустимые символы")
    } else {
        None
    }
}
fn required_text(ui: &mut egui::Ui, label: &str, value: &mut String, max: usize) -> bool {
    let caption = required_label(ui, label, text_problem(value, max));
    ui.add(egui::TextEdit::singleline(value).desired_width(f32::INFINITY))
        .labelled_by(caption);
    text_problem(value, max).is_none()
}
fn dns_problem(value: &str) -> Option<&'static str> {
    text_problem(value, 253).or_else(|| {
        let valid = orkestr_domain::canonical_host(value)
            .is_ok_and(|host| !host.bytes().all(|b| b.is_ascii_digit() || b == b'.'));
        (!valid).then_some("укажите DNS-имя без адреса, порта и IP")
    })
}
fn dns_field(ui: &mut egui::Ui, label: &str, value: &mut String) -> bool {
    let caption = required_label(ui, label, dns_problem(value));
    ui.add(egui::TextEdit::singleline(value).desired_width(f32::INFINITY))
        .labelled_by(caption);
    dns_problem(value).is_none()
}
fn password_field(ui: &mut egui::Ui, label: &str, password: &mut Zeroizing<String>) -> bool {
    let problem = if password.is_empty() {
        Some("обязательное поле")
    } else if password.len() > 4096 {
        Some("слишком длинный пароль")
    } else {
        None
    };
    let caption = required_label(ui, label, problem);
    ui.add(
        egui::TextEdit::singleline(&mut **password)
            .password(true)
            .desired_width(f32::INFINITY),
    )
    .labelled_by(caption);
    !password.is_empty() && password.len() <= 4096
}
fn remote_error(action: &str, raw: &str) -> String {
    let code = raw.split_once(':').map_or(raw, |(code, _)| code);
    let explanation=match code {
        "CONFLICT"=>"Профиль изменился на сервере. Нажмите «Обновить список» и снова откройте этот профиль.",
        "INVALID_TEXT"=>"Заполните обязательные текстовые поля.",
        "INVALID_HOST"|"TLS_DNS_NAME_AND_DEFAULT_SECURE_PORT_REQUIRED"=>"Укажите DNS-имя компьютера или контроллера.",
        "INVALID_DOMAIN"=>"Введите полное DNS-имя домена, например corp.example.",
        "LDAPS_CONTROLLER_NOT_FOUND"=>"Сервер не нашёл контроллер с доступным LDAPS и доверенным сертификатом. Проверьте имя домена или укажите контроллер вручную.",
        "INVALID_SECRET"=>"Укажите пароль учётной записи.",
        "INVALID_WINRM_PROFILE"=>"Для Kerberos требуется полное DNS-имя компьютера.",
        "DOMAIN_ACCOUNT_SCOPE_MISMATCH"=>"Сохранённая доменная учётная запись доступна для зарегистрированного компьютера из того же DNS-домена.",
        "CREDENTIAL_SCOPE_MISMATCH"=>"Учётная запись сохранена для другой цели или типа подключения.",
        "HTTPS_CONNECTION_FAILED"=>"Нет HTTPS-связи с Orkestr. Проверьте, запущен ли сервер, и совпадает ли адрес с сертификатом.",
        "INVALID_CA_CERTIFICATE"|"CA_FILE_UNAVAILABLE"=>"Выберите доступный файл сертификата Orkestr (.crt или .pem).",
        "UNAUTHORIZED"|"INVALID_TOKEN_FORMAT"=>"Подключитесь с действующим токеном.",
        "FORBIDDEN"=>"Для этой операции требуются права Admin.",
        "HTTP_404"|"NOT_FOUND"=>"Обновите сервер и консоль до одной сборки и повторите запрос.",
        "CREDENTIAL_UNAVAILABLE"=>"Сервер не смог сохранить учётные данные. Проверьте учётную запись Windows, под которой он запущен.",
        _=>return format!("{action} · {raw}"),
    };
    format!("{action}. {explanation} ({code})")
}
fn kind_select(ui: &mut egui::Ui, kind: &mut ProfileKind) {
    ui.horizontal(|ui| {
        ui.selectable_value(kind, ProfileKind::AdRead, "AD · только чтение");
        ui.selectable_value(kind, ProfileKind::Execution, "ПК · выполнение (M3)");
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn errors_and_required_fields_are_specific_to_the_operation() {
        assert!(dns_problem(" ").unwrap().contains("обязательное"));
        assert!(dns_problem("https://host.example:5986").is_some());
        assert!(dns_problem("192.0.2.1").is_some());
        assert!(dns_problem(" HOST.example. ").is_none());
        let invalid = remote_error("Сохранение учётных данных", "INVALID_TEXT");
        assert!(invalid.contains("обязательные"));
        assert!(!invalid.contains("сертификат"));
        assert!(remote_error("Сохранение подключения", "CONFLICT").contains("Обновить список"));
        assert!(remote_error("Загрузка профилей", "UNAUTHORIZED").contains("токеном"));
    }
    #[test]
    fn m2_windows_render_both_themes_and_zoom_without_network() {
        for dark in [false, true] {
            for zoom in [1., 1.25, 1.5, 2.] {
                for window in [
                    Window::Server,
                    Window::Setup,
                    Window::Credentials,
                    Window::Connections,
                    Window::Operators,
                ] {
                    let ctx = egui::Context::default();
                    crate::theme::install(&ctx);
                    ctx.set_visuals(if dark {
                        egui::Visuals::dark()
                    } else {
                        egui::Visuals::light()
                    });
                    ctx.set_zoom_factor(zoom);
                    let mut remote = Remote {
                        active: true,
                        actor: Some(Actor {
                            id: Uuid::new_v4(),
                            label: "Fixture administrator".into(),
                            role: Role::Admin,
                            expires_at: chrono::Utc::now() + chrono::Duration::days(1),
                        }),
                        window: Some(window),
                        ..Remote::default()
                    };
                    if window == Window::Connections {
                        let id = Uuid::new_v4();
                        remote.credentials.push(CredentialProfile {
                            id,
                            revision: 1,
                            label: "WinRM fixture".into(),
                            username: "FIXTURE\\test".into(),
                            kind: ProfileKind::Execution,
                            target: "test.invalid".into(),
                            owner_sid: "synthetic-owner".into(),
                            updated_at: chrono::Utc::now(),
                        });
                        remote.selected_credential = Some(id);
                        remote.winrm_authentication = Some(WinRmAuthentication::Kerberos);
                    }
                    let output = ctx.run(
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(
                                egui::Pos2::ZERO,
                                egui::vec2(1280., 820.),
                            )),
                            ..Default::default()
                        },
                        |ctx| remote.show(ctx),
                    );
                    assert!(!output.shapes.is_empty());
                    assert!(remote.sender.is_none());
                }
            }
        }
    }
    #[test]
    fn viewer_controls_are_read_only_and_secrets_not_preferences() {
        let remote = Remote {
            active: true,
            actor: Some(Actor {
                id: Uuid::new_v4(),
                label: "Viewer fixture".into(),
                role: Role::Viewer,
                expires_at: chrono::Utc::now() + chrono::Duration::days(1),
            }),
            ..Remote::default()
        };
        assert!(!remote.can_write());
        assert!(!remote.admin());
        let preferences = serde_json::to_string(&crate::theme::Preferences::default()).unwrap();
        assert!(!preferences.contains("token"));
        assert!(!preferences.contains("password"));
    }
}
