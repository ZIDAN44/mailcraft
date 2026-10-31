//! The calendar's side pane: the months to jump around in, then each account's calendars, which
//! to show, which new events go to, and moving those kept on this computer to the server.

use egui::{RichText, Sense, Ui, pos2, vec2};
use mailcraft_engine::model::pim::is_server_calendar;
use mailcraft_engine::{LocalEvents, Session};
use serde_json::json;

use super::*;

/// The side pane: a month to jump around in, then the calendars.
pub fn nav(s: &Session, st: &mut UiState, ui: &mut Ui, acts: &mut Vec<Action>) {
    let t = st.tokens;
    egui::ScrollArea::vertical().auto_shrink(false).id_salt("calendar-nav").show(ui, |ui| {
        if widgets::accent_button(ui, Icon::Plus, "New event", &t, !s.accounts.is_empty()).clicked() {
            acts.push(Action::Dialog(Dialog::new_event(s, None)));
        }
        ui.add_space(10.0);
        months(s, st, ui, acts);
        ui.add_space(12.0);
        ui.label(RichText::new(tl!("Calendars")).font(theme::bold(12.0)).color(t.faint));
        let several = s.pim.values().map(|p| p.calendars.len()).sum::<usize>() > 1;
        // Several mailboxes: each one's calendars under its address, in its colour (as Outlook
        // groups them), since each has its own "Calendar"; a shared mailbox's under its own name,
        // in the folder pane's order (issue #1).
        let sections = s.pane_sections();
        let groups: Vec<(&mailcraft_engine::AccountState, Option<String>, Vec<&mailcraft_engine::model::pim::CalendarInfo>)> = sections
            .iter()
            .filter_map(|sec| {
                let a = s.account(&sec.account)?;
                let p = s.pim.get(&a.config.id)?;
                let mailbox = sec.key.split_once('|').map(|(_, m)| m.to_string());
                let in_section = |c: &&mailcraft_engine::model::pim::CalendarInfo| {
                    let owner = c.id.strip_prefix("shared:").map(str::to_ascii_lowercase);
                    match &mailbox {
                        Some(m) => owner.as_deref() == Some(m.as_str()),
                        None => owner.is_none_or(|o| !sections.iter().any(|x| x.key == format!("{}|{o}", a.config.id))),
                    }
                };
                let cals: Vec<_> = p.calendars.iter().filter(in_section).collect();
                let label = match (&mailbox, sec.mailbox.as_ref().and_then(|path| a.folders.iter().find(|f| &f.path == path))) {
                    (Some(m), Some(top)) => Some(format!("{} ({m})", crate::i18n::folder(top))),
                    (Some(m), None) => Some(m.clone()),
                    _ => None,
                };
                (!cals.is_empty()).then_some((a, label, cals))
            })
            .collect();
        let grouped = groups.len() > 1;
        for (a, mailbox, cals) in groups {
            let Some(p) = s.pim.get(&a.config.id) else { continue };
            if grouped {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(8.0, 8.0), Sense::hover());
                    if mailbox.is_some() {
                        icons::paint(ui.painter(), r.expand(3.0), Icon::People, t.dim);
                    } else {
                        ui.painter().circle_filled(r.center(), 4.0, theme::account_color(a.config.color));
                    }
                    let name = mailbox.clone().unwrap_or_else(|| a.config.email.clone());
                    ui.add(egui::Label::new(RichText::new(name).font(theme::bold(12.0)).color(t.dim)).truncate());
                });
            }
            let default = p.default_calendar().map(|c| c.id.clone());
            let choices = p.calendars.iter().filter(|c| !c.read_only && is_server_calendar(&c.id)).count();
            for c in cals {
                let shared = c.id.strip_prefix("shared:").map(str::to_string);
                // A shared mailbox's calendar comes with the mailbox; only one opened by hand closes.
                let opened = shared.as_ref().is_some_and(|e| p.shared.iter().any(|x| x.eq_ignore_ascii_case(e)));
                let shown = s.calendar_shown(&a.config.id, &c.id);
                // Subscribed to by its link (issue #7): where it's read from, and how that went.
                let web = p.subscribed.iter().find(|w| w.id == c.id);
                let row = ui.horizontal(|ui| {
                    if grouped {
                        ui.add_space(10.0);
                    }
                    // Ticked: shown (in its colour); click to leave it out, as Outlook's boxes.
                    let (r, resp) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::click());
                    let c32 = color(Session::calendar_color(a, c), &t);
                    if shown {
                        ui.painter().rect_filled(r, 3.0, c32);
                        let p = |x: f32, y: f32| pos2(r.min.x + r.width() * x, r.min.y + r.height() * y);
                        ui.painter().line_segment([p(0.22, 0.52), p(0.42, 0.72)], egui::Stroke::new(1.8, egui::Color32::WHITE));
                        ui.painter().line_segment([p(0.42, 0.72), p(0.78, 0.3)], egui::Stroke::new(1.8, egui::Color32::WHITE));
                    } else {
                        ui.painter().rect_stroke(r.shrink(0.5), 3.0, egui::Stroke::new(1.5, c32), egui::StrokeKind::Inside);
                    }
                    let tip = if shown { tl!("Shown: click to hide it") } else { tl!("Hidden: click to show it") };
                    if resp.on_hover_text(tip).clicked() {
                        acts.push(Action::Run("calendar.show", json!({"account": a.config.id, "calendar": c.id, "shown": !shown})));
                    }
                    let name = if c.id == mailcraft_engine::model::pim::LOCAL { tl!("Calendar").to_string() } else { c.name.clone() };
                    let tip = if let Some(owner) = shared.as_ref().filter(|_| !opened) {
                        tf!("Shared mailbox {address}, opened by {email}", address = owner, email = a.config.email)
                    } else if shared.is_some() {
                        tf!("Shared with you, opened from {account}", account = a.config.email)
                    } else if let Some(w) = web {
                        let state = match &w.error {
                            Some(e) => tf!("Couldn't be read: {error}", error = e),
                            None if w.fetched == 0 => tl!("Reading it…").to_string(),
                            None => tf!("Read {time}", time = crate::i18n::date(&mailcraft_engine::model::dates::list_time(w.fetched, &s.clock))),
                        };
                        format!("{}\n{state}", w.url)
                    } else {
                        a.config.email.clone()
                    };
                    ui.label(RichText::new(name).color(t.text)).on_hover_text(tip);
                    if web.is_some_and(|w| w.error.is_some()) {
                        ui.label(RichText::new(tl!("couldn't be read")).color(t.danger).small());
                    } else if c.read_only {
                        ui.label(RichText::new(tl!("read-only")).color(t.faint).small());
                    } else if default.as_deref() == Some(c.id.as_str()) && choices > 1 {
                        ui.label(RichText::new(tl!("default")).color(t.faint).small())
                            .on_hover_text(tl!("New events and invitations go to this calendar"));
                    }
                });
                // One of the account's that can be added to: right-click makes it the one new
                // events and invitations go to (issue #30).
                if choices > 1 && !c.read_only && is_server_calendar(&c.id) && default.as_deref() != Some(c.id.as_str()) {
                    let (account, id) = (a.config.id.clone(), c.id.clone());
                    row.response.interact(Sense::click()).context_menu(|ui| {
                        if ui.button(tl!("Use as default calendar")).clicked() {
                            acts.push(Action::Run("calendar.setDefault", json!({"account": account, "calendar": id})));
                            ui.close();
                        }
                    });
                }
                // Subscribed to: right-click reads it again, or removes it.
                if web.is_some() {
                    let id = c.id.clone();
                    row.response.interact(Sense::click()).context_menu(|ui| {
                        if ui.button(tl!("Refresh")).clicked() {
                            acts.push(Action::Run("calendar.refreshSubscribed", json!({"calendar": id})));
                            ui.close();
                        }
                        if ui.button(tl!("Remove this calendar")).clicked() {
                            acts.push(Action::Run("calendar.unsubscribe", json!({"calendar": id})));
                            ui.close();
                        }
                    });
                }
                // Someone else's calendar opened by hand: right-click closes it.
                if let Some(email) = shared.filter(|_| opened) {
                    row.response.interact(Sense::click()).context_menu(|ui| {
                        if ui.button(tl!("Remove this calendar")).clicked() {
                            acts.push(Action::Run("calendar.closeShared", json!({"email": email})));
                            ui.close();
                        }
                    });
                }
            }
            // Events kept on this computer while the server's calendars weren't known: offered
            // to move there (issue #30), once per account.
            if mailbox.is_none()
                && let Some(offer) = s.local_events(&a.config.id)
            {
                local_events(ui, &t, &a.config.id, &offer, acts);
            }
        }
        // Each in a column of its own, or all together (Outlook's side-by-side view).
        if several {
            ui.add_space(4.0);
            let mut on = s.prefs.calendars_side_by_side;
            if ui.checkbox(&mut on, tl!("Side by side")).on_hover_text(tl!("Day and week show each calendar in a pane of its own")).changed() {
                acts.push(Action::Run("calendar.sideBySide", json!({"on": on})));
            }
        }
        // Outlook's Open Calendar: someone's calendar shared with an Exchange account.
        if s.accounts.iter().any(|a| a.config.graph.is_some() || a.config.ews.is_some()) {
            ui.add_space(6.0);
            if ui.link(RichText::new(tl!("Open shared calendar…")).small()).clicked() {
                acts.push(Action::Dialog(Dialog::OpenSharedCalendar { email: String::new(), error: None }));
            }
        }
        // Outlook's Add Calendar › From Internet (issue #7).
        if !s.accounts.is_empty() {
            ui.add_space(4.0);
            if ui.link(RichText::new(tl!("Subscribe to a calendar…")).small()).clicked() {
                acts.push(Action::Dialog(Dialog::SubscribeCalendar { url: String::new(), name: String::new(), error: None }));
            }
        }
    });
}

/// "Events kept on this computer only: 3 — Move them to Personal Calendar · Keep them here".
fn local_events(ui: &mut Ui, t: &Tokens, account: &str, offer: &LocalEvents, acts: &mut Vec<Action>) {
    ui.add_space(4.0);
    egui::Frame::new().fill(t.hover).corner_radius(4.0).inner_margin(egui::Margin::same(6)).show(ui, |ui| {
        ui.label(RichText::new(tf!("Events kept on this computer only: {count}", count = offer.count)).color(t.dim).small());
        ui.horizontal_wrapped(|ui| {
            if ui.link(RichText::new(tf!("Move them to {calendar}", calendar = offer.calendar.name)).small()).clicked() {
                acts.push(Action::Run("calendar.moveLocal", json!({"account": account, "calendar": offer.calendar.id})));
            }
            if ui.link(RichText::new(tl!("Keep them here")).small()).clicked() {
                acts.push(Action::Run("calendar.keepLocal", json!({"account": account})));
            }
        });
    });
}
