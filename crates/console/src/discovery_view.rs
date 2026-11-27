use crate::{app::Console, theme::Palette};
use eframe::egui;
use orkestr_application::Command;
use orkestr_protocol::*;

pub(crate) struct DiscoveryView {
    pub open: bool,
    pub busy: bool,
    pub token: u64,
    pub format: DiscoverySource,
    pub text: String,
    pub preview: Option<PreparedDiscovery>,
    pub approve: bool,
    pub error: Option<String>,
    pub result: Option<AppliedDiscovery>,
    pub apply_attempted: bool,
    pub reveal_preview: bool,
    ad_profile: Option<uuid::Uuid>,
    ad_seen: Option<uuid::Uuid>,
}
impl Default for DiscoveryView {
    fn default() -> Self {
        Self {
            open: false,
            busy: false,
            token: 0,
            format: DiscoverySource::Manual,
            text: String::new(),
            preview: None,
            approve: false,
            error: None,
            result: None,
            apply_attempted: false,
            reveal_preview: false,
            ad_profile: None,
            ad_seen: None,
        }
    }
}
impl DiscoveryView {
    fn invalidate(&mut self) {
        self.token = self.token.wrapping_add(1);
        self.preview = None;
        self.approve = false;
        self.result = None;
        self.error = None;
        self.apply_attempted = false;
    }
}

