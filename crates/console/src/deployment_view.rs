use crate::remote::Remote;
use eframe::egui;
use orkestr_application::Workspace;
use orkestr_domain::{DeviceId, PackageRevisionId};
use orkestr_protocol::{
    DeploymentState, InstallerExecution, ManagedInstaller, PlanDeployment, ProfileKind,
};
use uuid::Uuid;

#[derive(Default)]
pub(crate) struct DeploymentView {
    name: String,
    version: String,
    sha256: String,
    product_code: String,
    device: Option<DeviceId>,
    profile: Option<Uuid>,
    selected: Option<Uuid>,
    confirmed_revision: Option<(Uuid, u64)>,
}

impl DeploymentView {
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        remote: &mut Remote,
        workspace: &Workspace,
        package: Option<PackageRevisionId>,
    ) {
        if let Some(id) = remote.planned_deployment.take() {
            self.selected = Some(id);
            self.confirmed_revision = None;
        }
        ui.separator();
        ui.heading("Развёртывание пакетов");
        if let Some(error) = &remote.deployment_error {
            ui.label(error);
        }
        ui.add_enabled_ui(!remote.busy(), |ui| {
            egui::CollapsingHeader::new("Новая ревизия MSI").show(ui, |ui| {
                ui.small("Сначала поместите MSI в хранилище сервера командой stage-msi.");
                edit(ui, "Название", &mut self.name, 128);
                edit(
                    ui,
                    "Версия для проверки после установки",
                    &mut self.version,
                    80,
                );
                edit(ui, "SHA-256", &mut self.sha256, 64);
                edit(ui, "ProductCode MSI", &mut self.product_code, 40);
                let product_code = Uuid::parse_str(self.product_code.trim()).ok();
                let ready = !self.name.trim().is_empty()
                    && !self.version.trim().is_empty()
                    && self.sha256.trim().len() == 64;
                if ui
                    .add_enabled(
                        ready && product_code.is_some(),
                        egui::Button::new("Зарегистрировать ревизию"),
                    )
                    .clicked()
                {
                    if let Some(product_code) = product_code {
                        remote.register_msi(ManagedInstaller {
                            allow_upgrade: false,
                            version_components: None,
                            name: self.name.trim().into(),
                            version: self.version.trim().into(),
                            sha256: self.sha256.trim().into(),
                            product_code,
                            execution: Default::default(),
                            registry_key: None,
                        });
                    }
                }
            });
            egui::CollapsingHeader::new("План установки выбранного пакета").show(ui, |ui| {
                let targets = &workspace.inventory.devices;
                egui::ComboBox::from_id_salt("deploy_target")
                    .selected_text(
                        targets
                            .iter()
                            .find(|d| Some(d.device.id) == self.device)
                            .map_or("Целевой ПК", |d| d.device.host.as_str()),
                    )
                    .show_ui(ui, |ui| {
                        for record in targets.iter().filter(|r| !r.device.disabled) {
                            ui.selectable_value(
                                &mut self.device,
                                Some(record.device.id),
                                &record.device.host,
                            );
                        }
                    });
                let device = targets
                    .iter()
                    .find(|r| Some(r.device.id) == self.device && !r.device.disabled);
                let profiles: Vec<_> = remote
                    .profiles(ProfileKind::Execution)
                    .filter(|p| device.is_some_and(|d| d.device.host == p.host))
                    .cloned()
                    .collect();
                if !profiles.iter().any(|p| Some(p.id) == self.profile) {
                    self.profile = (profiles.len() == 1).then(|| profiles[0].id);
                }
                egui::ComboBox::from_id_salt("deploy_profile")
                    .selected_text(
                        profiles
                            .iter()
                            .find(|p| Some(p.id) == self.profile)
                            .map_or("Профиль WinRM", |p| p.label.as_str()),
                    )
                    .show_ui(ui, |ui| {
                        for profile in &profiles {
                            ui.selectable_value(
                                &mut self.profile,
                                Some(profile.id),
                                &profile.label,
                            );
                        }
                    });
                if ui.small_button("Обновить профили").clicked() {
                    remote.reload_profiles();
                }
                if let (Some(package_revision_id), Some(device), Some(profile)) = (
                    package,
                    device,
                    profiles.iter().find(|p| Some(p.id) == self.profile),
                ) {
                    if ui.button("Создать план для одобрения").clicked() {
                        remote.plan_deployment(PlanDeployment {
                            device_id: device.device.id,
                            package_revision_id,
                            connection_id: profile.id,
                            connection_revision: profile.revision,
                        });
                    }
                }
            });
            if ui.button("Обновить задания").clicked() {
                remote.load_deployments();
            }
        });
        for record in &remote.deployments {
            if ui
                .selectable_label(
                    self.selected == Some(record.id),
                    format!(
                        "{} · {} {} · {:?}",
                        record.host, record.installer.name, record.installer.version, record.state
                    ),
                )
                .clicked()
            {
                self.selected = Some(record.id);
                self.confirmed_revision = None;
            }
        }
        if let Some(record) = remote
            .deployments
            .iter()
            .find(|r| Some(r.id) == self.selected)
            .cloned()
        {
            ui.label(format!("{} · {}", record.id, record.code));
            ui.monospace(format!("SHA-256 {}", record.installer.sha256));
            ui.monospace(format!("Ревизия {}", record.plan.package_revision_id));
            ui.label(format!(
                "Проверка: системная запись {} · версия {}",
                record
                    .installer
                    .registry_key
                    .clone()
                    .unwrap_or_else(|| record.installer.product_code.to_string()),
                record.installer.version
            ));
            ui.label(if record.installer.allow_upgrade {
                "Замена известной меньшей версии: разрешена"
            } else {
                "Замена существующей версии: запрещена"
            });
            if let Some(count) = record.installer.version_components {
                ui.label(format!(
                    "Проверка версии: первые {count} числовых компонентов"
                ));
            }
            match &record.installer.execution {
                InstallerExecution::Msi {} => {
                    ui.label("MSI · /qn /norestart · без перезагрузки");
                }
                InstallerExecution::Exe { arguments } => {
                    ui.label("EXE · параметры запуска:");
                    ui.monospace(arguments);
                }
            }
            for (i, attempt) in record.attempts.iter().enumerate() {
                ui.label(format!(
                    "Попытка {} · {} · {:?} · обнаружение {:?}",
                    i + 1,
                    attempt.code,
                    attempt.exit_code,
                    attempt.detection
                ));
            }
            if let Some(detection) = record.reconciliation {
                ui.label(format!("Сверка версии · {detection:?}"));
            }
            ui.add_enabled_ui(!remote.busy(), |ui| {
                let revision = (record.id, record.revision);
                let retry = record.state == DeploymentState::Failed
                    && record.attempts.len() < 3
                    && (record.code == "RESTARTED_BEFORE_START"
                        || record.attempts.last().is_some_and(|a| a.safe_to_retry));
                if retry || record.state == DeploymentState::WaitingApproval {
                    let mut confirmed = self.confirmed_revision == Some(revision);
                    if ui
                        .checkbox(
                            &mut confirmed,
                            format!(
                                "Одобряю {} {} для {}",
                                record.installer.name, record.installer.version, record.host
                            ),
                        )
                        .changed()
                    {
                        self.confirmed_revision = confirmed.then_some(revision);
                    }
                    let (label, action) = if retry {
                        ("Повторить с одобрением", "retry")
                    } else {
                        ("Одобрить и поставить в очередь", "approve")
                    };
                    if ui
                        .add_enabled(confirmed, egui::Button::new(label))
                        .clicked()
                    {
                        self.confirmed_revision = None;
                        remote.change_deployment(record.id, action, record.revision);
                    }
                }
                if matches!(
                    record.state,
                    DeploymentState::WaitingApproval | DeploymentState::Queued
                ) && ui.button("Отменить до запуска").clicked()
                {
                    remote.change_deployment(record.id, "cancel", record.revision);
                }
                if record.state == DeploymentState::NeedsReconciliation
                    && ui.button("Сверить установленную версию").clicked()
                {
                    remote.change_deployment(record.id, "reconcile", record.revision);
                }
            });
        }
    }
}

fn edit(ui: &mut egui::Ui, label: &str, value: &mut String, limit: usize) {
    let caption = ui.label(label);
    ui.add(
        egui::TextEdit::singleline(value)
            .char_limit(limit)
            .desired_width(f32::INFINITY),
    )
    .labelled_by(caption.id);
}
