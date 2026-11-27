//! File › Options: the app's settings, a card per subject.

use super::*;

/// MailCraft's own colour, the accent when none is chosen ([`theme::APP_COLOR`]).
const APP_RGB: [u8; 3] = [theme::APP_COLOR.r(), theme::APP_COLOR.g(), theme::APP_COLOR.b()];

mod agents;
mod calendar;
mod general;
mod inbox;
mod mail;
mod safety;
pub(super) use agents::log as agents_log;

/// The parts of Options, each a page of its own under it in the rail (one page of thirteen cards
/// ran seven screens or more, with no way to a part but scrolling).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Section {
    #[default]
    General,
    Mail,
    Calendar,
    DataFiles,
    Safety,
    Agents,
}

impl Section {
    pub const ALL: [Section; 6] = [Section::General, Section::Mail, Section::Calendar, Section::DataFiles, Section::Safety, Section::Agents];

    pub fn label(self) -> &'static str {
        match self {
            Section::General => "General",
            Section::Mail => "Mail",
            Section::Calendar => "Calendar",
            Section::DataFiles => "Data files",
            Section::Safety => "Privacy and safety",
            Section::Agents => "AI agents",
        }
    }
}

/// A card of Options.
type Card = fn(&Session, &UiState, &mut Ui, &mut Vec<Action>);

/// Each section's cards, in order.
fn cards(section: Section) -> &'static [Card] {
    match section {
        Section::General => &[|s, st, ui, acts| general::general(s, st, ui, acts)],
        Section::Mail => &[
            |s, st, ui, acts| mail::reading(s, &st.tokens, ui, acts),
            |s, st, ui, acts| inbox::inbox(s, &st.tokens, ui, acts),
            |s, st, ui, acts| mail::checking(s, &st.tokens, ui, acts),
            |s, st, ui, acts| mail::kept(s, &st.tokens, ui, acts),
        ],
        Section::Calendar => {
            &[|s, st, ui, acts| calendar::calendar(s, &st.tokens, ui, acts), |s, st, ui, acts| super::meetings::meetings(s, st, ui, acts)]
        }
        Section::DataFiles => &[|s, st, ui, acts| general::data_files(s, st, ui, acts)],
        Section::Safety => &[
            |s, st, ui, acts| safety::safety(s, &st.tokens, ui, acts),
            |s, st, ui, acts| safety::senders(s, &st.tokens, ui, acts),
            |s, st, ui, acts| safety::privacy(s, &st.tokens, ui, acts),
        ],
        Section::Agents => &[|s, st, ui, acts| agents::agents(s, &st.tokens, ui, acts)],
    }
}

/// What's being looked for among the settings (lower case), while a search is typed.
fn finding(ui: &Ui) -> Option<String> {
    ui.data(|d| d.get_temp::<String>(find_id())).filter(|q| !q.is_empty())
}

fn find_id() -> egui::Id {
    egui::Id::unique("options-find")
}

/// How many settings a search matched, as a card is drawn.
fn found_id() -> egui::Id {
    egui::Id::unique("options-found")
}

pub(super) fn options(s: &Session, st: &UiState, ui: &mut Ui, section: Section, find: &mut String, acts: &mut Vec<Action>) {
    let t = st.tokens;
    heading(ui, &t, if find.trim().is_empty() { section.label() } else { "Options" });
    ui.set_max_width(ui.available_width().min(760.0));
    // A setting found by its name or what it does, in any section.
    ui.add(crate::widgets::line_edit(find).hint_text(tl!("Find a setting")).desired_width(320.0));
    ui.add_space(12.0);
    let query = find.trim().to_lowercase();
    ui.data_mut(|d| d.insert_temp(find_id(), query.clone()));
    if query.is_empty() {
        for card in cards(section) {
            card(s, st, ui, acts);
        }
        return;
    }
    // Searching: each card that has a setting found, with only those settings (counted in a pass
    // that draws nothing first).
    let mut any = false;
    for card in Section::ALL.iter().flat_map(|s| cards(*s)) {
        ui.data_mut(|d| d.insert_temp(found_id(), 0usize));
        ui.scope_builder(egui::UiBuilder::new().invisible().sizing_pass(), |ui| card(s, st, ui, &mut Vec::new()));
        if ui.data(|d| d.get_temp::<usize>(found_id())).unwrap_or(0) > 0 {
            any = true;
            card(s, st, ui, acts);
        }
    }
    if !any {
        ui.label(RichText::new(tl!("No setting matches that.")).color(t.dim));
    }
    ui.data_mut(|d| d.remove::<String>(find_id()));
}

/// One setting of a card, one per row, by its name and the line about it (in the rows Info and
/// Open & Export draw too).
fn setting(ui: &mut Ui, t: &Tokens, first: bool, label: &str, about: Option<&str>, control: impl FnOnce(&mut Ui)) {
    // While a search is typed: only the settings it finds, by their name or what they do.
    if let Some(q) = finding(ui) {
        let found = label.to_lowercase().contains(&q) || about.is_some_and(|a| a.to_lowercase().contains(&q));
        if !found {
            return;
        }
        ui.data_mut(|d| *d.get_temp_mut_or_default::<usize>(found_id()) += 1);
    }
    setting_row(ui, t, first, name(t, label), about.map(|a| note(t, a)), control);
}

