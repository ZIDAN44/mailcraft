//! The window before MailCraft is in it: the data files opening on a thread of the app's, and a
//! card saying so when that takes more than a moment (a data file not closed properly is checked
//! as it opens, which can take minutes for a big one). Portable: no threads and no clock (egui's
//! input time).

use egui::{Align, Layout, RichText, Ui, vec2};
use mailcraft_engine::store::{OpenProgress, OpenStage};

use crate::i18n::{self, Lang};
use crate::theme::{self, Tokens};

/// Nothing is drawn for this long, so a start with nothing to check shows no flash of a card.
pub const QUIET_START: f64 = 0.3;
/// How often the card is drawn again while it waits.
const REDRAW: std::time::Duration = std::time::Duration::from_millis(100);

/// What the window draws until MailCraft is in it.
pub struct BootCard {
    /// When the first frame was (egui's input time).
    since: Option<f64>,
    /// The language and the light or dark chosen in the settings, when they were read.
    language: String,
    dark: Option<bool>,
    /// The problems were seen, and Continue pressed.
    seen: bool,
}

impl BootCard {
    /// A card in the `language` setting (`auto`: the system's), light or dark as `dark` says
    /// (`None`: as the system is).
    pub fn new(language: &str, dark: Option<bool>) -> BootCard {
        BootCard { since: None, language: language.to_string(), dark, seen: false }
    }

    /// The language to draw in: the setting's, unless its script has no font before MailCraft
    /// loads one (Chinese, Japanese, Korean, Arabic and Hebrew come with the app's own fonts, or
    /// with fonts it finds later): English then, rather than boxes.
    fn lang(&self) -> Lang {
        let lang = Lang::from_pref(&self.language);
        let script = match lang.code() {
            "ja" => "Jpan",
            "ko" => "Kore",
            "zh-hans" => "Hans",
            "zh-hant" => "Hant",
            "ar" => "Arab",
            "he" => "Hebr",
            _ => return lang,
        };
        if crate::craft_fonts::has_script(script) { lang } else { Lang::EN }
    }

    /// Whether the window can go on to MailCraft, `ready` being whether what MailCraft opens with
    /// has come: the files are open (a data file that couldn't be is said, and Continue pressed).
    /// Asked every frame too, the window minimized or hidden (when [`BootCard::ui`] isn't drawn).
    pub fn may_go(&self, progress: &OpenProgress, ready: bool) -> bool {
        ready && progress.stage() == OpenStage::Done && (self.seen || progress.problems().is_empty())
    }

    /// Draw the card as `progress` says, once opening has taken longer than [`QUIET_START`];
    /// `ready` is whether what MailCraft opens with has come. True when the window can go on to
    /// MailCraft ([`BootCard::may_go`]).
    pub fn ui(&mut self, ui: &mut Ui, progress: &OpenProgress, ready: bool) -> bool {
        let now = ui.input(|i| i.time);
        let since = *self.since.get_or_insert(now);
        if self.may_go(progress, ready) {
            return true;
        }
        let problems = if progress.stage() == OpenStage::Done { progress.problems() } else { Vec::new() };
        ui.ctx().request_repaint_after(REDRAW);
        if now - since < QUIET_START && problems.is_empty() {
            return false;
        }
        let lang = self.lang();
        i18n::set_current(lang);
        if ui.ctx().global_style().right_to_left != lang.rtl() {
            ui.ctx().all_styles_mut(|s| s.right_to_left = lang.rtl());
        }
        let t = Tokens::for_dark(self.dark.unwrap_or_else(|| ui.visuals().dark_mode));
        theme::apply(ui.ctx(), &t);
        let rect = ui.max_rect();
        ui.painter().rect_filled(rect, 0.0, t.bg);
        let width = (rect.width() - 32.0).clamp(120.0, 460.0);
        let card =
            egui::Frame::new().fill(t.surface).stroke(egui::Stroke::new(1.0, t.border)).corner_radius(8.0).inner_margin(egui::Margin::same(20));
        ui.scope_builder(egui::UiBuilder::new().max_rect(rect).layout(Layout::top_down(Align::Center)), |ui| {
            ui.add_space((rect.height() * 0.3).min(240.0));
            card.show(ui, |ui| {
                ui.set_width(width);
                ui.with_layout(Layout::top_down(Align::Min), |ui| {
                    if !problems.is_empty() {
                        self.problems(ui, &t, &problems);
                    } else {
                        Self::opening(ui, &t, progress);
                    }
                });
            });
        });
        false
    }

    /// The data files opening, or one being checked.
    fn opening(ui: &mut Ui, t: &Tokens, progress: &OpenProgress) {
        let checking = progress.stage() == OpenStage::Checking;
        ui.horizontal(|ui| {
            ui.add(egui::Spinner::new().size(18.0).color(t.accent));
            let title = if checking { tl!("Checking your data file") } else { tl!("Opening your mail…") };
            ui.label(RichText::new(title).font(theme::bold(16.0)).color(t.text));
        });
        ui.add_space(8.0);
        if checking {
            let file = progress.file();
            ui.label(
                RichText::new(tf!(
                    "{file} wasn't closed properly last time. MailCraft is checking it; a large file can take a minute or two.",
                    file = file
                ))
                .color(t.dim),
            );
            // How far, from the database's passes over the file and what each has read of it.
            let permille = progress.permille();
            ui.add_space(8.0);
            // Its track set off from the card (a light card and the bar's own track are both white).
            ui.visuals_mut().extreme_bg_color = t.hover;
            ui.add(egui::ProgressBar::new(permille as f32 / 1000.0).desired_width(ui.available_width()).fill(t.accent));
            ui.add_space(4.0);
            ui.label(RichText::new(tf!("{percent}% checked", percent = permille / 10)).color(t.faint));
        }
        let (done, total) = progress.files();
        if total > 1 {
            ui.add_space(6.0);
            ui.label(RichText::new(tf!("{done} of {total} data files", done = done, total = total)).color(t.faint));
        }
    }

    /// Data files that couldn't be opened, and Continue.
    fn problems(&mut self, ui: &mut Ui, t: &Tokens, problems: &[(String, String)]) {
        ui.label(RichText::new(tl!("Some of your mail couldn't be opened")).font(theme::bold(16.0)).color(t.text));
        ui.add_space(8.0);
        // One with no file name is the whole opening (its words are in the log).
        let whole = problems.iter().any(|(file, _)| file.is_empty());
        for (file, why) in problems.iter().filter(|(file, _)| !file.is_empty()) {
            ui.label(RichText::new(format!("{file}: {why}")).color(t.dim));
        }
        ui.add_space(6.0);
        let after = if whole {
            tl!("MailCraft couldn't open its data files, so it runs without its mail cache this time.")
        } else {
            tl!("MailCraft opens without them; each account says what's wrong when you use it.")
        };
        ui.label(RichText::new(after).color(t.faint));
        ui.add_space(12.0);
        ui.allocate_ui_with_layout(vec2(ui.available_width(), 30.0), Layout::right_to_left(Align::Center), |ui| {
            // Pressed before what MailCraft opens with came: it goes on as soon as that has.
            if self.seen {
                ui.add(egui::Spinner::new().size(16.0).color(t.accent));
            } else if ui.add(egui::Button::new(RichText::new(tl!("Continue")).color(t.on_accent)).fill(t.accent).min_size(vec2(96.0, 30.0))).clicked()
            {
                self.seen = true;
            }
        });
    }
}

#[cfg(test)]
pub(crate) mod tests;
