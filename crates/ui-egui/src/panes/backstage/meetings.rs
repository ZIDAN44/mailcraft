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
            // A room added in a dialog of its own (four boxes in a row that wrapped, it was).
            setting_row(ui, &t, false, name(&t, tl!("Add a room")), None, |ui| {
                if ui.button(tl!("Add room…")).clicked() {
                    acts.push(Action::Dialog(Dialog::AddRoom {
                        name: String::new(),
                        url: String::new(),
                        code: String::new(),
                        details: String::new(),
                        error: None,
                    }));
                }
            });
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
            let ok = mailcraft_engine::model::ical::host_entry(&host).is_some();
            if crate::widgets::add_row(ui, &mut host, "jitsi.example.org", ok, tl!("That isn't a server's name, such as jitsi.example.org.")) {
                acts.push(Action::Run("meetings.addHost", json!({"host": host.trim()})));
                host.clear();
            }
            keep_typed(ui, "host", host);
            ui.add_space(8.0);
        },
    );
}
