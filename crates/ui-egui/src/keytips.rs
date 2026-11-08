//! Key tips on the ribbon, as Outlook's: Alt (or F10) shows a letter on each tab and on the
//! search box; a tab's letters open it and show its buttons' letters, and a button's run it.
//! Escape goes back a letter, then hides them. The letters are in [`table`]; the matcher is
//! GridCraft's, the ribbon's buttons draw their letters and are pressed by them as they're drawn
//! ([`apply`]).

use egui::{Context, Event, Key, Modifiers, os::OperatingSystem};

use crate::look::theme::Tokens;

pub mod table;
#[cfg(test)]
mod tests;

use table::{Entry, Top};

/// What a sequence of letters asks of the app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Act {
    /// Open the File page.
    File,
    /// Show this tab.
    Tab(crate::RibbonTab),
    /// Put the keyboard in the search box.
    Search,
    /// Press the button this whole sequence names (as it's drawn this frame).
    Press(String),
}

/// The letters typed so far, if key tips show.
#[derive(Default)]
pub struct KeyTips {
    prefix: Option<String>,
    alt_armed: bool,
    // Left Alt is down: Alt+H (held, as Outlook allows) types key tip letters.
    alt_held: bool,
    // Keep consuming a held letter after its action hides the key tips.
    held: Vec<Key>,
}

/// Set by the Alt codes (`app/alt_code.rs`) while Alt is held: digits typed then are a
/// character's code, and letting go of Alt types it rather than showing key tips.
pub(crate) fn alt_code_typed() -> egui::Id {
    egui::Id::unique("keytips-alt-code")
}

impl KeyTips {
    /// The letters typed so far (empty: Alt was pressed), if key tips show.
    pub fn prefix(&self) -> Option<&str> {
        self.prefix.as_deref()
    }

    pub fn clear(&mut self) {
        self.prefix = None;
        self.alt_armed = false;
        self.alt_held = false;
        self.held.clear();
    }

    fn toggle(&mut self) {
        self.prefix = if self.prefix.is_some() { None } else { Some(String::new()) };
        self.alt_armed = false;
    }

    /// Read this frame's keys before anything else does: those the key tips take are removed.
    /// Not `enabled` (a dialog or the File page is open), they hide.
    pub fn process(&mut self, ctx: &Context, enabled: bool) -> Vec<Act> {
        if !enabled || !ctx.input(|i| i.focused) {
            self.clear();
            return Vec::new();
        }
        let mac = ctx.os() == OperatingSystem::Mac;
        let events = ctx.input_mut(|i| std::mem::take(&mut i.events));
        let pairs = text_pairs(&events);
        let mut consumed = std::collections::HashSet::new();
        let mut acts = Vec::new();
        for (index, event) in events.iter().enumerate() {
            let mut handled = false;
            match event {
                Event::PointerButton { pressed: true, .. } | Event::MouseWheel { .. } | Event::WindowFocused(false) | Event::Ime(_) => self.clear(),
                Event::Copy | Event::Cut | Event::Paste(_) => self.clear(),
                Event::ModifiersChanged(m) => {
                    if !m.alt {
                        self.alt_held = false;
                    }
                    if command_modifier(*m) || (mac && m.alt) {
                        self.clear();
                    } else if m.shift {
                        self.alt_armed = false;
                    }
                }
                Event::Key { key: Key::AltLeft, pressed, repeat, modifiers, .. } if !mac && !command_modifier(*modifiers) && !modifiers.shift => {
                    self.alt_held = *pressed;
                    if *pressed {
                        if !repeat {
                            self.alt_armed = true;
                            ctx.data_mut(|d| d.remove::<bool>(alt_code_typed()));
                        }
                        handled = true;
                    } else if self.alt_armed && !ctx.data_mut(|d| d.remove_temp::<bool>(alt_code_typed())).unwrap_or(false) {
                        self.toggle();
                        handled = true;
                    } else {
                        self.alt_armed = false;
                    }
                }
                Event::Key { key, pressed, repeat, modifiers, .. } => {
                    self.alt_armed = false;
                    // A letter typed with left Alt held goes on with a sequence (Alt+H, R, P), as
                    // in Outlook; any other Alt combination hides the key tips.
                    let alt_letter = self.alt_held
                        && modifiers.alt
                        && !modifiers.shift
                        && !command_modifier(*modifiers)
                        && letter(*key).is_some_and(|c| {
                            let next = format!("{}{c}", self.prefix.as_deref().unwrap_or_default());
                            table::lookup(&next).is_some() || table::has_deeper(&next)
                        });
                    if alt_letter && *pressed && self.prefix.is_none() {
                        self.prefix = Some(String::new());
                    }
                    // The letter's release still carries Alt.
                    let alt_release = !*pressed && self.held.contains(key);
                    if command_modifier(*modifiers)
                        || (modifiers.alt && !alt_letter && !alt_release)
                        || matches!(key, Key::AltLeft | Key::AltRight | Key::ControlLeft | Key::ControlRight | Key::SuperLeft | Key::SuperRight)
                    {
                        self.clear();
                    } else if !pressed {
                        handled = self.held.contains(key);
                        self.held.retain(|held| held != key);
                    } else if *repeat && (self.prefix.is_some() || self.held.contains(key)) {
                        handled = true;
                    } else if *key == Key::F10 && modifiers.is_none() {
                        self.toggle();
                        handled = true;
                    } else if self.prefix.is_some() && !matches!(key, Key::ShiftLeft | Key::ShiftRight) {
                        handled = true;
                        if *key == Key::Escape {
                            // Escape goes back a letter; with none typed, the key tips hide.
                            if let Some(prefix) = self.prefix.as_mut() {
                                if prefix.is_empty() {
                                    self.prefix = None;
                                } else {
                                    prefix.pop();
                                }
                            }
                        } else if let Some(letter) = letter(*key) {
                            self.typed(letter, &mut acts);
                        }
                    }
                    if handled && *pressed && !self.held.contains(key) {
                        self.held.push(*key);
                    }
                }
                Event::Text(_) => {
                    self.alt_armed = false;
                    // Text paired with a key is decided with the key, which may come later on the web.
                    handled = self.prefix.is_some() && !pairs.iter().any(|(_, text)| *text == index);
                }
                _ => {}
            }
            if handled {
                consumed.insert(index);
                if let Some((_, text)) = pairs.iter().find(|(key, _)| *key == index) {
                    consumed.insert(*text);
                }
            }
        }
        ctx.input_mut(|i| {
            i.events = events.into_iter().enumerate().filter_map(|(index, event)| (!consumed.contains(&index)).then_some(event)).collect()
        });
        acts
    }

