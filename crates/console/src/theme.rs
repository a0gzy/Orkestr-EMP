//! Hallmark · pre-emit critique: P4 H4 E4 S5 R5 V4
//! Approved A: Workbench / Cobalt / side rail / status line. tokens.css is the source.
use eframe::egui::{self, Color32, FontFamily, FontId, Stroke, TextStyle};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThemeChoice {
    Light,
    Dark,
    #[default]
    System,
}
impl ThemeChoice {
    pub fn label(self) -> &'static str {
        match self {
            Self::Light => "Светлая",
            Self::Dark => "Тёмная",
            Self::System => "Системная",
        }
    }
    pub fn preference(self) -> egui::ThemePreference {
        match self {
            Self::Light => egui::ThemePreference::Light,
            Self::Dark => egui::ThemePreference::Dark,
            Self::System => egui::ThemePreference::System,
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Preferences {
    pub theme: ThemeChoice,
    pub zoom: f32,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            theme: ThemeChoice::System,
            zoom: 1.0,
        }
    }
}

#[derive(Clone, Copy)]
pub struct Palette {
    pub bg: Color32,
    pub surface: Color32,
    pub soft: Color32,
    pub ink: Color32,
    pub muted: Color32,
    pub line: Color32,
    pub accent: Color32,
    pub accent_soft: Color32,
    pub error: Color32,
    pub warn: Color32,
}
impl Palette {
    pub fn get(dark: bool) -> Self {
        if dark {
            Self {
                bg: oklch(0.19, 0.012, 255.),
                surface: oklch(0.23, 0.014, 255.),
                soft: oklch(0.27, 0.016, 255.),
                ink: oklch(0.94, 0.008, 255.),
                muted: oklch(0.75, 0.014, 255.),
                line: oklch(0.36, 0.018, 255.),
                accent: oklch(0.78, 0.11, 255.),
                accent_soft: oklch(0.30, 0.045, 255.),
                error: oklch(0.79, 0.12, 28.),
                warn: oklch(0.84, 0.09, 75.),
            }
        } else {
            Self {
                bg: oklch(0.97, 0.008, 255.),
                surface: oklch(0.99, 0.004, 255.),
                soft: oklch(0.95, 0.009, 255.),
                ink: oklch(0.26, 0.022, 255.),
                muted: oklch(0.47, 0.022, 255.),
                line: oklch(0.87, 0.014, 255.),
                accent: oklch(0.48, 0.18, 258.),
                accent_soft: oklch(0.94, 0.025, 258.),
                error: oklch(0.45, 0.16, 28.),
                warn: oklch(0.43, 0.09, 65.),
            }
        }
    }
}
fn oklch(l: f32, c: f32, h: f32) -> Color32 {
    let a = c * h.to_radians().cos();
    let b = c * h.to_radians().sin();
    let ll = (l + 0.39633778 * a + 0.21580376 * b).powi(3);
    let mm = (l - 0.105561346 * a - 0.06385417 * b).powi(3);
    let ss = (l - 0.08948418 * a - 1.2914855 * b).powi(3);
    let srgb = |v: f32| {
        let s = if v <= 0.0031308 {
            12.92 * v
        } else {
            1.055 * v.powf(1. / 2.4) - 0.055
        };
        (s.clamp(0., 1.) * 255.).round() as u8
    };
    Color32::from_rgb(
        srgb(4.0767417 * ll - 3.3077116 * mm + 0.23096994 * ss),
        srgb(-1.268438 * ll + 2.6097574 * mm - 0.3413194 * ss),
        srgb(-0.0041960863 * ll - 0.7034186 * mm + 1.7076147 * ss),
    )
}
pub fn install(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    for (file, name, family) in [
        ("segoeui.ttf", "Orkestr UI", FontFamily::Proportional),
        ("consola.ttf", "Orkestr Mono", FontFamily::Monospace),
    ] {
        let root = std::env::var_os("WINDIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| "C:\\Windows".into());
        if let Ok(bytes) = std::fs::read(root.join("Fonts").join(file)) {
            fonts
                .font_data
                .insert(name.into(), egui::FontData::from_owned(bytes).into());
            fonts
                .families
                .get_mut(&family)
                .unwrap()
                .insert(0, name.into());
        }
    }
    ctx.set_fonts(fonts);
    for (theme, dark) in [(egui::Theme::Light, false), (egui::Theme::Dark, true)] {
        let p = Palette::get(dark);
        let mut style = egui::Style::default();
        let mut visuals = if dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        };
        visuals.panel_fill = p.surface;
        visuals.window_fill = p.surface;
        visuals.extreme_bg_color = p.bg;
        visuals.faint_bg_color = p.soft;
        visuals.override_text_color = Some(p.ink);
        visuals.window_stroke = Stroke::new(1.0_f32, p.line);
        visuals.hyperlink_color = p.accent;
        visuals.selection.bg_fill = p.accent_soft;
        visuals.selection.stroke = Stroke::new(1.5_f32, p.accent);
        visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, p.ink);
        visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, p.line);
        visuals.widgets.inactive.bg_fill = p.soft;
        visuals.widgets.inactive.weak_bg_fill = p.soft;
        visuals.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, p.ink);
        visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, p.line);
        visuals.widgets.hovered.bg_fill = p.accent_soft;
        visuals.widgets.hovered.weak_bg_fill = p.accent_soft;
        visuals.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, p.ink);
        visuals.widgets.hovered.bg_stroke = Stroke::new(1.5_f32, p.accent);
        visuals.widgets.active.bg_fill = p.accent_soft;
        visuals.widgets.active.weak_bg_fill = p.accent_soft;
        visuals.widgets.active.fg_stroke = Stroke::new(1.0_f32, p.ink);
        visuals.widgets.active.bg_stroke = Stroke::new(1.5_f32, p.accent);
        visuals.warn_fg_color = p.warn;
        visuals.error_fg_color = p.error;
        style.visuals = visuals;
        style.spacing.item_spacing = egui::vec2(8., 8.);
        style.spacing.button_padding = egui::vec2(12., 7.);
        style.spacing.interact_size.y = 30.;
        style
            .text_styles
            .insert(TextStyle::Body, FontId::new(14., FontFamily::Proportional));
        style.text_styles.insert(
            TextStyle::Button,
            FontId::new(14., FontFamily::Proportional),
        );
        style
            .text_styles
            .insert(TextStyle::Small, FontId::new(12., FontFamily::Proportional));
        style.text_styles.insert(
            TextStyle::Heading,
            FontId::new(24., FontFamily::Proportional),
        );
        style.text_styles.insert(
            TextStyle::Monospace,
            FontId::new(13., FontFamily::Monospace),
        );
        ctx.set_style_of(theme, style);
    }
}
pub fn mark(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(34., 34.), egui::Sense::hover());
    let stroke = Stroke::new(2.3_f32, Palette::get(ui.visuals().dark_mode).accent);
    for path in [
        vec![
            (4., 23.),
            (4., 9.),
            (11., 5.),
            (18., 9.),
            (18., 23.),
            (11., 27.),
            (4., 23.),
        ],
        vec![
            (18., 9.),
            (25., 5.),
            (28., 7.),
            (28., 23.),
            (21., 27.),
            (18., 25.),
        ],
        vec![(4., 9.), (11., 13.), (18., 9.)],
        vec![(11., 13.), (11., 27.)],
    ] {
        ui.painter().add(egui::Shape::line(
            path.into_iter()
                .map(|(x, y)| rect.min + egui::vec2(x, y))
                .collect(),
            stroke,
        ));
    }
}

