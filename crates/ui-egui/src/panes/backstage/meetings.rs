//! File › Options › Online meetings (issue #25): the user's own rooms, added to a meeting with
//! Add online meeting; the hosts whose links join a meeting; and adding one to every meeting
//! with guests.

use super::*;

/// What's being typed into one of the card's boxes, kept between frames.
fn typed(ui: &Ui, key: &str) -> String {
    ui.data_mut(|d| d.get_temp::<String>(egui::Id::unique(("meetings-typed", key)))).unwrap_or_default()
}

fn keep_typed(ui: &Ui, key: &str, text: String) {
    ui.data_mut(|d| d.insert_temp(egui::Id::unique(("meetings-typed", key)), text));
}

pub(super) fn meetings(s: &Session, st: &UiState, ui: &mut Ui, acts: &mut Vec<Action>) {
    let t = st.tokens;
    card(
        ui,
        &t,
        tl!("Online meetings"),
        Some(tl!(
            "Your own meeting rooms (a BigBlueButton, Jitsi or other room's link, and its access code). Add online meeting in an event adds one: its link as the place, and how to join in the notes."
        )),
        |ui| {
            let mut on = s.prefs.meeting_for_all;
            setting_row(
                ui,
                &t,
                true,
                name(&t, tl!("Add an online meeting to all meetings with guests")),
                Some(note(&t, tl!("The first room's, as the meeting is made"))),
                |ui| {
                    if widgets::switch(ui, &t, &mut on, tl!("Add an online meeting to all meetings with guests")).changed() {
                        acts.push(Action::Run("meetings.forAll", json!({"on": on})));
                    }
                },
            );
            for (i, r) in s.prefs.meeting_rooms.iter().enumerate() {
                let mut about = if r.code.is_empty() { r.url.clone() } else { format!("{} · {}", r.url, tf!("access code {code}", code = r.code)) };
                if let Some(first) = r.details.lines().next() {
                    about.push_str(&format!(" · {}", crate::widgets::shorten(first, 40)));
                }
                setting_row(ui, &t, false, name(&t, &r.name), Some(note(&t, &about)), |ui| {
                    if ui.button(tl!("Remove")).clicked() {
                        acts.push(Action::Run("meetings.removeRoom", json!({"room": r.url})));
                    }
                    if i > 0
                        && ui.button(tl!("Make default")).on_hover_text(tl!("Add online meeting adds this room unless another is chosen")).clicked()
                    {
                        acts.push(Action::Run("meetings.defaultRoom", json!({"room": r.url})));
                    }
                });
            }
            ui.add_space(6.0);
            let (mut room, mut url, mut code, mut details) = (typed(ui, "room"), typed(ui, "url"), typed(ui, "code"), typed(ui, "details"));
            let mut add = false;
            ui.horizontal_wrapped(|ui| {
                ui.add(egui::TextEdit::singleline(&mut room).hint_text(tl!("Name")).desired_width(140.0));
                let r = ui.add(egui::TextEdit::singleline(&mut url).hint_text("https://meet.example.org/rooms/abc-def-ghi").desired_width(300.0));
                add |= r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                ui.add(egui::TextEdit::singleline(&mut code).hint_text(tl!("Access code")).desired_width(110.0));
                add |= ui.add_enabled(!url.trim().is_empty(), egui::Button::new(tl!("Add room"))).clicked();
            });
            // Dial-in numbers and the like, for the block in the invitation.
            ui.add(
                egui::TextEdit::multiline(&mut details)
                    .hint_text(tl!("Dial-in numbers and other lines for the invitation (optional)"))
                    .desired_rows(2)
                    .desired_width(ui.available_width().min(560.0)),
            );
            if add && !url.trim().is_empty() {
                acts.push(Action::Run(
                    "meetings.addRoom",
                    json!({"name": room.trim(), "url": url.trim(), "code": code.trim(), "details": details.trim()}),
                ));
                // One that can't be added stays to be corrected (the reason shows as a notice).
                if mailcraft_engine::model::ical::meeting_url(&url).is_some() {
                    room.clear();
                    url.clear();
                    code.clear();
                    details.clear();
                }
            }
            keep_typed(ui, "room", room);
            keep_typed(ui, "url", url);
            keep_typed(ui, "code", code);
            keep_typed(ui, "details", details);
            ui.add_space(10.0);

            // Other organisations' self-hosted servers: their links get Join meeting too.
            ui.label(name(&t, tl!("Meeting hosts")));
            ui.label(note(
                &t,
                tl!(
                    "Links to these servers join a meeting (Teams, Zoom, Google Meet, Webex, BigBlueButton rooms and your rooms' servers already do)"
                ),
            ));
            for (i, h) in s.prefs.meeting_hosts.iter().enumerate() {
                setting_row(ui, &t, i == 0, name(&t, h), None, |ui| {
                    if ui.button(tl!("Remove")).clicked() {
                        acts.push(Action::Run("meetings.removeHost", json!({"host": h})));
                    }
                });
            }
            let mut host = typed(ui, "host");
            let mut add = false;
            ui.horizontal(|ui| {
                let r = ui.add(egui::TextEdit::singleline(&mut host).hint_text("jitsi.example.org").desired_width(300.0));
                add |= r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                add |= ui.add_enabled(!host.trim().is_empty(), egui::Button::new(tl!("Add"))).clicked();
            });
            if add && !host.trim().is_empty() {
                acts.push(Action::Run("meetings.addHost", json!({"host": host.trim()})));
                if mailcraft_engine::model::ical::host_entry(&host).is_some() {
                    host.clear();
                }
            }
            keep_typed(ui, "host", host);
            ui.add_space(8.0);
        },
    );
}
