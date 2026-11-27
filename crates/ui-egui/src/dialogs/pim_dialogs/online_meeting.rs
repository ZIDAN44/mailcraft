//! Add online meeting in the event form (issue #25), as Outlook's Teams Meeting button: one of
//! the user's rooms (File › Options › Online meetings) goes into the place and the notes at once,
//! where it can be read and edited before Save; Remove online meeting takes it out again. With
//! several rooms the button has a menu of them. With "Add an online meeting to all meetings with
//! guests" on, a new meeting gets the first room's as soon as it has guests.

use egui::Ui;
use mailcraft_engine::Session;
use mailcraft_engine::model::ical::{CalEvent, MeetingLabels, MeetingRoom};
use serde_json::{Value, json};

use super::EventForm;
use crate::Action;
use crate::theme::Tokens;

/// The form's online meeting: the rooms to add, and what Save asks of the event.
#[derive(Clone, Debug, Default)]
pub struct MeetingState {
    pub rooms: Vec<MeetingRoom>,
    /// The user's meeting hosts, for whether the event already has a meeting.
    pub hosts: Vec<String>,
    pub for_all: bool,
    /// The link of the meeting in it now (added here, or saved with it).
    pub current: Option<String>,
    /// What Save asks: `Some(Some(room))` add it, `Some(None)` take it out, `None` as it is.
    pub change: Option<Option<MeetingRoom>>,
}

impl MeetingState {
    pub fn new(s: &Session, e: Option<&CalEvent>) -> MeetingState {
        MeetingState {
            rooms: s.prefs.meeting_rooms.clone(),
            hosts: s.meeting_hosts(),
            for_all: s.prefs.meeting_for_all,
            current: e.and_then(CalEvent::online_meeting),
            change: None,
        }
    }
}

/// The block's words, in the user's language: the guests read the invitation in it.
pub fn labels() -> MeetingLabels {
    MeetingLabels { heading: tl!("Online meeting").to_string(), join: tl!("Join:").to_string(), code: tl!("Access code:").to_string() }
}

/// The form's place and notes as an event (with its meeting as a `CONFERENCE`, so the link
/// leaves the place even when its block was edited away), changed by `f`, and put back.
fn change(form: &mut EventForm, f: impl FnOnce(&mut CalEvent)) {
    let extra = form.meeting.current.iter().map(|u| mailcraft_engine::model::ical::conference_line(u, "")).collect();
    let mut e = CalEvent { location: form.location.clone(), description: form.description.clone(), extra, ..Default::default() };
    f(&mut e);
    form.links = e.links();
    form.location = e.location;
    form.description = e.description;
}

/// Add a room's meeting to the form.
pub fn add(form: &mut EventForm, room: &MeetingRoom) {
    let labels = labels();
    change(form, |e| e.add_online_meeting(room, &labels));
    form.meeting.current = Some(room.url.clone());
    form.meeting.change = Some(Some(room.clone()));
}

/// Take the meeting out of the form.
pub fn remove(form: &mut EventForm) {
    change(form, |e| {
        e.remove_online_meeting();
    });
    if form.join.is_some() && form.join == form.meeting.current {
        form.join = None;
    }
    form.meeting.current = None;
    form.meeting.change = Some(None);
}

/// "Add an online meeting to all meetings with guests": a new meeting gets the first room's
/// once it has guests, unless it has a meeting (typed in, or taken out here).
pub fn add_for_all(form: &mut EventForm) {
    let m = &form.meeting;
    if !m.for_all || form.uid.is_some() || m.change.is_some() || m.current.is_some() || form.guests.trim().is_empty() {
        return;
    }
    let typed = CalEvent { location: form.location.clone(), description: form.description.clone(), ..Default::default() };
    if typed.join_link_in(&m.hosts).is_some() {
        return;
    }
    if let Some(room) = m.rooms.first().cloned() {
        add(form, &room);
    }
}

/// What Save sends: the room (with the block's words) or `false`, when the user changed it.
pub fn param(form: &EventForm, p: &mut Value) {
    match &form.meeting.change {
        Some(Some(r)) => {
            let l = labels();
            p["onlineMeeting"] = json!({
                "name": r.name, "url": r.url, "code": r.code, "details": r.details,
                "labels": {"heading": l.heading, "join": l.join, "code": l.code},
            });
        }
        Some(None) => p["onlineMeeting"] = json!(false),
        None => {}
    }
}

/// The form's Online meeting row, under the place: a switch that adds the first room (the arrow
/// beside it choosing another, when there are several) or takes the meeting out, its link after
/// it while it has one. With no rooms set up, no row: a greyed button over the title read as an
/// empty box, the first thing in the form (File › Options › Online meetings sets rooms up).
pub fn row(ui: &mut Ui, form: &mut EventForm, t: &Tokens, acts: &mut Vec<Action>) {
    if form.read_only || !form.can_invite {
        return;
    }
    add_for_all(form);
    let current = form.meeting.current.clone();
    let first = form.meeting.rooms.first().cloned();
    if current.is_none() && first.is_none() {
        return;
    }
    super::form_row(ui, t, true, None, "Online meeting", |ui| {
        let mut on = current.is_some();
        let hint = match &first {
            Some(r) => tf!("Adds {room}: its link as the place, and how to join in the notes", room = r.name),
            None => tl!("Add your meeting rooms under File › Options › Online meetings").to_string(),
        };
        if crate::widgets::switch(ui, t, &mut on, "Online meeting").on_hover_text(hint).changed() {
            match (&first, on) {
                (Some(r), true) => add(form, r),
                (_, false) => remove(form),
                (None, true) => {}
            }
        }
        // Several rooms: which one, from the arrow.
        if form.meeting.rooms.len() > 1 {
            let rooms = form.meeting.rooms.clone();
            let r = crate::widgets::tiny_icon_button(ui, crate::icons::Icon::ChevronDown, "Choose a room", t);
            egui::Popup::menu(&r).show(|ui| {
                for room in &rooms {
                    if ui.button(&room.name).on_hover_text(&room.url).clicked() {
                        add(form, room);
                        ui.close();
                    }
                }
            });
        }
        if let Some(url) = &current
            && ui.link(super::link_label(url, 40)).on_hover_text(url).clicked()
        {
            acts.push(Action::OpenUrl(url.clone()));
        }
    });
}