pub fn group_icon(ui: &mut egui::Ui, kind: &orkestr_domain::CollectionKind) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(14., 18.), egui::Sense::hover());
    let stroke = Stroke::new(1.2_f32, Palette::get(ui.visuals().dark_mode).muted);
    let points: &[(f32, f32)] = match kind {
        orkestr_domain::CollectionKind::Folder => &[
            (1., 14.),
            (1., 4.),
            (6., 4.),
            (8., 6.),
            (13., 6.),
            (13., 14.),
            (1., 14.),
        ],
        orkestr_domain::CollectionKind::Static(_) => &[
            (1., 4.),
            (13., 4.),
            (13., 15.),
            (1., 15.),
            (1., 4.),
            (4., 10.),
            (6., 12.),
            (10., 7.),
        ],
        orkestr_domain::CollectionKind::Dynamic(_) => &[
            (1., 4.),
            (13., 4.),
            (8., 10.),
            (8., 15.),
            (6., 14.),
            (6., 10.),
            (1., 4.),
        ],
    };
    ui.painter().add(egui::Shape::line(
        points
            .iter()
            .map(|(x, y)| rect.min + egui::vec2(*x, *y))
            .collect(),
        stroke,
    ));
}

pub fn muted(ui: &mut egui::Ui, text: impl Into<String>) -> egui::Response {
    ui.label(egui::RichText::new(text.into()).color(Palette::get(ui.visuals().dark_mode).muted))
}
