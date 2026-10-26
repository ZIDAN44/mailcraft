//! Design tokens (light and dark) and the egui style.

use std::sync::Arc;

use egui::{Color32, CornerRadius, FontData, FontDefinitions, FontFamily, Stroke, Visuals};

/// MailCraft's app colour: a deep teal, distinct from the other Craft apps.
pub const APP_COLOR: Color32 = Color32::from_rgb(0x0E, 0x7C, 0x86);

#[derive(Clone, Copy, Debug, PartialEq)]
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
    /// Buttons under the pointer.
    pub hover: Color32,
    /// Rows under the pointer (folders, messages, people, attachment chips): a tint of the app
    /// colour, lighter than the selection.
    pub hover_row: Color32,
    pub danger: Color32,
    pub flag: Color32,
    pub ok: Color32,
    pub warn: Color32,
    /// The top bar (search, sync, settings) and what's drawn on it (issue #8): a deep shade of
    /// the accent colour with white, or the chrome's colours when it's plain.
    pub bar: Color32,
    pub on_bar: Color32,
}

impl Tokens {
    pub fn light() -> Tokens {
        Tokens {
            dark: false,
            bg: Color32::from_rgb(0xF3, 0xF4, 0xF6),
            surface: Color32::WHITE,
            chrome: Color32::from_rgb(0xF7, 0xF8, 0xFA),
            border: Color32::from_rgb(0xD5, 0xD9, 0xDF),
            text: Color32::from_rgb(0x1F, 0x23, 0x28),
            dim: Color32::from_rgb(0x5B, 0x63, 0x6E),
            faint: Color32::from_rgb(0x84, 0x8B, 0x95),
            accent: APP_COLOR,
            on_accent: Color32::WHITE,
            accent_soft: Color32::from_rgb(0xD2, 0xEB, 0xEE),
            hover: Color32::from_rgb(0xE8, 0xEB, 0xEE),
            hover_row: Color32::from_rgb(0xE8, 0xF5, 0xF6),
            danger: Color32::from_rgb(0xC4, 0x2B, 0x1C),
            flag: Color32::from_rgb(0xC5, 0x3B, 0x2F),
            ok: Color32::from_rgb(0x1E, 0x8E, 0x3E),
            warn: Color32::from_rgb(0xB2, 0x6B, 0x00),
            bar: APP_COLOR,
            on_bar: Color32::WHITE,
        }
    }

    pub fn dark() -> Tokens {
        Tokens {
            dark: true,
            // Panes, chrome and the lines between them told apart (issue #8): the panes a step
            // lighter than the folder pane and ribbon, the borders plain to see.
            bg: Color32::from_rgb(0x12, 0x13, 0x16),
            surface: Color32::from_rgb(0x25, 0x28, 0x2C),
            chrome: Color32::from_rgb(0x19, 0x1A, 0x1D),
            border: Color32::from_rgb(0x4A, 0x4F, 0x57),
            text: Color32::from_rgb(0xE8, 0xEA, 0xED),
            dim: Color32::from_rgb(0xA9, 0xAF, 0xB7),
            faint: Color32::from_rgb(0x7C, 0x83, 0x8C),
            accent: Color32::from_rgb(0x3F, 0xB8, 0xC4),
            on_accent: Color32::from_rgb(0x0B, 0x1E, 0x21),
            accent_soft: Color32::from_rgb(0x1D, 0x3B, 0x40),
            hover: Color32::from_rgb(0x36, 0x3A, 0x41),
            hover_row: Color32::from_rgb(0x27, 0x33, 0x36),
            danger: Color32::from_rgb(0xF0, 0x6A, 0x5C),
            flag: Color32::from_rgb(0xF0, 0x6A, 0x5C),
            ok: Color32::from_rgb(0x5C, 0xC9, 0x7B),
            warn: Color32::from_rgb(0xE8, 0xA8, 0x3C),
            // The light accent with white on it read 2.3:1 (issue #8): a deep teal instead.
            bar: Color32::from_rgb(0x0B, 0x5F, 0x67),
            on_bar: Color32::WHITE,
        }
    }

    pub fn for_dark(dark: bool) -> Tokens {
        if dark { Tokens::dark() } else { Tokens::light() }
    }

