use eframe::egui::{self, Ui};
use orkestr_domain::*;
use std::collections::BTreeSet;

#[derive(Clone)]
pub struct GroupEditor {
    pub draft: Collection,
    pub expected: Option<u64>,
}
impl GroupEditor {
    pub fn new(parent: Option<CollectionId>) -> Self {
        Self {
            draft: Collection {
                id: CollectionId::new(),
                parent_id: parent,
                name: "Новая группа".into(),
                revision: 0,
                kind: CollectionKind::Dynamic(default_rule()),
            },
            expected: None,
        }
    }
    pub fn edit(c: &Collection) -> Self {
        Self {
            draft: c.clone(),
            expected: Some(c.revision),
        }
    }
}
pub fn default_rule() -> Expression {
    Expression::Ram {
        op: Compare::Ge,
        bytes: 8 * GIB,
    }
}

pub fn valid_parents(
    snapshot: &InventorySnapshot,
    id: CollectionId,
) -> Vec<(CollectionId, String)> {
    let paths = snapshot.paths().unwrap_or_default();
    snapshot
        .collections
        .iter()
        .filter(|c| matches!(c.kind, CollectionKind::Folder))
        .filter(|c| {
            let mut seen = BTreeSet::from([id]);
            let mut node = Some(c.id);
            while let Some(current) = node {
                if !seen.insert(current) {
                    return false;
                }
                node = snapshot
                    .collections
                    .iter()
                    .find(|c| c.id == current)
                    .and_then(|c| c.parent_id);
            }
            true
        })
        .map(|c| {
            (
                c.id,
                paths.get(&c.id).cloned().unwrap_or_else(|| c.name.clone()),
            )
        })
        .collect()
}

pub fn group_form(ui: &mut Ui, editor: &mut GroupEditor, snapshot: &InventorySnapshot) {
    ui.label("Название");
    ui.text_edit_singleline(&mut editor.draft.name);
    ui.label("Родительская папка / перенос");
    egui::ComboBox::from_id_salt("parent")
        .width(ui.available_width().min(360.))
        .truncate()
        .selected_text(
            editor
                .draft
                .parent_id
                .and_then(|id| snapshot.collections.iter().find(|c| c.id == id))
                .map_or("Корневой уровень", |c| &c.name),
        )
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut editor.draft.parent_id, None, "Корневой уровень");
            for (id, path) in valid_parents(snapshot, editor.draft.id) {
                ui.selectable_value(&mut editor.draft.parent_id, Some(id), path);
            }
        });
    let mut kind = match editor.draft.kind {
        CollectionKind::Folder => 0,
        CollectionKind::Static(_) => 1,
        CollectionKind::Dynamic(_) => 2,
    };
    let old = kind;
    ui.horizontal_wrapped(|ui| {
        ui.selectable_value(&mut kind, 0, "Папка");
        ui.selectable_value(&mut kind, 1, "Ручная");
        ui.selectable_value(&mut kind, 2, "Умная");
    });
    if kind != old {
        editor.draft.kind = match kind {
            0 => CollectionKind::Folder,
            1 => CollectionKind::Static(BTreeSet::new()),
            _ => CollectionKind::Dynamic(default_rule()),
        };
    }
    ui.separator();
    match &mut editor.draft.kind {
        CollectionKind::Folder => {
            ui.label("Состав папки — объединение всех подгрупп без повторов. Фильтр детям не наследуется.");
        }
        CollectionKind::Static(members) => {
            ui.label("Участники сохраняются по DeviceId. Изменение скана не меняет ручной состав.");
            egui::ScrollArea::vertical()
                .max_height(240.)
                .id_salt("static_members")
                .show(ui, |ui| {
                    for r in &snapshot.devices {
                        let mut selected = members.contains(&r.device.id);
                        if ui.checkbox(&mut selected, &r.device.host).changed() {
                            if selected {
                                members.insert(r.device.id);
                            } else {
                                members.remove(&r.device.id);
                            }
                        }
                    }
                    if snapshot.devices.is_empty() {
                        ui.label("Реестр пуст. Сначала импортируйте выбранную копию Inventory.");
                    }
                });
        }
        CollectionKind::Dynamic(expression) => {
            ui.label("Условия по последнему скану. Отсутствующие/устаревшие данные → Unknown.");
            ui.horizontal_wrapped(|ui| {
                ui.label("RAM, GiB:");
                for (label, rule) in [
                    (
                        "≥16",
                        Expression::Ram {
                            op: Compare::Ge,
                            bytes: 16 * GIB,
                        },
                    ),
                    ("≥8", default_rule()),
                    (
                        "<8",
                        Expression::Ram {
                            op: Compare::Lt,
                            bytes: 8 * GIB,
                        },
                    ),
                    (
                        "8–<16",
                        Expression::And(vec![
                            default_rule(),
                            Expression::Ram {
                                op: Compare::Lt,
                                bytes: 16 * GIB,
                            },
                        ]),
                    ),
                ] {
                    if ui.button(label).clicked() {
                        *expression = rule;
                    }
                }
            });
            expression_form(ui, expression, 0);
        }
    }
}

