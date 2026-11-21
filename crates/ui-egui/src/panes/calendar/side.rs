//! The calendar's side pane: the months to jump around in, then each account's calendars, which
//! to show, which new events go to, and moving those kept on this computer to the server.

use egui::{RichText, Sense, Ui, pos2, vec2};
use mailcraft_engine::model::pim::{CalendarInfo, is_server_calendar};
use mailcraft_engine::{FreeBusyState, LocalEvents, Session};
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
        let groups = groups(s);
        let grouped = groups.len() > 1;
        for (a, mailbox, cals) in groups {
            let Some(p) = s.pim.get(&a.config.id) else { continue };
            if grouped {
                group_heading(ui, &t, a, mailbox.as_deref());
            }
            let rows = Rows {
                a,
                p,
                grouped,
                default: p.default_calendar().map(|c| c.id.clone()),
                choices: p.calendars.iter().filter(|c| !c.read_only && is_server_calendar(&c.id)).count(),
            };
            for c in cals {
                calendar_row(s, &t, ui, &rows, c, acts);
            }
            // Events kept on this computer while the server's calendars weren't known: offered
            // to move there (issue #30), once per account.
            if mailbox.is_none()
                && let Some(offer) = s.local_events(&a.config.id)
            {
                local_events(ui, &t, &a.config.id, &offer, acts);
            }
        }
        // Calendar groups (issue #140), under the accounts' calendars.
        for a in &s.accounts {
            let Some(p) = s.pim.get(&a.config.id) else { continue };
            let rows = Rows {
                a,
                p,
                grouped: true,
                default: p.default_calendar().map(|c| c.id.clone()),
                choices: p.calendars.iter().filter(|c| !c.read_only && is_server_calendar(&c.id)).count(),
            };
            for (g, shown) in s.calendar_groups(&a.config.id) {
                calendar_group_heading(&t, ui, &a.config.id, &g.name, shown, acts);
                for e in &g.members {
                    let id = mailcraft_engine::model::pim::shared_calendar_id(e);
                    if let Some(c) = p.calendars.iter().find(|c| c.id == id) {
                        calendar_row(s, &t, ui, &rows, c, acts);
                    }
                }
            }
        }
        more(s, ui, several, acts);
    });
}

/// An account's calendars, with the name of the shared mailbox they're of.
type Group<'a> = (&'a mailcraft_engine::AccountState, Option<String>, Vec<&'a CalendarInfo>);

/// Several mailboxes: each one's calendars under its address, in its colour (as Outlook groups
/// them), since each has its own "Calendar"; a shared mailbox's under its own name, in the
/// folder pane's order (issue #1).
fn groups(s: &Session) -> Vec<Group<'_>> {
    let sections = s.pane_sections();
    sections
        .iter()
        .filter_map(|sec| {
            let a = s.account(&sec.account)?;
            let p = s.pim.get(&a.config.id)?;
            let mailbox = sec.key.split_once('|').map(|(_, m)| m.to_string());
            // A calendar group's people are listed under the group (issue #140).
            let in_group = |o: &str| p.groups.iter().any(|g| g.members.iter().any(|m| m == o));
            let in_section = |c: &&CalendarInfo| {
                let owner = c.id.strip_prefix("shared:").map(str::to_ascii_lowercase);
                if owner.as_deref().is_some_and(in_group) {
                    return false;
                }
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
        .collect()
}

/// A group's heading: the account's colour and address, or a shared mailbox's name.
fn group_heading(ui: &mut Ui, t: &Tokens, a: &mailcraft_engine::AccountState, mailbox: Option<&str>) {
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(8.0, 8.0), Sense::hover());
        if mailbox.is_some() {
            icons::paint(ui.painter(), r.expand(3.0), Icon::People, t.dim);
        } else {
            ui.painter().circle_filled(r.center(), 4.0, theme::account_color(a.config.color));
        }
        let name = mailbox.map_or_else(|| a.config.email.clone(), str::to_string);
        ui.add(egui::Label::new(RichText::new(name).font(theme::bold(12.0)).color(t.dim)).truncate());
    });
}