    /// The tokens for a theme with an accent colour of the user's (issue #8: `None` for
    /// MailCraft's teal) and the top bar coloured or plain. Any colour is made to read: the
    /// accent is darkened (light) or lightened (dark) until text in it reads on the panes, and
    /// the selection and hover tints are made from it light enough for text on them.
    pub fn themed(dark: bool, accent: Option<[u8; 3]>, plain_bar: bool) -> Tokens {
        let mut t = Tokens::for_dark(dark);
        if let Some(base) = accent {
            let panes = [rgb(t.surface), rgb(t.chrome), rgb(t.bg)];
            let towards = if dark { [255, 255, 255] } else { [0, 0, 0] };
            let mut a = base;
            for _ in 0..40 {
                if panes.iter().all(|p| contrast(a, *p) >= 4.6) {
                    break;
                }
                a = mix(a, towards, 0.06);
            }
            t.accent = color(a);
            // White on it, or a dark shade of it where white doesn't read (a light accent).
            let dark_text = mix(a, [0, 0, 0], 0.86);
            t.on_accent = if contrast([255, 255, 255], a) >= 4.5 { Color32::WHITE } else { color(dark_text) };
            if contrast(rgb(t.on_accent), a) < 4.5 {
                t.on_accent = if contrast([0, 0, 0], a) > contrast([255, 255, 255], a) { Color32::BLACK } else { Color32::WHITE };
            }
            // The tints: as much of the accent as still leaves text, dim text and the accent
            // itself reading on them.
            let surface = rgb(t.surface);
            let tint = |start: f32, step: f32| {
                let mut share = start;
                loop {
                    let c = mix(surface, a, share);
                    let ok = contrast(rgb(t.text), c) >= 7.2 && contrast(rgb(t.dim), c) >= 4.6 && contrast(a, c) >= 3.1;
                    if ok || share <= 0.02 {
                        return color(c);
                    }
                    share -= step;
                }
            };
            t.accent_soft = tint(if dark { 0.24 } else { 0.18 }, 0.02);
            t.hover_row = tint(if dark { 0.10 } else { 0.08 }, 0.01);
            // The bar: the accent made deep enough for white.
            let mut b = base;
            for _ in 0..40 {
                if contrast([255, 255, 255], b) >= 5.0 {
                    break;
                }
                b = mix(b, [0, 0, 0], 0.06);
            }
            if dark {
                b = mix(b, [0, 0, 0], 0.2);
            }
            t.bar = color(b);
        }
        if plain_bar {
            t.bar = t.chrome;
            t.on_bar = t.text;
        }
        t
    }
}

/// `a` moved `share` (0…1) of the way to `b`.
fn mix(a: [u8; 3], b: [u8; 3], share: f32) -> [u8; 3] {
    let s = share.clamp(0.0, 1.0);
    let m = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * s).round().clamp(0.0, 255.0) as u8;
    [m(a[0], b[0]), m(a[1], b[1]), m(a[2], b[2])]
}