    /// A letter typed with key tips showing.
    fn typed(&mut self, letter: char, acts: &mut Vec<Act>) {
        let next = format!("{}{letter}", self.prefix.as_deref().unwrap_or_default());
        match table::lookup(&next) {
            // A tab stays open with its buttons' letters showing.
            Some(Entry::Top(Top::Tab(tab))) => {
                self.prefix = Some(next);
                acts.push(Act::Tab(tab));
            }
            Some(Entry::Top(Top::File)) => {
                self.prefix = None;
                acts.push(Act::File);
            }
            Some(Entry::Top(Top::Search)) => {
                self.prefix = None;
                acts.push(Act::Search);
            }
            Some(Entry::Button(sequence)) => {
                self.prefix = None;
                acts.push(Act::Press(sequence));
            }
            // Part of a longer sequence (Send / Receive's J S): more letters are waited for.
            None if table::has_deeper(&next) => self.prefix = Some(next),
            // A letter that reaches nothing is taken, and nothing changes (as in Outlook).
            None => {}
        }
    }
}

/// A key's letter or digit, as the key tips name them.
fn letter(key: Key) -> Option<char> {
    let name = key.name();
    let mut chars = name.chars();
    let c = chars.next()?;
    (chars.next().is_none() && c.is_ascii_alphanumeric()).then(|| c.to_ascii_uppercase())
}

fn command_modifier(m: Modifiers) -> bool {
    m.ctrl || m.command || m.mac_cmd
}

/// The desktop sends a key and then its text; browsers can send the text first.
fn text_pairs(events: &[Event]) -> Vec<(usize, usize)> {
    let mut pairs = Vec::new();
    for (text_index, event) in events.iter().enumerate() {
        let Event::Text(_) = event else { continue };
        let before = events.iter().enumerate().take(text_index).rev().find(|(_, e)| !matches!(e, Event::ModifiersChanged(_)));
        let after = events.iter().enumerate().skip(text_index + 1).find(|(_, e)| !matches!(e, Event::ModifiersChanged(_)));
        for (key_index, event) in before.into_iter().chain(after) {
            if let Event::Key { key, pressed: true, modifiers, .. } = event
                && (key.name().len() == 1 || modifiers.alt || command_modifier(*modifiers))
                && !pairs.iter().any(|(paired, _)| *paired == key_index)
            {
                pairs.push((key_index, text_index));
                break;
            }
        }
    }
    pairs
}

