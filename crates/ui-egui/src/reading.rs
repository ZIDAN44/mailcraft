//! The reading pane (the open message with its conversation and attachments) and the inline
//! compose form that replaces it while a message is being written.

use egui::{Align, Color32, Margin, Rect, RichText, Sense, Ui, vec2};
use mailcraft_engine::mime::{self, AttachmentKind};
use mailcraft_engine::model::contacts::Contact;
use mailcraft_engine::model::{MessageKey, addr, dates};
use mailcraft_engine::{Session, model::EmailAddress};
use serde_json::json;

use crate::icons::{self, Icon};
use crate::rich_edit;
use crate::theme::{self, Tokens};
use crate::{Action, UiState, widgets};

mod attachments;
mod body;
mod compose;
mod people;
use attachments::*;
pub(crate) use attachments::{big_picture, decode};
use body::*;
pub use compose::compose_window;
#[cfg(test)]
pub(crate) use compose::parse_local;
use compose::*;
use people::*;

pub fn show(s: &mut Session, st: &mut UiState, ui: &mut Ui, acts: &mut Vec<Action>) {
    if s.compose.as_ref().is_some_and(|c| crate::compose_inline(c, ui.ctx())) {
        compose(s, st, ui, acts, false);
        return;
    }
    let t = st.tokens;
    if s.reading.is_none() {
        empty(ui, &t, if s.list.is_empty() { "" } else { tl!("Select a message to read it") });
        return;
    }
    // A message just opened comes in: fading up from a few pixels below, quickly.
    let key = s.reading.as_ref().map(|r| r.key.to_id()).unwrap_or_default();
    let k = crate::motion::entrance(ui.ctx(), egui::Id::unique(("reading", key)), crate::motion::NORMAL);
    ui.scope(|ui| {
        ui.multiply_opacity(k);
        ui.add_space(6.0 * (1.0 - k));
        message(s, st, ui, acts, false);
    });
}

/// A message in a window of its own. What it asks for applies to that message: it becomes the
/// open one first.
pub fn window(s: &mut Session, st: &mut UiState, ui: &mut Ui, w: mailcraft_engine::Reading, acts: &mut Vec<Action>) {
    let key = w.key.clone();
    let saved = s.reading.replace(w);
    let mut mine = Vec::new();
    message(s, st, ui, &mut mine, true);
    s.reading = saved;
    if !mine.is_empty() {
        acts.push(Action::Open(key));
        acts.extend(mine);
    }
}

