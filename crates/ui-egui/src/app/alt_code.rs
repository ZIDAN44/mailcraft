//! Alt+numpad character codes on Windows (issue #23): hold Alt, type a code on the numeric
//! keypad, let go of Alt, and its character is typed (`0201` → É), in any text. Windows makes
//! the character itself, but the window library drops it (it comes after the keys, with none of
//! its own), so the digits are collected here and the character typed on Alt's release, in the
//! system's code pages ([`mailcraft_engine::mime::alt_code_char`]). After WordCraft's.

/// A code another key ended, while Alt is still held.
const INTERRUPTED: &str = "-";

/// The digits typed so far with Alt held, in a window.
fn typed(viewport: egui::ViewportId) -> egui::Id {
    egui::Id::unique("alt-code").with(viewport)
}

/// The Windows code pages for the user's language (ANSI, OEM), which Alt codes are read in.
pub fn alt_code_pages() -> (u16, u16) {
    mailcraft_engine::mime::alt_code_pages(sys_locale::get_locale().as_deref())
}

/// Run on a window's input before egui reads it: digits typed with Alt alone held are taken out
/// and collected; when Alt is let go, the character their code names is typed as text. egui doesn't
/// tell the keypad's digits from the top row's, so those compose a code too (with Alt alone they
/// do nothing else). Any other key, or the window losing focus, drops a half-typed code.
pub fn alt_codes(ctx: &egui::Context, raw: &mut egui::RawInput, pages: (u16, u16)) {
    use egui::Key;
    let id = typed(raw.viewport_id);
    let mut code: String = ctx.data(|d| d.get_temp(id)).unwrap_or_default();
    let alt_only = |m: &egui::Modifiers| m.alt && !m.ctrl && !m.shift && !m.command && !m.mac_cmd;
    let digit = |k: Key| match k {
        Key::Num0 => Some('0'),
        Key::Num1 => Some('1'),
        Key::Num2 => Some('2'),
        Key::Num3 => Some('3'),
        Key::Num4 => Some('4'),
        Key::Num5 => Some('5'),
        Key::Num6 => Some('6'),
        Key::Num7 => Some('7'),
        Key::Num8 => Some('8'),
        Key::Num9 => Some('9'),
        _ => None,
    };
    let mut out = Vec::with_capacity(raw.events.len());
    for e in raw.events.drain(..) {
        match &e {
            egui::Event::Key { key, pressed, repeat, modifiers, .. } if alt_only(modifiers) && digit(*key).is_some() => {
                // A held digit's repeat adds nothing (as on Windows); codes past seven digits
                // name nothing, so collecting stops.
                if let Some(d) = digit(*key).filter(|_| *pressed && !*repeat && code.len() < 8 && code != INTERRUPTED) {
                    code.push(d);
                }
            }
            // Its digits' text, where a platform sends it with Alt held.
            egui::Event::Text(t) if !code.is_empty() && code != INTERRUPTED && t.chars().all(|c| c.is_ascii_digit()) => {}
            // Alt let go: the code's character, where it was typed.
            egui::Event::ModifiersChanged(m) if !m.alt && !code.is_empty() => {
                if let Some(c) = mailcraft_engine::mime::alt_code_char(&code, pages.0, pages.1) {
                    out.push(egui::Event::Text(c.to_string()));
                }
                code.clear();
                out.push(e);
            }
            // Another key with Alt held ends the code unfinished, and nothing more is typed
            // until Alt is let go (as on Windows); without Alt, there's nothing to end.
            egui::Event::Key { pressed: true, modifiers, .. } => {
                code = if modifiers.alt { INTERRUPTED.to_string() } else { String::new() };
                out.push(e);
            }
            egui::Event::WindowFocused(false) => {
                code.clear();
                out.push(e);
            }
            _ => out.push(e),
        }
    }
    raw.events = out;
    ctx.data_mut(|d| d.insert_temp(id, code));
}
