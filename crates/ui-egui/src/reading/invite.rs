//! A meeting invitation in the open message (or an answer to one, or a cancellation): its card,
//! with what can be done with it, and how it fits the calendar (`day`).

use super::*;

mod day;

#[cfg(test)]
pub(crate) use day::repeats as invite_repeats_for_tests;

/// When an invitation's event is, as its card says: the day alone for an all-day one (its start
/// is a local midnight), else the date and time it starts and the time it ends.
pub(crate) fn invite_when(e: &mailcraft_engine::model::ical::CalEvent, s: &Session) -> String {
    if e.all_day {
        crate::i18n::date(&dates::day_text(s.clock.local_day(s.clock.utc(e.start)), &s.clock))
    } else {
        format!("{} – {}", crate::i18n::date(&dates::full_time(e.start, &s.clock)), crate::calendar::hm(e.end, s))
    }
}

/// A meeting invitation (or an answer to one, or a cancellation) carried by the message.
pub(super) fn invite_card(s: &Session, st: &mut UiState, ui: &mut Ui, key: &MessageKey, acts: &mut Vec<Action>) {
    // Its files still coming: looked for once they're here (what's shown, not read again).
    if s.reading.as_ref().is_some_and(|r| &r.key == key && r.text_first) {
        return;
    }
    let fresh = st.invite.as_ref().is_some_and(|(k, rev, _, _)| k == key && *rev == s.pim_revision);
    if !fresh {
        let inv = s.invite();
        // How a request fits the calendar: worked out with it, not each frame.
        let day = inv.as_ref().filter(|i| i.method == "REQUEST" && i.problem.is_none()).and_then(|i| s.invite_day(&i.event));
        st.invite = Some((key.clone(), s.pim_revision, inv, day));
    }
    let Some(inv) = st.invite.as_ref().and_then(|(_, _, i, _)| i.clone()) else { return };
    let day = st.invite.as_ref().and_then(|(_, _, _, d)| d.clone());
    let t = st.tokens;
    let e = &inv.event;
    egui::Frame::new().fill(t.accent_soft).corner_radius(8).inner_margin(Margin::same(12)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::hover());
            icons::paint(ui.painter(), r, Icon::Calendar, t.accent);
            ui.vertical(|ui| {
                ui.label(RichText::new(&e.summary).font(theme::bold(15.0)).color(t.text));
                ui.label(RichText::new(invite_when(e, s)).color(t.dim));
                if let Some(rule) = e.rrule.as_deref().filter(|r| !r.trim().is_empty()) {
                    ui.label(RichText::new(day::repeats(rule)).color(t.dim));
                }
                if !e.location.is_empty() {
                    ui.label(RichText::new(&e.location).color(t.dim));
                }
            });
        });
        ui.add_space(6.0);
        // One that can't be taken in says why, and offers nothing to do.
        if let Some(why) = inv.problem {
            let text = match why {
                InviteProblem::NotFromOrganizer => tl!("This update wasn't sent by the meeting's organizer, so it can't change your calendar."),
                InviteProblem::NotFromGuest => tl!("This answer wasn't sent by the guest it's from, so it can't be recorded."),
                InviteProblem::OutOfDate => tl!("This invitation is out of date: your calendar has a newer version."),
            };
            ui.label(RichText::new(text).color(t.danger));
            return;
        }
        match inv.method.as_str() {
            "REQUEST" => {
                // How it fits the calendar, and the hours around it (issue #194).
                if let Some(day) = &day {
                    day::show(s, &t, ui, e, day, acts);
                    ui.add_space(6.0);
                }
                ui.horizontal_wrapped(|ui| {
                    let current = match inv.answer.as_str() {
                        "ACCEPTED" => Some("You accepted."),
                        "TENTATIVE" => Some("You said maybe."),
                        "DECLINED" => Some("You declined."),
                        _ => None,
                    };
                    if let Some(c) = current {
                        ui.label(RichText::new(tl!(c)).color(t.text));
                    }
                    for (label, code, icon) in
                        [("Accept", "accepted", Icon::Read), ("Tentative", "tentative", Icon::Clock), ("Decline", "declined", Icon::Close)]
                    {
                        if widgets::pill_button(ui, icon, label, &t).clicked() {
                            acts.push(Action::Run("invite.respond", json!({"answer": code})));
                        }
                    }
                    // Passed on to someone else, where the organizer allows it (issue #234).
                    if mailcraft_engine::forwarding_allowed(e) && widgets::pill_button(ui, Icon::Forward, "Forward", &t).clicked() {
                        acts.push(Action::Dialog(crate::Dialog::ForwardMeeting {
                            uid: e.uid.clone(),
                            summary: e.summary.clone(),
                            to: String::new(),
                        }));
                    }
                    // A new time, where the organizer takes proposals (issue #238).
                    if mailcraft_engine::proposals_allowed(e) && widgets::pill_button(ui, Icon::Clock, "Propose new time", &t).clicked() {
                        acts.push(Action::Dialog(crate::Dialog::Propose(Box::new(crate::pim_dialogs::ProposeForm::new(s, e)))));
                    }
                    if inv.in_calendar && ui.link(tl!("Show in calendar")).clicked() {
                        show_day(e.start, s, acts);
                    }
                });
            }
            // A guest's proposal of a new time for the user's meeting, and the organizer's no to
            // the user's (issue #238).
            "COUNTER" => {
                let name = inv.replier.as_ref().map(|w| if w.name.is_empty() { w.email.clone() } else { w.name.clone() }).unwrap_or_default();
                ui.label(RichText::new(tf!("{name} proposes a new time: {when}", name = name, when = invite_when(e, s))).color(t.text));
                ui.horizontal_wrapped(|ui| {
                    if widgets::pill_button(ui, Icon::Read, "Accept proposal", &t).clicked() {
                        acts.push(Action::Run("invite.acceptProposal", json!({})));
                    }
                    if widgets::pill_button(ui, Icon::Close, "Decline proposal", &t).clicked() {
                        acts.push(Action::Run("invite.declineProposal", json!({})));
                    }
                });
            }
            "DECLINECOUNTER" => {
                ui.label(RichText::new(tl!("The organizer declined your proposed time: the meeting stays as it was.")).color(t.text));
            }
            "CANCEL" => {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(tl!("This event was cancelled.")).color(t.danger));
                    if inv.in_calendar && widgets::pill_button(ui, Icon::Delete, "Remove from calendar", &t).clicked() {
                        acts.push(Action::Run("invite.apply", json!({})));
                    }
                });
            }
            _ => {
                if let Some(who) = &inv.replier {
                    let name = if who.name.is_empty() { who.email.clone() } else { who.name.clone() };
                    let line = match who.partstat.as_str() {
                        "ACCEPTED" => tf!("{name} accepted.", name = name),
                        "DECLINED" => tf!("{name} declined.", name = name),
                        "TENTATIVE" => tf!("{name} said maybe.", name = name),
                        _ => tf!("{name} answered.", name = name),
                    };
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(line).color(t.text));
                        // Recorded as it came, or by the button (which said nothing, issue #236).
                        if inv.recorded {
                            ui.label(RichText::new(tl!("Recorded in your calendar.")).color(t.dim));
                        } else if inv.in_calendar && widgets::pill_button(ui, Icon::Calendar, "Record the answer", &t).clicked() {
                            acts.push(Action::Run("invite.apply", json!({})));
                        }
                    });
                }
            }
        }
    });
    ui.add_space(10.0);
}

/// Show the calendar on the day of a time.
fn show_day(t: i64, s: &Session, acts: &mut Vec<Action>) {
    crate::calendar::show_day(s.clock.local_day(t), acts);
}