fn message(s: &mut Session, st: &mut UiState, ui: &mut Ui, acts: &mut Vec<Action>, in_window: bool) {
    let t = st.tokens;
    let Some(r) = s.reading.clone() else { return };
    let summary = s.summary(&r.key).cloned();
    // An attachment shown in place of the text (its chip clicked); Escape goes back.
    let viewing = st
        .attachment_view
        .as_ref()
        .filter(|(k, _)| *k == r.key)
        .map(|(_, i)| *i)
        .filter(|i| r.parsed.as_ref().is_some_and(|p| p.attachments.iter().any(|a| a.index == *i)));
    if viewing.is_some() && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
        close_attachment_view(st);
    }
    let viewing = viewing.filter(|_| st.attachment_view.is_some());
    egui::ScrollArea::vertical().auto_shrink(false).id_salt(("reading", in_window, r.key.to_id(), viewing)).show(ui, |ui| {
        egui::Frame::new().inner_margin(Margin { left: 24, right: 24, top: 18, bottom: 24 }).show(ui, |ui| {
            let Some(p) = &r.parsed else {
                ui.add_space(30.0);
                match &r.error {
                    Some(e) => {
                        ui.horizontal(|ui| {
                            let (ir, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
                            icons::paint(ui.painter(), ir, Icon::Warning, t.warn);
                            ui.label(RichText::new(tf!("This message couldn't be downloaded: {error}", error = e)).color(t.dim));
                        });
                        if ui.button(tl!("Try again")).clicked() {
                            acts.push(Action::Open(r.key.clone()));
                        }
                    }
                    None => {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(RichText::new(tl!("Downloading the message…")).color(t.dim));
                        });
                    }
                }
                return;
            };
            // Previewing an attachment: back to the message, and what can be done with the file.
            if let Some(index) = viewing
                && let Some(a) = p.attachments.iter().find(|a| a.index == index)
            {
                preview_header(&t, ui, a, acts, &mut st.attachment_view);
                if st.attachment_view.is_none() {
                    close_attachment_view(st);
                }
            }
            if viewing.is_none() {
                // Subject.
                ui.horizontal_wrapped(|ui| {
                    let subject = if p.subject.is_empty() { tl!("(no subject)") } else { p.subject.as_str() };
                    crate::bidi::label(ui, subject, theme::bold(21.0), t.text);
                    if summary.as_ref().is_some_and(|m| m.flags.flagged) {
                        let (ir, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
                        icons::paint(ui.painter(), ir, Icon::Flag, t.flag);
                    }
                });
                if let Some(m) = &summary {
                    let cats = s.categories_of(m);
                    if !cats.is_empty() {
                        ui.horizontal_wrapped(|ui| {
                            for c in cats {
                                let color = theme::category_color(c.color, t.dark);
                                egui::Frame::new().fill(color.gamma_multiply(0.18)).corner_radius(10).inner_margin(Margin::symmetric(8, 2)).show(
                                    ui,
                                    |ui| {
                                        ui.label(RichText::new(&c.name).color(color).small());
                                    },
                                );
                            }
                        });
                    }
                }
                ui.add_space(10.0);

                // Conversation.
                if r.thread.len() > 1 && !in_window {
                    thread_list(s, &t, ui, &r.thread, &r.key, acts);
                    ui.add_space(10.0);
                }

                // Sender card.
                ui.horizontal_top(|ui| {
                    let (ar, _) = ui.allocate_exact_size(vec2(44.0, 44.0), Sense::hover());
                    widgets::avatar(ui, ar.center(), 21.0, &p.from.initials(), theme::avatar_color(&p.from.email, t.dark));
                    ui.add_space(4.0);
                    // The date to the right, with room kept for it (the address ran under it in a
                    // narrow pane); under the sender when the pane is too narrow for both.
                    let when = summary.as_ref().map_or(p.date, |m| if p.date > 0 { p.date } else { m.date });
                    let date = egui::WidgetText::from(RichText::new(crate::i18n::date(&dates::full_time(when, &s.clock))).color(t.dim).small())
                        .into_galley(ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, egui::TextStyle::Small);
                    let rest = ui.available_width() - date.size().x - 16.0;
                    let beside = rest >= 200.0;
                    let room = if beside { rest } else { ui.available_width() };
                    ui.allocate_ui_with_layout(vec2(room, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                        ui.set_max_width(room);
                        ui.horizontal_wrapped(|ui| {
                            let shown = crate::bidi::shown(p.from.display()).0;
                            let name = person_link(ui, &t, RichText::new(shown).font(theme::bold(15.0)).color(t.text));
                            person_menus(s, &t, &name, &p.from, acts);
                            if !p.from.name.is_empty() {
                                ui.label(RichText::new(format!("<{}>", p.from.email)).color(t.dim));
                            }
                            let known = known_contact(s, &p.from.email).is_some();
                            if !known && !p.from.email.is_empty() && widgets::tiny_icon_button(ui, Icon::Person, "Add to contacts", &t).clicked() {
                                acts.push(Action::Dialog(crate::Dialog::contact_from(&p.from)));
                            }
                        });
                        for (label, list) in [("To:", &p.to), ("Cc:", &p.cc)] {
                            if list.is_empty() {
                                continue;
                            }
                            ui.horizontal_wrapped(|ui| {
                                ui.spacing_mut().item_spacing.x = 0.0;
                                ui.label(RichText::new(format!("{} ", tl!(label))).color(t.dim).small());
                                for (i, a) in list.iter().take(MAX_PEOPLE_SHOWN).enumerate() {
                                    if i > 0 {
                                        ui.label(RichText::new("; ").color(t.dim).small());
                                    }
                                    let r = person_link(ui, &t, RichText::new(crate::bidi::shown(a.display()).0).color(t.dim).small());
                                    person_menus(s, &t, &r, a, acts);
                                }
                                if list.len() > MAX_PEOPLE_SHOWN {
                                    let more = list.len() - MAX_PEOPLE_SHOWN;
                                    ui.label(RichText::new(tf!(" and {n} more", n = more)).color(t.dim).small())
                                        .on_hover_text(addr::format_list(list));
                                }
                            });
                        }
                        if !beside {
                            ui.label(date.clone());
                        }
                    });
                    if beside {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                            ui.label(date);
                        });
                    }
                });
                ui.add_space(8.0);
                // What Outlook says above a message: its follow-up date, its importance.
                let due = summary.as_ref().and_then(|m| s.follow_up_due(m));
                let flagged = summary.as_ref().is_some_and(|m| m.flags.flagged);
                let importance = summary.as_ref().map_or(0, |m| m.importance);
                if flagged || importance != 0 {
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        if flagged {
                            let today = dates::today(&s.clock);
                            let (ir, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                            icons::paint(ui.painter(), ir, Icon::Flag, t.flag);
                            let text = match due {
                                Some(d) if d < today => tf!("Follow up. Overdue: it was due {date}.", date = crate::i18n::date(&dates::day_label(d))),
                                Some(d) if d == today => tl!("Follow up. Due today.").to_string(),
                                Some(d) => tf!("Follow up. Due {date}.", date = crate::i18n::date(&dates::day_label(d))),
                                None => tl!("Follow up.").to_string(),
                            };
                            let overdue = due.is_some_and(|d| d < today);
                            ui.label(RichText::new(text).color(if overdue { t.danger } else { t.dim }).small());
                            ui.add_space(10.0);
                        }
                        if importance > 0 {
                            ui.label(RichText::new(tl!("This message was sent with high importance.")).color(t.danger).small());
                        } else if importance < 0 {
                            ui.label(RichText::new(tl!("This message was sent with low importance.")).color(t.dim).small());
                        }
                    });
                    ui.add_space(6.0);
                }
                if let Some((_, item)) = s.outbox_entry(&r.key) {
                    egui::Frame::new().fill(t.warn.gamma_multiply(0.12)).corner_radius(6).inner_margin(Margin::symmetric(10, 6)).show(ui, |ui| {
                        let why = match item.error.as_deref() {
                            Some(e) if e != "offline" => tf!("Waiting in the Outbox. Last try: {error}", error = widgets::shorten(e, 140)),
                            _ if item.not_before > s.clock.now => {
                                tf!(
                                    "Held in the Outbox: it will be sent {when}.",
                                    when = crate::i18n::date(&dates::full_time(item.not_before, &s.clock))
                                )
                            }
                            _ => tl!("Waiting in the Outbox: it will be sent when you're back online.").to_string(),
                        };
                        ui.label(RichText::new(why).color(t.text));
                    });
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        if widgets::pill_button(ui, Icon::Send, "Send now", &t).clicked() {
                            acts.push(Action::Run("outbox.send", json!({"key": r.key.to_id()})));
                        }
                        if widgets::pill_button(ui, Icon::Drafts, "Edit", &t).clicked() {
                            acts.push(Action::Run("outbox.edit", json!({"key": r.key.to_id()})));
                        }
                        if widgets::pill_button(ui, Icon::Delete, "Delete", &t).clicked() {
                            acts.push(Action::Run("mail.delete", json!({})));
                        }
                    });
                } else if !in_window {
                    // Wrapped: a narrow pane cut off Save and More.
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                        if widgets::pill_button(ui, Icon::Reply, "Reply", &t).clicked() {
                            acts.push(Action::Run("mail.reply", json!({})));
                        }
                        if widgets::pill_button(ui, Icon::ReplyAll, "Reply all", &t).clicked() {
                            acts.push(Action::Run("mail.replyAll", json!({})));
                        }
                        if widgets::pill_button(ui, Icon::Forward, "Forward", &t).clicked() {
                            acts.push(Action::Run("mail.forward", json!({})));
                        }
                        ui.add_space(6.0);
                        if widgets::pill_button(ui, Icon::Print, "Print", &t).on_hover_text(tl!("Open as a PDF to print")).clicked() {
                            acts.push(Action::Print);
                        }
                        let more = widgets::pill_button(ui, Icon::File, "Save", &t);
                        egui::Popup::menu(&more).show(|ui| {
                            if ui.button(tl!("Save as PDF…")).clicked() {
                                acts.push(Action::ExportPdf);
                                ui.close();
                            }
                            if ui.button(tl!("Save as email (.eml)…")).clicked() {
                                acts.push(Action::SaveMessage);
                                ui.close();
                            }
                        });
                        // More, as Outlook's ⋯ beside Forward.
                        let more = widgets::more_pill(ui, "More actions", &t);
                        egui::Popup::menu(&more).show(|ui| {
                            if ui.button(tl!("Reply with meeting")).clicked() {
                                acts.push(Action::Dialog(crate::Dialog::meeting_from(s, p)));
                                ui.close();
                            }
                            if ui.button(tl!("Forward as attachment")).clicked() {
                                acts.push(Action::Run("mail.forwardAsAttachment", json!({})));
                                ui.close();
                            }
                        });
                    });
                }
                ui.add_space(12.0);
                if !in_window {
                    invite_card(s, st, ui, &r.key, acts);
                }
            }

            // Attachments.
            // Pictures the formatted message shows in place aren't listed again as files.
            let formatted = s.prefs.show_html && !p.rich.is_empty();
            let shown: Vec<usize> = if formatted { p.images.iter().filter_map(|i| mime::picture_attachment(p, i)).collect() } else { Vec::new() };
            let in_place = |a: &mime::AttachmentInfo| shown.contains(&a.index);
            // An invitation is shown as its card above, not as a file.
            let invitation = st.invite.as_ref().is_some_and(|(k, _, i)| k == &r.key && i.is_some()) && !in_window;
            let is_invite_file = |a: &mime::AttachmentInfo| {
                invitation && (a.mime.to_ascii_lowercase().starts_with("text/calendar") || a.name.to_ascii_lowercase().ends_with(".ics"))
            };
            let files: Vec<_> = p
                .attachments
                .iter()
                .filter(|a| (!a.inline || a.kind() == AttachmentKind::Image) && !in_place(a) && !is_invite_file(a))
                .cloned()
                .collect();
            if !files.is_empty() {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(if files.len() == 1 { tl!("1 attachment").to_string() } else { tf!("{n} attachments", n = files.len()) })
                            .color(t.dim)
                            .small(),
                    );
                    if files.len() > 1 && ui.link(RichText::new(tl!("Save all…")).small()).clicked() {
                        acts.push(Action::SaveAll);
                    }
                });
                // Wrapped into rows, scrolling past three, as Outlook's are.
                widgets::chip_grid(ui, egui::Id::unique(("attachments", r.key.to_id())), files.len(), 240.0, 400.0, CHIP_H, 3, |ui, i, w| {
                    if let Some(a) = files.get(i) {
                        attachment_card(s, st, ui, &r.key, a, w, acts);
                    }
                });
                ui.add_space(10.0);
            }
            if let Some(index) = viewing
                && let Some(a) = p.attachments.iter().find(|a| a.index == index)
            {
                let sep = ui.available_rect_before_wrap();
                ui.painter().line_segment([sep.left_top(), sep.right_top()], egui::Stroke::new(1.0, t.border));
                ui.add_space(12.0);
                attachment_preview(s, st, ui, &r.key, a, acts);
                return;
            }
            if p.had_html {
                html_bar(s, &t, ui, p, acts);
            }
            if let Some(u) = &p.unsubscribe {
                ui.label(RichText::new(tf!("Mailing list: unsubscribe at {address}", address = widgets::shorten(u, 80))).color(t.faint).small());
                ui.add_space(6.0);
            }
            let sep = ui.available_rect_before_wrap();
            ui.painter().line_segment([sep.left_top(), sep.right_top()], egui::Stroke::new(1.0, t.border));
            ui.add_space(12.0);
            if s.prefs.show_html && !p.rich.is_empty() {
                rich_body(s, st, ui, &r.key, p, acts);
            } else if crate::bidi::has_rtl(&p.text) {
                // Right-to-left text, laid out line by line. egui can't select it (its text layout
                // miscounts right-to-left letters, so a selection copies nothing): right-click
                // copies it as written instead.
                let r = crate::bidi::label(ui, &p.text, theme::regular(14.5), t.text);
                rtl_copy_menu(&r, None, &p.text);
            } else {
                let mut text = p.text.as_str();
                ui.add(
                    egui::TextEdit::multiline(&mut text)
                        .frame(egui::Frame::NONE)
                        .desired_width(f32::INFINITY)
                        .font(theme::regular(14.5))
                        .text_color(t.text),
                );
            }
        });
    });
}