/// A calendar group's heading: ticked when all its calendars are shown (a click shows or hides
/// them together), its name; right-click to delete it.
fn calendar_group_heading(t: &Tokens, ui: &mut Ui, account: &str, name: &str, shown: bool, acts: &mut Vec<Action>) {
    ui.add_space(4.0);
    let row = ui.horizontal(|ui| {
        ui.set_min_width(ui.available_width());
        let (r, resp) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::click());
        if shown {
            ui.painter().rect_filled(r, 3.0, t.accent);
            let p = |x: f32, y: f32| pos2(r.min.x + r.width() * x, r.min.y + r.height() * y);
            ui.painter().line_segment([p(0.22, 0.52), p(0.42, 0.72)], egui::Stroke::new(1.8, egui::Color32::WHITE));
            ui.painter().line_segment([p(0.42, 0.72), p(0.78, 0.3)], egui::Stroke::new(1.8, egui::Color32::WHITE));
        } else {
            ui.painter().rect_stroke(r.shrink(0.5), 3.0, egui::Stroke::new(1.5, t.accent), egui::StrokeKind::Inside);
        }
        let tip = if shown { tl!("Shown: click to hide them") } else { tl!("Hidden: click to show them") };
        crate::widgets::named(ui, &resp, &format!("{name}: {tip}"), true, Some(shown));
        if resp.on_hover_text(tip).clicked() {
            acts.push(Action::Run("calendar.groupShow", json!({"account": account, "name": name, "shown": !shown})));
        }
        ui.add(egui::Label::new(RichText::new(name).font(theme::bold(12.0)).color(t.dim)).selectable(false).truncate());
    });
    let (account, name) = (account.to_string(), name.to_string());
    row.response.interact(Sense::click()).context_menu(|ui| {
        // More people put in it (issue #209), picked as a new group's are.
        if ui.button(tl!("Add people…")).clicked() {
            let mut d = Dialog::names_for(crate::dialogs::NamesFor::Group, "");
            if let Dialog::SelectNames(f) = &mut d {
                f.group.clone_from(&name);
                f.adding = true;
            }
            acts.push(Action::Dialog(d));
            ui.close();
        }
        if ui.button(tl!("Delete calendar group")).clicked() {
            acts.push(Action::Run("calendar.groupDelete", json!({"account": account, "name": name})));
            ui.close();
        }
    });
}

/// What an account's calendar rows share.
struct Rows<'a> {
    a: &'a mailcraft_engine::AccountState,
    p: &'a mailcraft_engine::model::pim::Pim,
    /// Under a group's heading: set in.
    grouped: bool,
    /// The calendar new events go to, and how many it could be.
    default: Option<String>,
    choices: usize,
}

