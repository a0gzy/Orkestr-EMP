use super::theme::*;
use eframe::egui::Color32;
fn luminance(c: Color32) -> f32 {
    let linear = |v: u8| {
        let v = v as f32 / 255.;
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(c.r()) + 0.7152 * linear(c.g()) + 0.0722 * linear(c.b())
}
#[test]
fn text_tokens_have_aa_contrast_on_both_surfaces() {
    for dark in [false, true] {
        let p = Palette::get(dark);
        for foreground in [p.ink, p.muted, p.accent] {
            for background in [p.bg, p.surface] {
                let a = luminance(foreground);
                let b = luminance(background);
                let contrast = (a.max(b) + 0.05) / (a.min(b) + 0.05);
                assert!(contrast >= 4.5, "dark={dark}, contrast={contrast}");
            }
        }
    }
}