/// A meeting invitation (or an answer to one, or a cancellation) carried by the message.
fn invite_card(s: &Session, st: &mut UiState, ui: &mut Ui, key: &MessageKey, acts: &mut Vec<Action>) {
    let fresh = st.invite.as_ref().is_some_and(|(k, rev, _)| k == key && *rev == s.pim_revision);
    if !fresh {
        st.invite = Some((key.clone(), s.pim_revision, s.invite()));
    }
    let Some(inv) = st.invite.as_ref().and_then(|(_, _, i)| i.clone()) else { return };
    let t = st.tokens;
    let e = &inv.event;
    egui::Frame::new().fill(t.accent_soft).corner_radius(8).inner_margin(Margin::same(12)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::hover());
            icons::paint(ui.painter(), r, Icon::Calendar, t.accent);
            ui.vertical(|ui| {
                ui.label(RichText::new(&e.summary).font(theme::bold(15.0)).color(t.text));
                let when = if e.all_day {
                    crate::i18n::date(&dates::full_time(e.start - i64::from(s.clock.offset), &s.clock))
                        .split(' ')
                        .take(2)
                        .collect::<Vec<_>>()
                        .join(" ")
                } else {
                    format!("{} – {}", crate::i18n::date(&dates::full_time(e.start, &s.clock)), crate::calendar::hm(e.end, s))
                };
                ui.label(RichText::new(when).color(t.dim));
                if !e.location.is_empty() {
                    ui.label(RichText::new(&e.location).color(t.dim));
                }
            });
        });
        ui.add_space(6.0);
        match inv.method.as_str() {
            "REQUEST" => {
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
                    if inv.in_calendar && ui.link(tl!("Show in calendar")).clicked() {
                        show_day(e.start, s, acts);
                    }
                });
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
                        if inv.in_calendar && widgets::pill_button(ui, Icon::Calendar, "Record the answer", &t).clicked() {
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
    let c = dates::civil(t, &s.clock);
    acts.push(Action::Run("view.module", json!({"name": "calendar"})));
    acts.push(Action::Run("calendar.view", json!({"mode": "day"})));
    acts.push(Action::Run("calendar.goto", json!({"date": format!("{:04}-{:02}-{:02}", c.year, c.month, c.day)})));
}