fn node_kind(e: &Expression) -> usize {
    match e {
        Expression::And(_) => 0,
        Expression::Or(_) => 1,
        Expression::Ram { .. } => 2,
        Expression::RamUnknown => 3,
        Expression::Text { .. } => 4,
        Expression::Software(_) => 5,
        Expression::IsUnknown(_) => 6,
    }
}
pub fn expression_form(ui: &mut Ui, e: &mut Expression, depth: usize) {
    if depth > 16 {
        ui.label("Превышен предел вложенности");
        return;
    }
    let mut kind = node_kind(e);
    let old = kind;
    egui::ComboBox::from_id_salt("node_kind")
        .selected_text(
            [
                "И — все условия",
                "ИЛИ — любое условие",
                "RAM",
                "RAM неизвестна",
                "Текстовое поле",
                "Установленное ПО",
                "Результат Unknown",
            ][kind],
        )
        .show_ui(ui, |ui| {
            for (i, label) in [
                "И — все условия",
                "ИЛИ — любое условие",
                "RAM",
                "RAM неизвестна",
                "Текстовое поле",
                "Установленное ПО",
                "Результат Unknown",
            ]
            .iter()
            .enumerate()
            {
                ui.selectable_value(&mut kind, i, *label);
            }
        });
    if kind != old {
        *e = match kind {
            0 => Expression::And(vec![default_rule()]),
            1 => Expression::Or(vec![default_rule()]),
            2 => default_rule(),
            3 => Expression::RamUnknown,
            4 => Expression::Text {
                field: TextField::Os,
                op: TextOp::Contains,
                value: "Windows".into(),
            },
            5 => Expression::Software(SoftwareFilter {
                application_id: None,
                name: Some(String::new()),
                publisher: None,
                channel: None,
                architecture: None,
                version: None,
            }),
            _ => Expression::IsUnknown(Box::new(default_rule())),
        };
    }
    match e {
        Expression::And(items) | Expression::Or(items) => {
            let mut remove = None;
            for (i, item) in items.iter_mut().enumerate() {
                ui.push_id(i, |ui| {
                    ui.group(|ui| {
                        ui.horizontal(|ui| {
                            ui.strong(format!("Условие {}", i + 1));
                            if ui.small_button("Удалить условие").clicked() {
                                remove = Some(i);
                            }
                        });
                        expression_form(ui, item, depth + 1);
                    });
                });
            }
            if let Some(i) = remove {
                items.remove(i);
            }
            if ui
                .add_enabled(
                    items.len() < 32 && depth < 15,
                    egui::Button::new("+ Условие / вложенная группа"),
                )
                .clicked()
            {
                items.push(default_rule());
            }
        }
        Expression::Ram { op, bytes } => {
            ui.horizontal_wrapped(|ui| {
                compare_form(ui, op);
                ui.add(
                    egui::DragValue::new(bytes)
                        .speed(GIB as f64 / 16.)
                        .range(0..=u64::MAX),
                );
                ui.label("байт installed physical RAM");
            });
            ui.small(format!(
                "{:.3} GiB (не usable/free)",
                *bytes as f64 / GIB as f64
            ));
        }
        Expression::RamUnknown => {
            ui.label("Нет точных bytes RAM или скан устарел/не выполнен.");
        }
        Expression::Text { field, op, value } => {
            egui::ComboBox::from_id_salt("text_field")
                .selected_text(text_label(*field))
                .show_ui(ui, |ui| {
                    for f in [
                        TextField::Host,
                        TextField::Name,
                        TextField::Domain,
                        TextField::Ou,
                        TextField::Os,
                        TextField::Source,
                        TextField::Site,
                    ] {
                        ui.selectable_value(field, f, text_label(f));
                    }
                });
            egui::ComboBox::from_id_salt("text_op")
                .selected_text(text_op_label(*op))
                .show_ui(ui, |ui| {
                    for o in [
                        TextOp::Equals,
                        TextOp::NotEquals,
                        TextOp::Contains,
                        TextOp::NotContains,
                        TextOp::StartsWith,
                        TextOp::IsEmpty,
                    ] {
                        ui.selectable_value(op, o, text_op_label(o));
                    }
                });
            if *op != TextOp::IsEmpty {
                ui.label("Значение (без учёта регистра)");
                ui.text_edit_singleline(value);
            }
        }
        Expression::Software(f) => {
            ui.small("Все поля ниже относятся к одной записи ПО. Импортированный HKLM-скан имеет Partial coverage.");
            optional_text(ui, "Название продукта", &mut f.name);
            optional_text(ui, "Издатель", &mut f.publisher);
            optional_text(ui, "Канал", &mut f.channel);
            optional_text(ui, "Архитектура", &mut f.architecture);
            if let Some(id) = f.application_id {
                ui.monospace(format!("ApplicationId: {id}"));
            }
            let mut use_version = f.version.is_some();
            if ui.checkbox(&mut use_version, "Сравнивать версию").changed() {
                f.version =
                    use_version.then(|| (Compare::Lt, VersionOperand::Literal("1.0".into())));
            }
            if let Some((op, operand)) = &mut f.version {
                compare_form(ui, op);
                match operand {
                    VersionOperand::Literal(v) => {
                        ui.label("Версия-числовые сегменты");
                        ui.text_edit_singleline(v);
                    }
                    VersionOperand::Library { approved, .. } => {
                        ui.label(if *approved {
                            "Последняя одобренная (M4)"
                        } else {
                            "Последняя известная в библиотеке (M4)"
                        });
                        ui.small(
                            "Ссылка сохранена, но живая библиотека в M1 недоступна → Unknown.",
                        );
                    }
                }
            }
        }
        Expression::IsUnknown(inner) => {
            ui.indent("unknown", |ui| expression_form(ui, inner, depth + 1));
        }
    }
}
fn optional_text(ui: &mut Ui, label: &str, value: &mut Option<String>) {
    ui.label(label);
    let mut text = value.clone().unwrap_or_default();
    if ui.text_edit_singleline(&mut text).changed() {
        *value = (!text.is_empty()).then_some(text);
    }
}
fn compare_form(ui: &mut Ui, op: &mut Compare) {
    egui::ComboBox::from_id_salt("numeric_op")
        .selected_text(compare_label(*op))
        .show_ui(ui, |ui| {
            for o in [
                Compare::Eq,
                Compare::Ne,
                Compare::Gt,
                Compare::Ge,
                Compare::Lt,
                Compare::Le,
            ] {
                ui.selectable_value(op, o, compare_label(o));
            }
        });
}
pub fn compare_label(op: Compare) -> &'static str {
    match op {
        Compare::Eq => "=",
        Compare::Ne => "≠",
        Compare::Gt => ">",
        Compare::Ge => "≥",
        Compare::Lt => "<",
        Compare::Le => "≤",
    }
}
fn text_label(field: TextField) -> &'static str {
    match field {
        TextField::Host => "Имя хоста",
        TextField::Name => "Имя ПК",
        TextField::Domain => "Домен",
        TextField::Ou => "OU",
        TextField::Os => "ОС",
        TextField::Source => "Источник",
        TextField::Site => "Площадка",
    }
}
fn text_op_label(op: TextOp) -> &'static str {
    match op {
        TextOp::Equals => "Равно",
        TextOp::NotEquals => "Не равно",
        TextOp::Contains => "Содержит",
        TextOp::NotContains => "Не содержит",
        TextOp::StartsWith => "Начинается с",
        TextOp::IsEmpty => "Пусто",
    }
}