/// A calendar's row: ticked when shown, its name (where it's from, on hover) and what it is
/// (read-only, the default); right-click for what can be done with it.
fn calendar_row(s: &Session, t: &Tokens, ui: &mut Ui, rows: &Rows<'_>, c: &CalendarInfo, acts: &mut Vec<Action>) {
    let Rows { a, p, grouped, ref default, choices } = *rows;
    let shared = c.id.strip_prefix("shared:").map(str::to_string);
    // A shared mailbox's calendar comes with the mailbox; only one opened by hand closes.
    let opened = shared.as_ref().is_some_and(|e| p.shared.iter().any(|x| x.eq_ignore_ascii_case(e)));
    let shown = s.calendar_shown(&a.config.id, &c.id);
    // Subscribed to by its link (issue #7): where it's read from, and how that went.
    let web = p.subscribed.iter().find(|w| w.id == c.id);
    // The width of the pane, its name and tags no more than drawn on it: a right-click anywhere on
    // it opens its menu (issue #119: its name, a text that could be selected, took the click, and
    // the row was as wide as what's in it, so Remove couldn't be reached).
    let row = ui.horizontal(|ui| {
        ui.set_min_width(ui.available_width());
        if grouped {
            ui.add_space(10.0);
        }
        // Ticked: shown (in its colour); click to leave it out, as Outlook's boxes.
        let (r, resp) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::click());
        let c32 = color(Session::calendar_color(a, c), t);
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
        let name = calendar_name(&c.id, &c.name);
        // Shared for its free/busy time alone (issue #140): how asking for those went.
        let free_busy = s.free_busy_state(&a.config.id, &c.id);
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
        let tip = match free_busy {
            None => tip,
            Some(state) => {
                let how = match state {
                    FreeBusyState::Asking => tl!("Asking for its free/busy time…").to_string(),
                    FreeBusyState::Shown => tl!("Shared for its free/busy time alone").to_string(),
                    FreeBusyState::Failed(e) => tf!("Its free/busy time couldn't be had: {error}", error = e),
                };
                format!("{tip}\n{how}")
            }
        };
        ui.add(egui::Label::new(RichText::new(name).color(t.text)).selectable(false)).on_hover_text(tip);
        if web.is_some_and(|w| w.error.is_some()) || matches!(free_busy, Some(FreeBusyState::Failed(_))) {
            ui.add(egui::Label::new(RichText::new(tl!("couldn't be read")).color(t.danger).small()).selectable(false));
        } else if free_busy.is_some() {
            ui.add(egui::Label::new(RichText::new(tl!("free/busy")).color(t.faint).small()).selectable(false));
        } else if c.read_only {
            ui.add(egui::Label::new(RichText::new(tl!("read-only")).color(t.faint).small()).selectable(false));
        } else if default.as_deref() == Some(c.id.as_str()) && choices > 1 {
            ui.add(egui::Label::new(RichText::new(tl!("default")).color(t.faint).small()).selectable(false))
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

/// Under the calendars: side by side, opening a shared calendar, subscribing to one.
fn more(s: &Session, ui: &mut Ui, several: bool, acts: &mut Vec<Action>) {
    // Each in a column of its own, or all together (Outlook's side-by-side view).
    if several {
        ui.add_space(4.0);
        let mut on = s.prefs.calendars_side_by_side;
        if ui.checkbox(&mut on, tl!("Side by side")).on_hover_text(tl!("Day and week show each calendar in a pane of its own")).changed() {
            acts.push(Action::Run("calendar.sideBySide", json!({"on": on})));
        }
    }
    // Outlook's Open Calendar: someone's calendar shared with an Exchange account; and its
    // calendar groups (issue #140).
    if s.accounts.iter().any(|a| a.config.graph.is_some() || a.config.ews.is_some()) {
        ui.add_space(6.0);
        if ui.link(RichText::new(tl!("Open shared calendar…")).small()).clicked() {
            acts.push(Action::Dialog(Dialog::OpenSharedCalendar { email: String::new(), error: None }));
        }
        ui.add_space(4.0);
        if ui.link(RichText::new(tl!("New calendar group…")).small()).clicked() {
            acts.push(Action::Dialog(Dialog::names_for(crate::dialogs::NamesFor::Group, "")));
        }
    }
    // Outlook's Add Calendar › From Internet (issue #7).
    if !s.accounts.is_empty() {
        ui.add_space(4.0);
        if ui.link(RichText::new(tl!("Subscribe to a calendar…")).small()).clicked() {
            acts.push(Action::Dialog(Dialog::SubscribeCalendar { url: String::new(), name: String::new(), error: None }));
        }
        // Meetings in the Inbox from before the account came here (issue #237).
        ui.add_space(4.0);
        if ui.link(RichText::new(tl!("Invitations in the Inbox…")).small()).clicked() {
            acts.push(Action::Dialog(Dialog::WaitingInvites(Box::default())));
        }
    }
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