fn empty(ui: &mut Ui, t: &Tokens, msg: &str) {
    let msg = tl!(msg);
    ui.centered_and_justified(|ui| {
        ui.vertical_centered(|ui| {
            ui.add_space(ui.available_height() * 0.35);
            let (r, _) = ui.allocate_exact_size(vec2(56.0, 56.0), Sense::hover());
            icons::paint(ui.painter(), r, Icon::Read, t.border);
            ui.add_space(8.0);
            ui.label(RichText::new(msg).color(t.faint));
        });
    });
}

fn thread_list(s: &Session, t: &Tokens, ui: &mut Ui, thread: &[MessageKey], open: &MessageKey, acts: &mut Vec<Action>) {
    egui::Frame::new().fill(t.chrome).stroke(egui::Stroke::new(1.0, t.border)).corner_radius(8).inner_margin(Margin::symmetric(10, 8)).show(
        ui,
        |ui| {
            ui.label(RichText::new(tf!("{n} messages in this conversation", n = thread.len())).color(t.dim).small());
            for k in thread {
                let Some(m) = s.summary(k) else { continue };
                let current = k == open;
                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click());
                if current || resp.hovered() {
                    ui.painter().rect_filled(rect, 4.0, if current { t.accent_soft } else { t.hover_row });
                }
                let font = if !m.flags.seen { theme::bold(13.0) } else { theme::regular(13.0) };
                let p = ui.painter_at(rect);
                p.text(rect.left_center() + vec2(8.0, 0.0), egui::Align2::LEFT_CENTER, m.from.display(), font, t.text);
                p.text(
                    rect.left_center() + vec2(170.0, 0.0),
                    egui::Align2::LEFT_CENTER,
                    widgets::shorten(&m.preview, 70),
                    theme::regular(12.5),
                    t.dim,
                );
                p.text(
                    rect.right_center() - vec2(8.0, 0.0),
                    egui::Align2::RIGHT_CENTER,
                    crate::i18n::date(&dates::list_time(m.date, &s.clock)),
                    theme::regular(12.0),
                    t.dim,
                );
                if resp.clicked() && !current {
                    acts.push(Action::Open(k.clone()));
                }
            }
        },
    );
}