impl Console {
    pub(crate) fn discovery_modal(&mut self, ctx: &egui::Context) {
        if !self.discovery.open {
            return;
        }
        let mut view = std::mem::take(&mut self.discovery);
        let mut close = false;
        egui::Modal::new(egui::Id::new("device_discovery")).show(ctx,|ui| {
            ui.set_width((ctx.screen_rect().width()-64.).clamp(260.,800.));
            ui.heading("Добавить компьютеры");
            ui.label("Добавление записей в реестр с предпросмотром и подтверждением.");
            ui.small("Изменять реестр может Admin. Пароли и пути сервера сюда не вводятся.");
            egui::ScrollArea::vertical().id_salt("discovery_scroll")
                .max_height((ctx.screen_rect().height()-220.).max(180.)).show(ui,|ui| {
                let before = (view.format,view.text.clone(),view.ad_profile);
                ui.add_enabled_ui(!view.busy && !view.apply_attempted && self.can_manage_devices(),|ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.selectable_value(&mut view.format,DiscoverySource::Manual,"Ручной список");
                        let csv = ui.selectable_value(&mut view.format,DiscoverySource::Csv,"CSV");
                        self.probes.insert("discovery_csv",csv.rect);
                        if self.remote.admin() {
                            ui.selectable_value(&mut view.format, DiscoverySource::Ad, "AD · подразделение");
                        }
                    });
                    if before.0 != view.format && (before.0 == DiscoverySource::Ad || view.format == DiscoverySource::Ad) {
                        view.text.clear();
                    }
                    let caption=ui.label(match view.format {
                        DiscoverySource::Manual => "DNS-имена — по одному в строке",
                        DiscoverySource::Csv => "CSV: host, необязательно name и site; разделитель — запятая",
                        DiscoverySource::Ad => "Полный DN подразделения, например OU=Lab,DC=example,DC=test",
                    });
                    if view.format == DiscoverySource::Ad {
                        let profiles: Vec<_> = self.remote.profiles(ProfileKind::AdRead).cloned().collect();
                        if profiles.is_empty() && ui.button("Настроить подключение к AD…").clicked() {
                            self.remote.open_setup(ProfileKind::AdRead,None);
                            close=true;
                        }
                        if !profiles.iter().any(|p| Some(p.id) == view.ad_profile) {
                            view.ad_profile = (profiles.len() == 1).then(|| profiles[0].id);
                        }
                        ui.add(egui::TextEdit::singleline(&mut view.text).char_limit(2048).desired_width(f32::INFINITY)).labelled_by(caption.id);
                        ui.horizontal_wrapped(|ui| {
                            egui::ComboBox::from_id_salt("ad_discovery_profile").selected_text(profiles.iter().find(|p| Some(p.id)==view.ad_profile).map_or("Профиль AD Read", |p| p.label.as_str())).show_ui(ui, |ui| {
                                for profile in &profiles { ui.selectable_value(&mut view.ad_profile,Some(profile.id),&profile.label); }
                            });
                            if ui.small_button("Обновить профили").clicked() { self.remote.reload_profiles(); }
                        });
                        return;
                    }
                    let text = ui.add(egui::TextEdit::multiline(&mut view.text).desired_rows(5)
                        .desired_width(f32::INFINITY).char_limit(DISCOVERY_TEXT_LIMIT)).labelled_by(caption.id);
                    self.probes.insert("discovery_text",text.rect);
                    ui.small(format!("{} / {} байт UTF-8 · до 1000 записей",view.text.len(),DISCOVERY_TEXT_LIMIT));
                    if view.text.len()>DISCOVERY_TEXT_LIMIT {ui.colored_label(Palette::get(ui.visuals().dark_mode).error,"Сократите текст до 24 KiB UTF-8 перед предпросмотром.");}
                    if ui.button("Загрузить локальный UTF-8 файл…").clicked() {
                        if let Some(path) = rfd::FileDialog::new().add_filter("UTF-8 список/CSV",&["txt","csv"]).pick_file() {
                            view.invalidate();
                            self.submit(Command::DiscardDiscovery);
                            if self.submit(Command::ReadDiscoveryFile { token:view.token,path }) { view.busy=true; }
                        }
                    }
                });
                if before != (view.format,view.text.clone(),view.ad_profile) {
                    view.invalidate();
                    self.submit(Command::DiscardDiscovery);
                }
                let can_start = view.format != DiscoverySource::Ad || (view.ad_profile.is_some() && !self.remote.inventory_busy());
                let prepare = ui.add_enabled(can_start && !view.busy && !view.apply_attempted && !view.text.trim().is_empty()
                    && view.text.len() <= DISCOVERY_TEXT_LIMIT && self.can_manage_devices(),egui::Button::new("Предпросмотр списка"));
                self.probes.insert("discovery_prepare",prepare.rect);
                if prepare.clicked() {
                    view.invalidate();
                    if view.format == DiscoverySource::Ad {
                        let profile = self.remote.profiles(ProfileKind::AdRead).find(|p| Some(p.id) == view.ad_profile).cloned();
                        if let Some(profile) = profile {
                            self.remote.start_inventory(StartInventoryJob { connection_id:profile.id, expected_revision:profile.revision,
                                    task:InventoryTask::DiscoverOu { base_dn:view.text.trim().to_owned() } });
                        }
                    } else if self.submit(Command::PrepareDiscovery { token:view.token,
                        input:PrepareDiscovery { format:view.format,text:view.text.clone() } }) { view.busy=true; }
                }
                if view.format == DiscoverySource::Ad {
                    let job = self.remote.inventory_job.as_ref().filter(|j| Some(j.connection_id)==view.ad_profile
                        && matches!(&j.task, InventoryTask::DiscoverOu { base_dn } if base_dn==view.text.trim())).cloned();
                    if let Some(job) = job {
                        ui.label(format!("Запрос OU · {:?} · {}",job.state,job.code));
                        if matches!(job.state,JobState::Queued|JobState::Running) && ui.small_button("Обновить состояние запроса OU").clicked() {
                            self.remote.poll_inventory_job();
                        }
                        if view.ad_seen != Some(job.id) {
                            if let Some(preview) = job.discovery {
                                view.ad_seen = Some(job.id);
                                view.preview = Some(preview);
                                view.reveal_preview = true;
                            }
                        }
                    }
                    if let Some(error) = &self.remote.inventory_error { ui.label(error); }
                }
                if let Some(message) = &view.error { ui.colored_label(Palette::get(ui.visuals().dark_mode).error,message); }
                if view.busy { ui.horizontal(|ui| {ui.spinner(); ui.label("Обработка вне UI-потока…");}); }
                if let Some(result) = &view.result {
                    ui.separator();
                    ui.strong(format!("Записано изменений: {} · снимок #{}",result.changes,result.generation));
                    ui.label(result.backup.as_ref().map_or_else(||"No-op: резервная копия не потребовалась".into(),|b|format!("Резервная копия рядом с базой backend: {b}")));
                    ui.label("Для сканирования откройте карточку добавленного ПК.");
                    if ui.button("Новый список").clicked() { view.invalidate(); }
                } else if let Some(preview) = view.preview.clone() {
                    ui.separator();
                    let heading=ui.strong(format!("{} строк · {} изменений · снимок #{}",preview.rows.len(),preview.changes,preview.generation));
                    if view.reveal_preview { heading.scroll_to_me(Some(egui::Align::Min));view.reveal_preview=false; }
                    ui.small(format!("Preview {} · до {} UTC",preview.id,preview.expires_at.format("%H:%M:%S")));
                    egui::ScrollArea::both().id_salt("discovery_rows").max_height(180.).show(ui,|ui| {
                        egui::Grid::new("discovery_grid").striped(true).show(ui,|ui| {
                            for label in ["Строка","DNS-имя / цель","Действие","Имя: было → станет","Площадка: было → станет"] { ui.strong(label); } ui.end_row();
                            for row in &preview.rows {
                                ui.label(row.row.to_string());ui.label(row.target_host.as_ref().filter(|h|*h!=&row.host).map_or_else(||row.host.clone(),|h|format!("{} → {h}",row.host))).on_hover_text(row.directory.as_ref().map_or_else(||row.device_id.map_or_else(||"Новая/неоднозначная identity".into(),|id|id.to_string()),|ad|format!("{}\n{}\n{}",ad.guid,ad.ou,ad.os)));
                                ui.label(match row.action { DiscoveryAction::Add=>"Добавить",DiscoveryAction::Update=>"Обновить",
                                    DiscoveryAction::Unchanged=>"Без изменений",DiscoveryAction::Duplicate=>"Дубликат",DiscoveryAction::Conflict=>"Конфликт" });
                                ui.label(format!("{} → {}",row.previous_name.as_deref().unwrap_or("—"),row.name.as_deref().unwrap_or("—")));
                                ui.label(format!("{} → {}",row.previous_site.as_deref().unwrap_or("—"),row.site.as_deref().unwrap_or("—")));ui.end_row();
                            }
                        });
                    });
                    if !preview.can_apply { ui.label("Конфликты блокируют весь список. Исправьте вход и пересчитайте preview."); }
                    if view.apply_attempted {
                        ui.label("Apply был отправлен. При потере ответа повторите тот же запрос: сервер вернёт сохранённый результат без повторной записи.");
                    } else {
                        ui.label("При изменениях backend создаст новый приватный backup рядом со своей базой. Клиентский путь серверу не передаётся.");
                    }
                    ui.add_enabled_ui(!view.busy && preview.can_apply && self.can_manage_devices(),|ui| {
                        let approve = ui.checkbox(&mut view.approve,"Подтверждаю изменение реестра по этому preview (без скана)");
                        self.probes.insert("discovery_approve",approve.rect);
                        let valid = view.apply_attempted || preview.expires_at > chrono::Utc::now();
                        if !valid { ui.label("Preview истёк — выполните новый предпросмотр."); }
                        let apply = ui.add_enabled(view.approve && valid,egui::Button::new(if view.apply_attempted {"Повторить тот же apply / получить результат"}else{"Применить с резервной копией"}));
                        self.probes.insert("discovery_apply",apply.rect);
                        if apply.clicked() && self.submit(Command::ApplyDiscovery { token:view.token,
                            input:ApplyDiscovery { preview_id:preview.id,confirmed:true } }) {
                            view.busy=true; view.apply_attempted=true; view.error=None;
                        }
                    });
                }
            });
            ui.separator();
            let cancel = ui.add_enabled(!view.busy,egui::Button::new(if view.apply_attempted && view.result.is_none() {"Закрыть — не отменяет apply"}else{"Закрыть"}));
            self.probes.insert("discovery_close",cancel.rect);
            if cancel.clicked() { close=true; }
        });
        if !view.busy && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            close = true;
        }
        if close {
            view.open = false;
            // Retain an ambiguous apply ID for reopening/retry; closing is not cancellation.
            if !view.apply_attempted {
                view.invalidate();
                self.submit(Command::DiscardDiscovery);
            }
        }
        self.discovery = view;
    }
}
