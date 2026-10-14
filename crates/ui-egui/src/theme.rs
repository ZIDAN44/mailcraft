//! Design tokens (light and dark) and the egui style.

use std::sync::Arc;

use egui::{Color32, CornerRadius, FontData, FontDefinitions, FontFamily, Stroke, Visuals};

/// MailCraft's app colour: a deep teal, distinct from the other Craft apps.
pub const APP_COLOR: Color32 = Color32::from_rgb(0x0E, 0x7C, 0x86);

#[derive(Clone, Copy, Debug)]
pub struct Tokens {
    pub dark: bool,
    /// Window background (behind the panes).
    pub bg: Color32,
    /// Panes: message list, reading pane.
    pub surface: Color32,
    /// The folder pane and toolbar.
    pub chrome: Color32,
    pub border: Color32,
    pub text: Color32,
    pub dim: Color32,
    pub faint: Color32,
    pub accent: Color32,
    /// Text on an accent fill.
    pub on_accent: Color32,
    /// Selection background.
    pub accent_soft: Color32,
    pub hover: Color32,
    pub danger: Color32,
    pub flag: Color32,
    pub ok: Color32,
    pub warn: Color32,
}

impl Tokens {
    pub fn light() -> Tokens {
        Tokens {
            dark: false,
            bg: Color32::from_rgb(0xF3, 0xF4, 0xF6),
            surface: Color32::WHITE,
            chrome: Color32::from_rgb(0xF7, 0xF8, 0xFA),
            border: Color32::from_rgb(0xE1, 0xE4, 0xE8),
            text: Color32::from_rgb(0x1F, 0x23, 0x28),
            dim: Color32::from_rgb(0x5B, 0x63, 0x6E),
            faint: Color32::from_rgb(0x8C, 0x93, 0x9D),
            accent: APP_COLOR,
            on_accent: Color32::WHITE,
            accent_soft: Color32::from_rgb(0xDC, 0xF0, 0xF2),
            hover: Color32::from_rgb(0xEE, 0xF1, 0xF4),
            danger: Color32::from_rgb(0xC4, 0x2B, 0x1C),
            flag: Color32::from_rgb(0xC5, 0x3B, 0x2F),
            ok: Color32::from_rgb(0x1E, 0x8E, 0x3E),
            warn: Color32::from_rgb(0xB2, 0x6B, 0x00),
        }
    }

    pub fn dark() -> Tokens {
        Tokens {
            dark: true,
            bg: Color32::from_rgb(0x17, 0x18, 0x1B),
            surface: Color32::from_rgb(0x22, 0x24, 0x28),
            chrome: Color32::from_rgb(0x1C, 0x1E, 0x21),
            border: Color32::from_rgb(0x34, 0x37, 0x3C),
            text: Color32::from_rgb(0xE8, 0xEA, 0xED),
            dim: Color32::from_rgb(0xA9, 0xAF, 0xB7),
            faint: Color32::from_rgb(0x7C, 0x83, 0x8C),
            accent: Color32::from_rgb(0x3F, 0xB8, 0xC4),
            on_accent: Color32::from_rgb(0x0B, 0x1E, 0x21),
            accent_soft: Color32::from_rgb(0x1D, 0x3B, 0x40),
            hover: Color32::from_rgb(0x2B, 0x2E, 0x33),
            danger: Color32::from_rgb(0xF0, 0x6A, 0x5C),
            flag: Color32::from_rgb(0xF0, 0x6A, 0x5C),
            ok: Color32::from_rgb(0x5C, 0xC9, 0x7B),
            warn: Color32::from_rgb(0xE8, 0xA8, 0x3C),
        }
    }

    pub fn for_dark(dark: bool) -> Tokens {
        if dark { Tokens::dark() } else { Tokens::light() }
    }
}

/// Avatar colours, picked by a hash of the address so a sender always gets the same one.
pub fn avatar_color(key: &str, dark: bool) -> Color32 {
    const LIGHT: [Color32; 10] = [
        Color32::from_rgb(0x0E, 0x7C, 0x86),
        Color32::from_rgb(0x8E, 0x44, 0xAD),
        Color32::from_rgb(0xC0, 0x56, 0x2B),
        Color32::from_rgb(0x2E, 0x7D, 0x32),
        Color32::from_rgb(0x34, 0x5E, 0xC9),
        Color32::from_rgb(0xAD, 0x1F, 0x5C),
        Color32::from_rgb(0x6D, 0x6A, 0x12),
        Color32::from_rgb(0x00, 0x7A, 0x5E),
        Color32::from_rgb(0x7B, 0x4F, 0x2C),
        Color32::from_rgb(0x52, 0x5C, 0xB8),
    ];
    let mut h: u32 = 2_166_136_261;
    for b in key.to_lowercase().bytes() {
        h = (h ^ u32::from(b)).wrapping_mul(16_777_619);
    }
    let c = LIGHT.get((h % 10) as usize).copied().unwrap_or(APP_COLOR);
    if dark { c.gamma_multiply(1.25) } else { c }
}

/// Account marker colours (the account's `color` index).
pub fn account_color(i: u8) -> Color32 {
    const C: [Color32; 8] = [
        Color32::from_rgb(0x0E, 0x7C, 0x86),
        Color32::from_rgb(0xC0, 0x56, 0x2B),
        Color32::from_rgb(0x8E, 0x44, 0xAD),
        Color32::from_rgb(0x2E, 0x7D, 0x32),
        Color32::from_rgb(0x34, 0x5E, 0xC9),
        Color32::from_rgb(0xAD, 0x1F, 0x5C),
        Color32::from_rgb(0x6D, 0x6A, 0x12),
        Color32::from_rgb(0x52, 0x5C, 0xB8),
    ];
    C.get(usize::from(i) % C.len()).copied().unwrap_or(APP_COLOR)
}