fn color(c: [u8; 3]) -> Color32 {
    Color32::from_rgb(c[0], c[1], c[2])
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
    // Windows, menus and lists that open over the window: a firm edge and a soft shadow, so
    // they stand off what's under them (issue #8).
    v.window_stroke = Stroke::new(1.0, if t.dark { Color32::from_rgb(0x5A, 0x60, 0x69) } else { Color32::from_rgb(0xBF, 0xC5, 0xCD) });
    v.popup_shadow = egui::epaint::Shadow { offset: [0, 4], blur: 14, spread: 0, color: Color32::from_black_alpha(if t.dark { 110 } else { 38 }) };
    v.window_shadow = egui::epaint::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(if t.dark { 120 } else { 45 }) };
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
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, if t.dark { Color32::from_rgb(0x55, 0x5A, 0x63) } else { Color32::from_rgb(0xC6, 0xCC, 0xD3) });
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
/// WCAG relative luminance of an sRGB colour.
fn luminance(c: [u8; 3]) -> f32 {
    let ch = |v: u8| {
        let v = f32::from(v) / 255.0;
        if v <= 0.039_28 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * ch(c[0]) + 0.7152 * ch(c[1]) + 0.0722 * ch(c[2])
}

fn contrast(a: [u8; 3], b: [u8; 3]) -> f32 {
    let (x, y) = (luminance(a), luminance(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

fn rgb(c: Color32) -> [u8; 3] {
    [c.r(), c.g(), c.b()]
}

/// How saturated a colour is (0 grey … 1 pure).
fn saturation(c: [u8; 3]) -> f32 {
    let (hi, lo) = (c.iter().max().copied().unwrap_or(0), c.iter().min().copied().unwrap_or(0));
    if hi == 0 { 0.0 } else { f32::from(hi - lo) / f32::from(hi) }
}

/// The colours to draw a message's text with: the sender's text colour and highlight when they
/// read well on this theme, else adjusted. Grey or white boxes behind text are dropped in dark
/// mode (the page is dark), real highlights kept with dark or light text as fits.
pub fn mail_colors(fg: Option<[u8; 3]>, bg: Option<[u8; 3]>, t: &Tokens, default: Color32) -> (Color32, Option<Color32>) {
    mail_colors_on(fg, bg, rgb(t.surface), t, default)
}

/// The page colour messages are drawn on.
pub fn surface_rgb(t: &Tokens) -> [u8; 3] {
    rgb(t.surface)
}

/// A box's colour (a highlight, a table cell's background) on `page`, when it's kept: one too
/// close to the page to see goes (a report's light grey stripes, #f0f0f0 on white, stay), and grey
/// or white ones go in dark mode (the page is dark).
pub fn box_color(bg: Option<[u8; 3]>, page: [u8; 3], t: &Tokens) -> Option<[u8; 3]> {
    bg.filter(|b| contrast(*b, page) > 1.06 && !(t.dark && luminance(*b) > 0.35 && saturation(*b) < 0.25))
}

/// [`mail_colors`] for text on `page` (a coloured table cell's background).
pub fn mail_colors_on(fg: Option<[u8; 3]>, bg: Option<[u8; 3]>, page: [u8; 3], t: &Tokens, default: Color32) -> (Color32, Option<Color32>) {
    let bg = box_color(bg, page, t);
    let under = bg.unwrap_or(page);
    let fg = match fg {
        Some(c) if contrast(c, under) >= 3.0 => Some(c),
        // Too close to what's under it: a lighter or darker version of the same hue, if that reads.
        Some(c) => {
            let towards = if luminance(under) < 0.3 { 255.0 } else { 0.0 };
            let mix = |v: u8| (f32::from(v) * 0.45 + towards * 0.55) as u8;
            let m = [mix(c[0]), mix(c[1]), mix(c[2])];
            (contrast(m, under) >= 3.0).then_some(m)
        }
        None => None,
    };
    let text = match (fg, bg) {
        (Some(c), _) => Color32::from_rgb(c[0], c[1], c[2]),
        // A highlight without its own text colour: black or white, whichever reads.
        (None, Some(b)) => {
            if contrast([0, 0, 0], b) >= contrast([255, 255, 255], b) {
                Color32::BLACK
            } else {
                Color32::WHITE
            }
        }
        (None, None) => default,
    };
    (text, bg.map(|b| Color32::from_rgb(b[0], b[1], b[2])))
}

pub fn install_fonts(ctx: &egui::Context, regular: Option<Vec<u8>>, bold: Option<Vec<u8>>) {
    install_fonts_with(ctx, regular, bold, Vec::new(), None);
}

/// [`install_fonts`] with fallbacks, used after the others: for Chinese, Japanese and Korean
/// (font files and their index in a collection, first preferred), and for Arabic and Hebrew
/// when the main font hasn't them.
pub fn install_fonts_with(ctx: &egui::Context, regular: Option<Vec<u8>>, bold: Option<Vec<u8>>, cjk: Vec<(Vec<u8>, u32)>, rtl: Option<Vec<u8>>) {
    crate::bidi::set_coverage(regular.as_deref());
    let mut fonts = FontDefinitions::default();
    if let Some(bytes) = rtl {
        fonts.font_data.insert("rtl".into(), Arc::new(FontData::from_owned(bytes)));
        for family in [FontFamily::Proportional, FontFamily::Monospace] {
            if let Some(f) = fonts.families.get_mut(&family) {
                f.push("rtl".into());
            }
        }
    }
    for (i, (bytes, index)) in cjk.into_iter().enumerate() {
        let mut data = FontData::from_owned(bytes);
        data.index = index;
        let name = format!("cjk-{i}");
        fonts.font_data.insert(name.clone(), Arc::new(data));
        for family in [FontFamily::Proportional, FontFamily::Monospace] {
            if let Some(f) = fonts.families.get_mut(&family) {
                f.push(name.clone());
            }
        }
    }
    add_craft_fallbacks(&mut fonts);
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

/// Last of all, the fonts built in from craft-fonts (none unless built with them).
pub(crate) fn add_craft_fallbacks(fonts: &mut FontDefinitions) {
    for (i, f) in crate::craft_fonts::fallbacks().enumerate() {
        let name = format!("craft-{i}");
        fonts.font_data.insert(name.clone(), Arc::new(FontData::from_static(f.bytes)));
        for family in [FontFamily::Proportional, FontFamily::Monospace] {
            if let Some(list) = fonts.families.get_mut(&family) {
                list.push(name.clone());
            }
        }
    }
}

pub fn bold(size: f32) -> egui::FontId {
    egui::FontId::new(size, FontFamily::Name("bold".into()))
}

pub fn regular(size: f32) -> egui::FontId {
    egui::FontId::proportional(size)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every theme a user can pick (issue #8): light and dark, MailCraft's teal, each named
    /// accent and colours chosen freely (the hardest: white, black, yellow, pure green, grey),
    /// with the top bar coloured and plain.
    fn every_theme() -> Vec<(String, Tokens)> {
        let mut accents: Vec<(String, Option<[u8; 3]>)> = vec![("teal".into(), None)];
        accents.extend(mailcraft_engine::ACCENTS.iter().map(|(n, _)| (n.to_string(), mailcraft_engine::accent_rgb(n))));
        for hex in ["#ffffff", "#000000", "#ffff00", "#00ff00", "#808080", "#ff00ff", "#0000ff", "#7fffd4"] {
            accents.push((hex.into(), mailcraft_engine::accent_rgb(hex)));
        }
        let mut out = Vec::new();
        for dark in [false, true] {
            for (name, a) in &accents {
                for plain in [false, true] {
                    let which = format!("{} {name}{}", if dark { "dark" } else { "light" }, if plain { " plain bar" } else { "" });
                    out.push((which, Tokens::themed(dark, *a, plain)));
                }
            }
        }
        out
    }

    /// MailCraft's own teal is the theme it always had, and its dark top bar is now deep enough
    /// for white on it.
    #[test]
    fn the_default_accent_is_the_theme_as_it_was() {
        for dark in [false, true] {
            let (t, was) = (Tokens::themed(dark, None, false), Tokens::for_dark(dark));
            assert_eq!(t, was);
            assert!(contrast(rgb(t.on_bar), rgb(t.bar)) >= 4.5, "{dark}: {}", contrast(rgb(t.on_bar), rgb(t.bar)));
        }
    }

    /// Issue #8: the panes, the chrome around them and the lines between them told apart, in both
    /// themes, and text that reads on every pane (WCAG: 4.5 for text, 3 for the faintest notes).
    #[test]
    fn panes_borders_and_text_stand_apart() {
        for (which, t) in every_theme() {
            let c = |a: Color32, b: Color32| contrast(rgb(a), rgb(b));
            assert!(c(t.border, t.surface) >= 1.4, "{which}: border on a pane {}", c(t.border, t.surface));
            assert!(c(t.border, t.chrome) >= 1.3, "{which}: border on the chrome {}", c(t.border, t.chrome));
            if t.dark {
                assert!(c(t.surface, t.chrome) >= 1.15, "dark: a pane against the folder pane {}", c(t.surface, t.chrome));
            }
            for under in [t.surface, t.chrome, t.bg] {
                assert!(c(t.text, under) >= 7.0, "{which}: text {}", c(t.text, under));
                assert!(c(t.dim, under) >= 4.5, "{which}: dim text {}", c(t.dim, under));
                assert!(c(t.faint, under) >= 3.0, "{which}: faint text {}", c(t.faint, under));
            }
        }
    }

    /// The app colour and the status colours where they're drawn (gridcraft's contrast test
    /// taught this: its dark danger red and a selected heading fell short). Text on the New
    /// mail button and in links, unread times and selected rows reads (4.5); icons, flags and
    /// marks stand out (3).
    #[test]
    fn accent_and_status_colours_stand_out() {
        for (which, t) in every_theme() {
            let c = |a: Color32, b: Color32| contrast(rgb(a), rgb(b));
            let mut short = Vec::new();
            let mut need = |what: &str, a: Color32, b: Color32, min: f32| {
                if c(a, b) < min {
                    short.push(format!("{what} {:.2} < {min}", c(a, b)));
                }
            };
            need("on_accent on accent", t.on_accent, t.accent, 4.5);
            need("on_bar on bar", t.on_bar, t.bar, 4.5);
            for (name, under) in [("surface", t.surface), ("chrome", t.chrome), ("bg", t.bg)] {
                // Text in the app colour is drawn on the panes, not on the window behind them.
                if name != "bg" {
                    need(&format!("accent text on {name}"), t.accent, under, 4.5);
                }
                for (s, col) in [("danger", t.danger), ("flag", t.flag), ("ok", t.ok), ("warn", t.warn)] {
                    need(&format!("{s} on {name}"), col, under, 3.0);
                }
            }
            for (name, under) in [("accent_soft", t.accent_soft), ("hover", t.hover), ("hover_row", t.hover_row)] {
                need(&format!("text on {name}"), t.text, under, 7.0);
                need(&format!("dim on {name}"), t.dim, under, 4.5);
                need(&format!("accent on {name}"), t.accent, under, 3.0);
            }
            assert!(short.is_empty(), "{which}: {}", short.join("; "));
        }
    }
}