/// [`setting`], for one the command `cmd` changes: its controls greyed out, and said, while the
/// organisation has it locked (issue #245; the command would be refused anyway).
fn setting_by(s: &Session, cmd: &str, ui: &mut Ui, t: &Tokens, first: bool, label: &str, about: Option<&str>, control: impl FnOnce(&mut Ui)) {
    setting_held(s.command_locked(cmd).is_some(), ui, t, first, label, about, control);
}

/// [`setting`], its controls greyed out and said to be the organisation's when `locked`.
fn setting_held(locked: bool, ui: &mut Ui, t: &Tokens, first: bool, label: &str, about: Option<&str>, control: impl FnOnce(&mut Ui)) {
    if !locked {
        setting(ui, t, first, label, about, control);
        return;
    }
    let locked = tl!("Set by your organisation");
    let said = about.map_or_else(|| locked.to_string(), |a| format!("{a} · {locked}"));
    setting(ui, t, first, label, Some(&said), |ui| {
        ui.add_enabled_ui(false, control);
    });
}

/// A setting that is a switch: its name, also the switch's to a screen reader; the line about
/// it; whether it's on; the command told whether it's turned on.
struct Switch<'a> {
    label: &'a str,
    about: Option<&'a str>,
    on: bool,
    cmd: &'static str,
}

/// A setting that is a switch, its name given once.
fn switch_row(s: &Session, ui: &mut Ui, t: &Tokens, first: bool, w: Switch<'_>, acts: &mut Vec<Action>) {
    let Switch { label, about, on, cmd } = w;
    setting_by(s, cmd, ui, t, first, label, about, |ui| {
        let mut v = on;
        if widgets::switch(ui, t, &mut v, label).changed() {
            acts.push(Action::Run(cmd, json!({"on": v})));
        }
    });
}

/// The accent colours as swatches (MailCraft's teal first), then any colour with a picker.
fn accent_choice(ui: &mut Ui, t: &Tokens, current: &str, acts: &mut Vec<Action>) {
    let names = [
        ("teal", tl!("Teal")),
        ("blue", tl!("Blue")),
        ("indigo", tl!("Indigo")),
        ("violet", tl!("Violet")),
        ("rose", tl!("Rose")),
        ("red", tl!("Red")),
        ("orange", tl!("Orange")),
        ("green", tl!("Green")),
    ];
    // The row is laid out from its end: the swatches go in the other way round, so teal reads
    // first (and the picker last).
    let from_end = ui.layout().prefer_right_to_left() != crate::i18n::current().rtl();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let picker = |ui: &mut Ui, acts: &mut Vec<Action>| {
            // Any colour: what's chosen now, or the accent in use, to start from.
            // A round swatch like the others, ringed only when it's the one chosen; a "+" on
            // it while it isn't (a block in the accent's colour read as a second chosen swatch).
            let rgb = mailcraft_engine::accent_rgb(current).unwrap_or(APP_RGB);
            let mine = current.starts_with('#');
            let (rect, r) = ui.allocate_exact_size(vec2(24.0, 24.0), Sense::click());
            widgets::named(ui, &r, tl!("Any colour"), true, Some(mine));
            if mine {
                ui.painter().circle_stroke(rect.center(), 11.5, egui::Stroke::new(2.0, t.text));
                ui.painter().circle_filled(rect.center(), 8.5, egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2]));
            } else {
                ui.painter().circle_stroke(rect.center(), 8.5, egui::Stroke::new(1.0, if r.hovered() { t.text } else { t.dim }));
                let c = rect.center();
                let ink = egui::Stroke::new(1.5, t.dim);
                ui.painter().line_segment([c - vec2(4.0, 0.0), c + vec2(4.0, 0.0)], ink);
                ui.painter().line_segment([c - vec2(0.0, 4.0), c + vec2(0.0, 4.0)], ink);
            }
            let r = r.on_hover_text(tl!("Any colour"));
            egui::Popup::menu(&r).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
                let mut c = egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2]);
                if egui::widgets::color_picker::color_picker_color32(ui, &mut c, egui::widgets::color_picker::Alpha::Opaque) {
                    acts.push(Action::Run("view.accent", json!({"color": format!("#{:02x}{:02x}{:02x}", c.r(), c.g(), c.b())})));
                }
            });
        };
        if from_end {
            picker(ui, acts);
            ui.add_space(8.0);
        }
        let ordered: Vec<(&str, &str)> = if from_end { names.iter().rev().copied().collect() } else { names.to_vec() };
        for (code, label) in ordered {
            let rgb = mailcraft_engine::accent_rgb(code).unwrap_or(APP_RGB);
            let on = current == code || (code == "teal" && current.is_empty());
            let (rect, resp) = ui.allocate_exact_size(vec2(24.0, 24.0), Sense::click());
            widgets::named(ui, &resp, label, true, Some(on));
            let fill = egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2]);
            if on {
                ui.painter().circle_stroke(rect.center(), 11.5, egui::Stroke::new(2.0, t.text));
            } else if resp.hovered() {
                ui.painter().circle_stroke(rect.center(), 11.5, egui::Stroke::new(1.0, t.border));
            }
            ui.painter().circle_filled(rect.center(), 8.5, fill);
            if resp.on_hover_text(label).clicked() && !on {
                acts.push(Action::Run("view.accent", json!({"color": code})));
            }
        }
        if !from_end {
            ui.add_space(8.0);
            picker(ui, acts);
        }
    });
}
