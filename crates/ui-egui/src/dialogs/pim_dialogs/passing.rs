//! Passing on a meeting the user is invited to: forwarding it (issue #234: one more guest) or
//! delegating it (issue #254: someone else goes in the user's place). Who to is asked in a box of
//! people that suggests them as a message's To does; a series opened on one occurrence can be
//! passed on for that occurrence alone (issue #234: forwarded, the whole series went).

use super::*;
use crate::look::people_box::{self, PeopleBox};

/// What's done with the meeting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Passing {
    Forward,
    Delegate,
}

/// The form asking whom a meeting is passed on to.
#[derive(Clone, Debug)]
pub struct PassForm {
    pub passing: Passing,
    pub uid: String,
    pub summary: String,
    /// Who to, as typed.
    pub to: String,
    pub people: PeopleBox,
    /// The occurrence of a series the meeting's form was open on: it alone may be passed on.
    pub occurrence: Option<i64>,
    /// That occurrence alone, not the series.
    pub only_this: bool,
}

impl PassForm {
    pub fn new(passing: Passing, uid: &str, summary: &str, occurrence: Option<i64>) -> Box<PassForm> {
        Box::new(PassForm {
            passing,
            uid: uid.to_string(),
            summary: summary.to_string(),
            to: String::new(),
            people: PeopleBox::default(),
            occurrence,
            only_this: occurrence.is_some(),
        })
    }
}

/// The form; whether it stays open.
pub fn pass(ui: &mut Ui, f: &mut PassForm, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    ui.set_width(420.0);
    let delegate = f.passing == Passing::Delegate;
    crate::dialogs::title(ui, t, if delegate { "Delegate meeting" } else { "Forward meeting" });
    ui.add(egui::Label::new(RichText::new(f.summary.as_str()).color(t.dim)).truncate());
    if delegate {
        ui.add(
            egui::Label::new(
                RichText::new(tl!("Someone else goes in your place: they get the invitation to answer, and the organizer is told."))
                    .color(t.dim)
                    .small(),
            )
            .wrap(),
        );
    }
    ui.add_space(6.0);
    let name = tl!(if delegate { "Who goes instead of you" } else { "To" });
    ui.label(name);
    let hint = if delegate { tl!("Name <address>") } else { tl!("Names <addresses>, separated by ; or ,") };
    people_box::in_dialog(ui, t, "pass", name, &mut f.to, hint, f32::INFINITY, &mut f.people);
    if f.occurrence.is_some() {
        ui.add_space(4.0);
        ui.radio_value(&mut f.only_this, true, tl!("This occurrence"));
        ui.radio_value(&mut f.only_this, false, tl!("The whole series"));
    }
    let (ok, cancel) = crate::dialogs::buttons(ui, t, if delegate { "Delegate" } else { "Forward" }, !f.to.trim().is_empty());
    if ok {
        let mut p = json!({"uid": f.uid, "to": f.to.trim()});
        if f.only_this {
            p["occurrence"] = json!(f.occurrence);
        }
        acts.push(Action::Submit(if delegate { "invite.delegate" } else { "invite.forward" }, p));
    }
    !cancel
}