/// What the ribbon's buttons need this frame: the letters typed so far, the tab shown, the
/// sequence of the button to press, if one was finished, and whether the ribbon is being drawn
/// (only its buttons take key tips, not a message window's of the same name).
#[derive(Clone, Default)]
pub struct Frame {
    pub prefix: Option<String>,
    pub tab: Option<crate::RibbonTab>,
    pub press: Option<String>,
    pub ribbon: bool,
}

fn frame_id() -> egui::Id {
    egui::Id::unique("keytips-frame")
}

/// Hand this frame's key tips to the ribbon's buttons.
pub fn set_frame(ctx: &Context, frame: Frame) {
    ctx.data_mut(|d| d.insert_temp(frame_id(), frame));
}

/// The ribbon drawn by `draw`: its buttons take the key tips, and none drawn after it does (found
/// in review: Alt, H, D pressed a message window's own Delete, the ribbon's being off).
pub fn on_ribbon<R>(ui: &mut egui::Ui, draw: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let set = |ctx: &Context, on: bool| {
        ctx.data_mut(|d| {
            let f = d.get_temp_mut_or_default::<Frame>(frame_id());
            f.ribbon = on;
            if !on {
                f.press = None;
            }
        })
    };
    set(ui.ctx(), true);
    let r = draw(ui);
    set(ui.ctx(), false);
    r
}

/// A ribbon button labelled `label` (in English, as [`table::BUTTONS`] names it): its letters
/// drawn under it while its tab's show, and pressed when they were typed (and it can be).
pub fn apply(ui: &egui::Ui, label: &str, resp: &mut egui::Response, enabled: bool, t: &Tokens) {
    let Some(frame) = ui.ctx().data(|d| d.get_temp::<Frame>(frame_id())).filter(|f| f.ribbon) else { return };
    let Some(tab) = frame.tab else { return };
    let Some(sequence) = table::button_sequence(tab, label) else { return };
    // Pressed once: a button drawn in its place this frame isn't pressed too.
    if frame.press.as_deref() == Some(sequence.as_str()) && enabled {
        resp.flags.insert(egui::response::Flags::FAKE_PRIMARY_CLICKED);
        ui.ctx().data_mut(|d| d.get_temp_mut_or_default::<Frame>(frame_id()).press = None);
    }
    // Its letters show once its tab's are typed, while they can still be typed.
    let Some(prefix) = frame.prefix.as_deref() else { return };
    let tab_letters = table::tab_letters(tab).map_or(0, str::len);
    if prefix.len() >= tab_letters && sequence.len() > prefix.len() && sequence.starts_with(prefix) {
        badge(ui.ctx(), resp.rect, sequence.get(tab_letters..).unwrap_or_default(), enabled, t);
    }
}

/// A key tip's letters, in a small dark box on the middle of `rect`'s bottom edge, mostly below
/// it (as Outlook draws them), over everything else; faded on a button that can't be used now.
pub fn badge(ctx: &Context, rect: egui::Rect, letters: &str, enabled: bool, t: &Tokens) {
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("keytips")));
    let font = egui::FontId::proportional(11.0);
    let galley = painter.layout_no_wrap(letters.to_string(), font, t.surface);
    let size = egui::vec2(galley.size().x + 8.0, galley.size().y + 2.0);
    let at = egui::Rect::from_center_size(egui::pos2(rect.center().x, rect.bottom() + size.y / 4.0), size);
    let fade = if enabled { 1.0 } else { 0.45 };
    painter.rect_filled(at, 2.0, t.text.gamma_multiply(fade));
    painter.galley_with_override_text_color(at.min + egui::vec2(4.0, 1.0), galley, t.surface);
}

impl crate::MailApp {
    /// This frame's key tips, read before the shortcuts: a tab shown, the search box given the
    /// keyboard, a button to press as the ribbon is drawn. Whether the File page is to open.
    pub(crate) fn keytips(&mut self, ctx: &Context) -> bool {
        let enabled = self.ui.dialog.is_none() && self.ui.backstage.is_none();
        let mut file = false;
        let mut press = None;
        for act in self.ui.keytips.process(ctx, enabled) {
            match act {
                Act::File => file = true,
                Act::Tab(tab) => self.ui.tab = tab,
                Act::Search => self.ui.focus_search = true,
                Act::Press(sequence) => press = Some(sequence),
            }
        }
        if self.ui.keytips.prefix().is_some() {
            ctx.request_repaint();
        }
        set_frame(ctx, Frame { prefix: self.ui.keytips.prefix().map(str::to_string), tab: Some(self.ui.tab), press, ribbon: false });
        file
    }
}