/// Category colours (the category's `color` index).
pub fn category_color(i: u8, dark: bool) -> Color32 {
    const C: [Color32; 8] = [
        Color32::from_rgb(0xD1, 0x34, 0x38),
        Color32::from_rgb(0xE0, 0x7A, 0x1F),
        Color32::from_rgb(0x2E, 0x8B, 0x57),
        Color32::from_rgb(0x2F, 0x6D, 0xD4),
        Color32::from_rgb(0x8A, 0x4F, 0xC7),
        Color32::from_rgb(0xC9, 0xA2, 0x00),
        Color32::from_rgb(0x1C, 0x95, 0x9F),
        Color32::from_rgb(0xB0, 0x4A, 0x8C),
    ];
    let c = C.get(usize::from(i) % C.len()).copied().unwrap_or(APP_COLOR);
    if dark { c.gamma_multiply(1.15) } else { c }
}

/// Apply the tokens to egui's style.
pub fn apply(ctx: &egui::Context, t: &Tokens) {
    let mut v = if t.dark { Visuals::dark() } else { Visuals::light() };
    v.panel_fill = t.surface;
    v.window_fill = t.surface;
    // Text boxes: a touch darker than the pane, so fields read as fields.
    v.extreme_bg_color = if t.dark { Color32::from_rgb(0x1A, 0x1C, 0x1F) } else { Color32::WHITE };
    v.faint_bg_color = t.chrome;
    v.window_stroke = Stroke::new(1.0, t.border);
    v.selection.bg_fill = t.accent.gamma_multiply(if t.dark { 0.55 } else { 0.35 });
    v.selection.stroke = Stroke::new(1.0, t.accent);
    v.hyperlink_color = t.accent;
    v.override_text_color = Some(t.text);
    let r = CornerRadius::same(6);
    v.window_corner_radius = CornerRadius::same(10);
    v.menu_corner_radius = CornerRadius::same(8);
    for w in [&mut v.widgets.noninteractive, &mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
        w.corner_radius = r;
    }
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, t.border);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, t.text);
    v.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
    v.widgets.inactive.bg_fill = if t.dark { Color32::from_rgb(0x2C, 0x2F, 0x34) } else { Color32::from_rgb(0xF1, 0xF3, 0xF5) };
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, t.text);
    // A hairline so text boxes and drop-downs read as fields on any background.
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, if t.dark { Color32::from_rgb(0x3E, 0x42, 0x48) } else { Color32::from_rgb(0xCF, 0xD4, 0xDA) });
    v.widgets.hovered.weak_bg_fill = t.hover;
    v.widgets.hovered.bg_fill = t.hover;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, t.border);
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, t.text);
    v.widgets.active.weak_bg_fill = t.accent_soft;
    v.widgets.active.bg_fill = t.accent_soft;
    v.widgets.active.fg_stroke = Stroke::new(1.0, t.text);
    v.widgets.open.weak_bg_fill = t.accent_soft;
    ctx.set_visuals(v);
    ctx.global_style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(8.0, 6.0);
        s.spacing.button_padding = egui::vec2(8.0, 4.0);
        s.spacing.interact_size.y = 26.0;
        s.spacing.menu_margin = egui::Margin::same(6);
        s.text_styles.insert(egui::TextStyle::Body, egui::FontId::proportional(14.0));
        s.text_styles.insert(egui::TextStyle::Button, egui::FontId::proportional(13.5));
        s.text_styles.insert(egui::TextStyle::Small, egui::FontId::proportional(12.0));
        s.text_styles.insert(egui::TextStyle::Heading, egui::FontId::proportional(20.0));
    });
}

/// Use an installed system UI font when the app found one (never bundled: see AGENTS.md).
pub fn install_fonts(ctx: &egui::Context, regular: Option<Vec<u8>>, bold: Option<Vec<u8>>) {
    install_fonts_with(ctx, regular, bold, None);
}

/// [`install_fonts`] with a fallback for Chinese and Japanese (a font file and its index in a
/// collection), used after the others.
pub fn install_fonts_with(ctx: &egui::Context, regular: Option<Vec<u8>>, bold: Option<Vec<u8>>, cjk: Option<(Vec<u8>, u32)>) {
    let mut fonts = FontDefinitions::default();
    if let Some((bytes, index)) = cjk {
        let mut data = FontData::from_owned(bytes);
        data.index = index;
        fonts.font_data.insert("cjk".into(), Arc::new(data));
        if let Some(f) = fonts.families.get_mut(&FontFamily::Proportional) {
            f.push("cjk".into());
        }
        if let Some(f) = fonts.families.get_mut(&FontFamily::Monospace) {
            f.push("cjk".into());
        }
    }
    if let Some(bytes) = regular {
        fonts.font_data.insert("system".into(), Arc::new(FontData::from_owned(bytes)));
        if let Some(f) = fonts.families.get_mut(&FontFamily::Proportional) {
            f.insert(0, "system".into());
        }
    }
    let bold_family = FontFamily::Name("bold".into());
    let mut bold_list = Vec::new();
    if let Some(bytes) = bold {
        fonts.font_data.insert("system-bold".into(), Arc::new(FontData::from_owned(bytes)));
        bold_list.push("system-bold".to_string());
    }
    bold_list.extend(fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default());
    fonts.families.insert(bold_family, bold_list);
    ctx.set_fonts(fonts);
}

pub fn bold(size: f32) -> egui::FontId {
    egui::FontId::new(size, FontFamily::Name("bold".into()))
}

pub fn regular(size: f32) -> egui::FontId {
    egui::FontId::proportional(size)
}
