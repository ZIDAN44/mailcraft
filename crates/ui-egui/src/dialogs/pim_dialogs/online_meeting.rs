//! Add online meeting in the event form (issue #25), as Outlook's Teams Meeting button: one of
//! the user's rooms (File › Options › Online meetings) goes into the place and the notes at once,
//! where it can be read and edited before Save; Remove online meeting takes it out again. With
//! several rooms the button has a menu of them. With "Add an online meeting to all meetings with
//! guests" on, a new meeting gets the first room's as soon as it has guests.

use egui::{RichText, Ui};
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

/// The button row: Add online meeting (a menu of rooms when there are several), or the meeting's
/// link and Remove online meeting when it has one.
pub fn row(ui: &mut Ui, form: &mut EventForm, t: &Tokens, acts: &mut Vec<Action>) {
    if form.read_only || !form.can_invite {
        return;
    }
    add_for_all(form);
    ui.horizontal(|ui| {
        if let Some(url) = form.meeting.current.clone() {
            ui.label(RichText::new(tl!("Online meeting")).color(t.dim));
            if ui.link(crate::widgets::shorten(url.split_once("://").map_or(url.as_str(), |(_, r)| r), 48)).on_hover_text(&url).clicked() {
                acts.push(Action::OpenUrl(url.clone()));
            }
            if ui.button(tl!("Remove online meeting")).clicked() {
                remove(form);
            }
            return;
        }
        let first = form.meeting.rooms.first().cloned();
        let hint = match &first {
            Some(r) => tf!("Adds {room}: its link as the place, and how to join in the notes", room = r.name),
            None => tl!("Add your meeting rooms under File › Options › Online meetings").to_string(),
        };
        if ui
            .add_enabled(first.is_some(), egui::Button::new(tl!("Add online meeting")))
            .on_hover_text(hint)
            .on_disabled_hover_text(tl!("Add your meeting rooms under File › Options › Online meetings"))
            .clicked()
            && let Some(r) = &first
        {
            add(form, r);
        }
        // Several rooms: a split button, the arrow choosing which.
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
    });
    ui.add_space(8.0);
}
