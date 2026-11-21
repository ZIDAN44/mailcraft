//! Invitations in the Inbox (issue #237): the meetings waiting for an answer there that the
//! calendar hasn't got (mail that was there before the account came here), each with a box to
//! tick; those ticked go into the calendar unanswered. Offered, not done unasked: one deleted
//! from the calendar on purpose looks the same.

use mailcraft_engine::model::MessageKey;

use super::*;

/// How often the list is looked at again while messages are still being read for it (seconds).
const AGAIN_AFTER: f64 = 2.0;

/// One invitation found: its message, what it's for, and whether it's ticked.
#[derive(Clone, Debug)]
pub struct Waiting {
    pub key: MessageKey,
    pub summary: String,
    pub start: i64,
    pub end: i64,
    pub organizer: String,
    pub picked: bool,
}

#[derive(Clone, Debug, Default)]
pub struct WaitingForm {
    pub found: Vec<Waiting>,
    /// Messages still being read for it, and when the list was last made (`None`: not yet).
    pub reading: usize,
    pub looked: Option<f64>,
    /// The messages asked for already, to read them.
    pub asked: std::collections::HashSet<MessageKey>,
}

impl WaitingForm {
    /// The list made again from the session (every account's Inbox) when it's due: at first,
    /// then while messages are still being read. What was ticked stays ticked.
    pub fn refresh(&mut self, s: &mut Session, now: f64) {
        if self.looked.is_some_and(|at| self.reading == 0 || now - at < AGAIN_AFTER) {
            return;
        }
        let accounts: Vec<String> = s.accounts.iter().map(|a| a.config.id.clone()).collect();
        let (mut found, mut reading) = (Vec::new(), 0);
        for a in accounts {
            let (f, r) = s.waiting_invitations(&a, &mut self.asked);
            reading += r;
            found.extend(f.into_iter().map(|w| {
                let picked = self.found.iter().find(|o| o.key == w.key).is_none_or(|o| o.picked);
                Waiting { key: w.key, summary: w.summary, start: w.start, end: w.end, organizer: w.organizer, picked }
            }));
        }
        found.sort_by_key(|w| w.start);
        self.found = found;
        self.reading = reading;
        self.looked = Some(now);
    }
}

pub fn waiting(ui: &mut Ui, f: &mut WaitingForm, s: &Session, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    ui.set_width(520.0);
    form_header(ui, t, Ok(Icon::Calendar), "Invitations in the Inbox");
    ui.label(
        RichText::new(tl!("Meetings waiting for your answer that your calendar doesn't have yet. Those you tick go in unanswered."))
            .color(t.dim)
            .small(),
    );
    ui.add_space(6.0);
    if f.looked.is_none() {
        ui.label(RichText::new(tl!("Looking…")).color(t.dim));
    } else if f.found.is_empty() && f.reading == 0 {
        ui.label(RichText::new(tl!("None: every invitation in the Inbox is in your calendar or answered.")).color(t.text));
    }
    egui::ScrollArea::vertical().max_height(300.0).auto_shrink([false, true]).show(ui, |ui| {
        for w in &mut f.found {
            let when = crate::i18n::date(&dates::full_time(w.start, &s.clock));
            let line =
                if w.organizer.is_empty() { format!("{} · {when}", w.summary) } else { format!("{} · {when} · {}", w.summary, w.organizer) };
            ui.checkbox(&mut w.picked, line);
        }
    });
    if f.reading > 0 {
        ui.label(RichText::new(tf!("Still reading {n} messages…", n = f.reading)).color(t.dim).small());
    }
    let keys: Vec<String> = f.found.iter().filter(|w| w.picked).map(|w| w.key.to_id()).collect();
    footer_rule(ui, t);
    let mut keep = true;
    ui.allocate_ui_with_layout(vec2(ui.available_width(), 32.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if widgets::accent_button(ui, Icon::Calendar, "Put in calendar", t, !keys.is_empty()).clicked() {
            acts.push(Action::Submit("invite.placeWaiting", json!({"keys": keys})));
        }
        if ui.button(tl!("Close")).clicked() {
            keep = false;
        }
    });
    keep
}
