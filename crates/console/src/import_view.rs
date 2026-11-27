use crate::{app::Console, details};
use eframe::egui;
use orkestr_application::{Command, ImportFormat};
use std::path::PathBuf;

pub struct ImportView {
    path: String,
    origin: String,
    backup_path: String,
    format: ImportFormat,
    copy_confirmed: bool,
    pub approve: bool,
    pub token: u64,
    pub report: Option<orkestr_application::ImportReport>,
    pub busy: bool,
}
impl Default for ImportView {
    fn default() -> Self {
        Self {
            path: String::new(),
            origin: String::new(),
            backup_path: String::new(),
            format: ImportFormat::Inventory,
            copy_confirmed: false,
            approve: false,
            token: 0,
            report: None,
            busy: false,
        }
    }
}
impl Console {
    pub(crate) fn import_page(&mut self, ui: &mut egui::Ui) {
        ui.heading("Импорт копий");
        ui.label("Чтение без записи в источник. Никаких подключений к ПК, переноса паролей или запуска пакетов.");
        ui.separator();
        let before = (
            self.import.path.clone(),
            self.import.origin.clone(),
            self.import.format,
        );
        ui.add_enabled_ui(!self.import.busy, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.selectable_value(&mut self.import.format, ImportFormat::Inventory, "Inventory · SQLite backup");
                let deploy=ui.selectable_value(&mut self.import.format,ImportFormat::Deploy,"Deploy · console.json");
                self.probes.insert("import_deploy",deploy.rect);
            });
            ui.label("Локальная закрытая backup-копия");
            ui.horizontal(|ui| {
                let path=ui.add(egui::TextEdit::singleline(&mut self.import.path).desired_width((ui.available_width()-120.).max(160.)));
                self.probes.insert("import_path",path.rect);
                if ui.button("Выбрать…").clicked() {
                    if let Some(path)=rfd::FileDialog::new().pick_file(){self.import.path=path.display().to_string();}
                }
            });
            if self.import.format==ImportFormat::Deploy {
                ui.label("Происхождение пакетов (необязательно; одинаковое имя для копий одного старого сервера)");
                ui.text_edit_singleline(&mut self.import.origin);
                ui.small("При одинаковом происхождении legacy ID сохраняет PackageId, но создаётся новая неутверждённая revision. Не объединяйте разные серверы одним именем.");
            }
            let confirm=ui.checkbox(&mut self.import.copy_confirmed,"Это моя согласованная локальная копия, а не живая база старой программы");
            self.probes.insert("import_copy_confirm",confirm.rect);
            let prepare=ui.add_enabled(self.import.copy_confirmed&&!self.import.path.trim().is_empty(),egui::Button::new("Проверить · dry-run"));
            self.probes.insert("import_prepare",prepare.rect);
            if prepare.clicked() {
                self.import.token=self.import.token.wrapping_add(1);
                self.import.report=None;
                self.import.approve=false;
                self.error=None;
                if self.submit(Command::PrepareImport{token:self.import.token,format:self.import.format,path:PathBuf::from(&self.import.path),origin:(!self.import.origin.trim().is_empty()).then(||self.import.origin.clone())}) {
                    self.import.busy=true;
                }
            }
        });
        if before
            != (
                self.import.path.clone(),
                self.import.origin.clone(),
                self.import.format,
            )
        {
            self.import.token = self.import.token.wrapping_add(1);
            self.import.report = None;
            self.import.approve = false;
            self.import.copy_confirmed = false;
            self.import.backup_path.clear();
            self.submit(Command::DiscardImport);
        }
        if self.import.busy {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Обработка на ограниченном локальном worker…");
            });
        }
        let report = self.import.report.clone();
        if let Some(report) = report {
            ui.separator();
            egui::ScrollArea::vertical().id_salt("import_report").max_height(240.).show(ui,|ui|{
                ui.strong("Результат dry-run");ui.label(format!("ПК: {} · обновлений: {} · сканов: {} · групп: {} · пакетов: {} · требует проверки: {}",report.devices,report.updated_devices,report.scans,report.collections,report.packages,report.review_items));
                ui.small(format!("Снимок #{} · {}",report.base_generation,report.format));ui.monospace(format!("SHA-256: {}",report.source_sha256));
                for warning in &report.warnings{ui.label(format!("Предупреждение · {warning}"));}
                for error in &report.errors{ui.colored_label(crate::theme::Palette::get(ui.visuals().dark_mode).error,format!("Блокирующая ошибка · {error}"));}
                ui.label("Rollback: перед записью создаётся НОВЫЙ согласованный backup Orkestr. Для восстановления закройте консоль и откройте копию backup по новому пути. Источник не меняется.");
            });
            ui.add_enabled_ui(report.can_apply() && !self.import.busy, |ui| {
                let approve = ui.checkbox(
                    &mut self.import.approve,
                    "Подтверждаю импорт данных; все старые расписания остаются выключенными",
                );
                self.probes.insert("import_approve", approve.rect);
                ui.label("Новый rollback backup (не перезаписывается)");
                let backup = ui.add(
                    egui::TextEdit::singleline(&mut self.import.backup_path)
                        .desired_width(ui.available_width().min(600.)),
                );
                self.probes.insert("import_backup", backup.rect);
                let apply = ui.add_enabled(
                    self.import.approve,
                    egui::Button::new("Применить с rollback backup…"),
                );
                self.probes.insert("import_apply", apply.rect);
                if apply.clicked() {
                    let backup = if self.import.backup_path.trim().is_empty() {
                        rfd::FileDialog::new()
                            .set_file_name("before-import.sqlite")
                            .save_file()
                    } else {
                        Some(PathBuf::from(self.import.backup_path.trim()))
                    };
                    if let Some(backup) = backup {
                        if self.submit(Command::ApplyImport {
                            token: self.import.token,
                            backup,
                        }) {
                            self.import.busy = true;
                        }
                    }
                }
            });
        }
    }
    pub(crate) fn review_page(&mut self, ui: &mut egui::Ui) {
        ui.heading("Требует проверки");
        ui.label("Только read-only metadata. Расписания выключены; отсюда невозможно запустить пакет или команду.");
        ui.separator();
        let w = self.workspace.as_ref().unwrap();
        egui::ScrollArea::vertical()
            .id_salt("review")
            .show(ui, |ui| {
                if w.reviews.is_empty() {
                    ui.label("Записей для проверки нет.");
                }
                for r in &w.reviews {
                    egui::CollapsingHeader::new(format!("{} · {} · выключено", r.kind, r.name))
                        .id_salt(&r.id)
                        .show(ui, |ui| {
                            ui.monospace(format!("Source SHA-256: {}", r.source_sha256));
                            details::value_ui(ui, &r.data, 0);
                        });
                }
            });
    }
}
