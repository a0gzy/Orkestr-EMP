use eframe::egui::{self, Ui};
use orkestr_domain::*;
use orkestr_protocol::{DeviceInventoryStatus, InventoryRunStatus, InventoryState};
use serde_json::Value;

pub const TABS: &[&str] = &[
    "Обзор",
    "Оборудование",
    "ПО",
    "ОС и обновления",
    "Пользователи",
    "Процессы и службы",
    "Сеть и ресурсы",
    "Компоненты и задачи",
    "Все данные",
    "Провайдеры",
];
pub fn device_card(
    ui: &mut Ui,
    record: &DeviceRecord,
    tab: &mut usize,
    status: Option<&DeviceInventoryStatus>,
    error: Option<&str>,
) {
    if let Some(error) = error {
        ui.colored_label(
            crate::theme::Palette::get(ui.visuals().dark_mode).error,
            format!("Статусы недоступны: {error}. Сохранённый скан не удалён."),
        );
    } else if let Some(status) = status {
        if let Some(latest) = &status.latest {
            ui.label(format!(
                "Последняя попытка: {} · {} UTC",
                inventory_state_label(latest.state),
                latest.started_at.format("%Y-%m-%d %H:%M:%S")
            ));
            if record.scan.as_ref().is_some_and(|s| s.id.0 != latest.id) {
                ui.strong(
                    "Ниже показан прежний сохранённый снимок, не результат последней попытки.",
                );
            }
        } else {
            ui.small("Структурированных попыток M3 в реестре нет. Legacy-данные сохранены без выдуманных статусов.");
        }
    } else {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label("Загрузка статусов последней попытки…");
        });
    }
    ui.horizontal_wrapped(|ui| {
        for (i, name) in TABS.iter().enumerate() {
            ui.selectable_value(tab, i, *name);
        }
    });
    ui.separator();
    if *tab == 9 {
        if let Some(status) = status {
            if let Some(latest) = &status.latest {
                provider_status(ui, "Последняя попытка", latest);
            }
            if let Some(head) = &status.head {
                if status.latest.as_ref().is_none_or(|r| r.id != head.id) {
                    provider_status(ui, "Провайдеры сохранённого снимка", head);
                }
            }
            if status.latest.is_none() {
                ui.label("Отчёта провайдеров M3 нет; пустой раздел не означает успешный сбор.");
            }
        }
        return;
    }
    egui::ScrollArea::both().id_salt("device_card_scroll").auto_shrink([false,false]).show(ui,|ui| {
        let d=&record.device;
        if *tab==0 {
            egui::Grid::new("device_summary").num_columns(2).spacing([24.,10.]).show(ui,|ui| {
                for (key,value) in [("DeviceId",d.id.to_string()),("Имя хоста",d.host.clone()),("Псевдонимы",d.aliases.join(", ")),("AD GUID",d.ad_guid.map_or_else(||"—".into(),|v|v.to_string())),("ОС",d.os.clone()),("Домен",d.domain.clone()),("OU",d.ou.clone()),("Источник",d.source.clone()),("Площадка",d.site.clone()),("Отключён",if d.disabled {"Да".into()}else{"Нет".into()})] {ui.label(key);ui.label(if value.is_empty(){"—"}else{&value});ui.end_row();}
            });
            if let Some(scan)=&record.scan {ui.separator();ui.label(format!("Скан: {} · {}",scan.scanned_at,scan.id));ui.label(format!("RAM: {}",ram_label(Some(scan))));ui.label(format!("Полнота ПО: {:?}",scan.software_state));}
            else {ui.label("Сканирование не выполнялось. Данные неизвестны.");}
            egui::CollapsingHeader::new("Метаданные каталога / источника").show(ui,|ui|value_ui(ui,&d.inventory_metadata,0));
        } else if let Some(scan)=&record.scan {
            ui.label(format!("Данные скана {} · {}",scan.id,scan.scanned_at));
            match *tab {
                1=>sections(ui,&scan.details,&["ComputerSystem","BIOS","Manufacturer","Model","Serial","CPU","CPUs","MemoryGB","MemoryModules","BIOSVersion","BIOSManufacturer","Disks","DiskDrives","Displays","Hardware"]),
                2=>{
                    ui.label(format!("Полнота ПО: {:?}. Partial не доказывает отсутствие приложения.",scan.software_state));
                    software_table(ui,&scan.software);
                    sections(ui,&scan.details,&["SoftwareWarning"]);
                }
                3=>sections(ui,&scan.details,&["OperatingSystem","OSVersion","Architecture","Build","OSSerial","Installed","SystemDrive","LastBoot","Uptime","HotFixes"]),
                4=>sections(ui,&scan.details,&["User","LocalUsers","LocalGroups"]),
                5=>sections(ui,&scan.details,&["Processes","Services"]),
                6=>sections(ui,&scan.details,&["HostName","NICs","Printers","Shares"]),
                7=>sections(ui,&scan.details,&["Environment","WindowsFeatures","Startup","TaskSchedules"]),
                _=>{ui.label("Приватные данные. Не публикуйте снимок: Environment/скрипты могут содержать чувствительную информацию.");value_ui(ui,&scan.details,0);}
            }
            if *tab!=8 {egui::CollapsingHeader::new("Предупреждения провайдеров").default_open(true).show(ui,|ui| {
                if let Some(w)=scan.details.get("Warnings"){value_ui(ui,w,0);}else if status.is_some_and(|s|s.head.is_some()){ui.label("Статусы этого снимка доступны во вкладке «Провайдеры».");}else{ui.label("Статусы дополнительных провайдеров не переданы источником.");}
            });}
        } else {ui.label("Нет скана. Этот раздел не собран — пустое состояние не означает успешную проверку.");}
    });
}
pub(crate) fn inventory_state_label(state: InventoryState) -> &'static str {
    match state {
        InventoryState::Complete => "Полный результат",
        InventoryState::Partial => "Частичный результат",
        InventoryState::Failed => "Ошибка",
        InventoryState::Unknown => "Неизвестно / данные не собраны",
    }
}
fn provider_status(ui: &mut Ui, title: &str, run: &InventoryRunStatus) {
    ui.push_id(run.id, |ui| {
        ui.strong(title);
        ui.label(format!(
            "{} · {} — {} UTC",
            inventory_state_label(run.state),
            run.started_at,
            run.finished_at
        ));
        egui::ScrollArea::both()
            .id_salt("provider_status")
            .max_height(320.)
            .show(ui, |ui| {
                egui::Grid::new("providers").striped(true).show(ui, |ui| {
                    for label in ["Провайдер", "Состояние", "Строк", "Код"]
                    {
                        ui.strong(label);
                    }
                    ui.end_row();
                    for p in &run.providers {
                        ui.label(p.provider.section());
                        ui.label(match p.state {
                            ProviderState::Complete => "Полный",
                            ProviderState::Partial => "Частично",
                            ProviderState::Failed => "Ошибка",
                            ProviderState::NotCollected => "Не собран",
                        });
                        ui.label(p.rows.to_string());
                        ui.label(p.failure.map_or_else(|| "—".into(), |f| format!("{f:?}")));
                        ui.end_row();
                    }
                });
            });
    });
}
pub fn ram_label(scan: Option<&DeviceScan>) -> String {
    scan.and_then(|s| s.installed_ram_bytes).map_or_else(
        || "Unknown".into(),
        |b| format!("{:.2} GiB", b as f64 / GIB as f64),
    )
}
pub fn software_table(ui: &mut Ui, rows: &[InstalledSoftware]) {
    if rows.is_empty() {
        ui.label("Записей ПО нет; учитывайте полноту и дату скана.");
        return;
    }
    egui::Grid::new("software")
        .num_columns(5)
        .striped(true)
        .spacing([20., 8.])
        .show(ui, |ui| {
            for label in [
                "Приложение",
                "Издатель",
                "Установленная версия",
                "Канал",
                "Архитектура",
            ] {
                ui.strong(label);
            }
            ui.end_row();
            for app in rows {
                for v in [
                    app.name.as_str(),
                    &app.publisher,
                    &app.version,
                    app.channel.as_deref().unwrap_or("—"),
                    app.architecture.as_deref().unwrap_or("—"),
                ] {
                    ui.label(v);
                }
                ui.end_row();
            }
        });
}
fn sections(ui: &mut Ui, details: &Value, keys: &[&str]) {
    for key in keys {
        ui.push_id(*key, |ui| {
            egui::CollapsingHeader::new(*key)
                .default_open(details.get(*key).is_some())
                .show(ui, |ui| {
                    if let Some(v) = details.get(*key) {
                        value_ui(ui, v, 0);
                    } else {
                        crate::theme::muted(
                            ui,
                            "Не собрано / провайдер не представлен в этом скане",
                        );
                    }
                });
        });
    }
}
pub fn value_ui(ui: &mut Ui, value: &Value, depth: usize) {
    if depth >= 24 {
        ui.label("Предел отображения вложенности (исходные данные сохранены)");
        return;
    }
    match value {
        Value::Object(map) => {
            for (key, v) in map {
                ui.push_id(key, |ui| {
                    if v.is_array() || v.is_object() {
                        egui::CollapsingHeader::new(key)
                            .default_open(depth == 0)
                            .show(ui, |ui| value_ui(ui, v, depth + 1));
                    } else {
                        ui.horizontal_wrapped(|ui| {
                            ui.strong(key);
                            primitive(ui, v);
                        });
                    }
                });
            }
            if map.is_empty() {
                crate::theme::muted(ui, "Нет полей");
            }
        }
        Value::Array(rows) => {
            if rows.is_empty() {
                crate::theme::muted(
                    ui,
                    "Нет записей в источнике; статус провайдера смотрите отдельно",
                );
            }
            for (i, row) in rows.iter().enumerate() {
                ui.push_id(i, |ui| {
                    if row.is_object() || row.is_array() {
                        egui::CollapsingHeader::new(format!("Запись {}", i + 1))
                            .default_open(rows.len() <= 3)
                            .show(ui, |ui| value_ui(ui, row, depth + 1));
                    } else {
                        primitive(ui, row);
                    }
                });
            }
        }
        _ => primitive(ui, value),
    }
}
fn primitive(ui: &mut Ui, value: &Value) {
    ui.add(
        egui::Label::new(match value {
            Value::String(s) => s.clone(),
            Value::Null => "Unknown / null".into(),
            _ => value.to_string(),
        })
        .wrap()
        .selectable(true),
    );
}
