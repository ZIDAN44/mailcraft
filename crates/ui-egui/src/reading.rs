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
    message(s, st, ui, acts, false);
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
        st.attachment_view = None;
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
                ui.horizontal(|ui| {
                    let (ar, _) = ui.allocate_exact_size(vec2(44.0, 44.0), Sense::hover());
                    widgets::avatar(ui, ar.center(), 21.0, &p.from.initials(), theme::avatar_color(&p.from.email, t.dark));
                    ui.add_space(4.0);
                    ui.vertical(|ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(RichText::new(crate::bidi::shown(p.from.display()).0).font(theme::bold(15.0)).color(t.text));
                            if !p.from.name.is_empty() {
                                ui.label(RichText::new(format!("<{}>", p.from.email)).color(t.dim));
                            }
                            let known =
                                s.people(&p.from.email).iter().any(|x| x.card.emails.iter().any(|e| e.value.eq_ignore_ascii_case(&p.from.email)));
                            if !known && !p.from.email.is_empty() && widgets::tiny_icon_button(ui, Icon::Person, "Add to contacts", &t).clicked() {
                                acts.push(Action::Run("contact.fromMessage", json!({})));
                            }
                        });
                        if !p.to.is_empty() {
                            ui.label(RichText::new(tf!("To: {names}", names = addr::display_list(&p.to))).color(t.dim).small())
                                .on_hover_text(addr::format_list(&p.to));
                        }
                        if !p.cc.is_empty() {
                            ui.label(RichText::new(tf!("Cc: {names}", names = addr::display_list(&p.cc))).color(t.dim).small())
                                .on_hover_text(addr::format_list(&p.cc));
                        }
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                        let when = summary.as_ref().map_or(p.date, |m| if p.date > 0 { p.date } else { m.date });
                        ui.label(RichText::new(crate::i18n::date(&dates::full_time(when, &s.clock))).color(t.dim).small());
                    });
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
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
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
                widgets::chip_grid(ui, egui::Id::new(("attachments", r.key.to_id())), files.len(), 240.0, 400.0, CHIP_H, 3, |ui, i, w| {
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

fn kind_icon(k: AttachmentKind) -> Icon {
    match k {
        AttachmentKind::Image => Icon::Image,
        _ => Icon::File,
    }
}

/// Decode an image attachment into a thumbnail texture (cached; `None` if it isn't a picture
/// we can read). Pictures come from mail, so size is capped before decoding.
fn preview(s: &Session, st: &mut UiState, ctx: &egui::Context, key: &MessageKey, index: usize) -> Option<egui::TextureHandle> {
    if let Some(t) = st.previews.get(&(key.clone(), index)) {
        return t.clone();
    }
    if st.previews.len() > 64 {
        st.previews.clear();
    }
    let tex = (|| {
        let raw = s.store.as_ref()?.read_body(key)?;
        let data = mime::attachment_data(&raw, index)?;
        decode(ctx, &format!("att-{}-{index}", key.to_id()), &data, 480)
    })();
    st.previews.insert((key.clone(), index), tex.clone());
    tex
}

/// A picture attachment at a size to look at (decoded once and kept by egui while it's shown).
pub(crate) fn big_picture(s: &Session, ctx: &egui::Context, key: &MessageKey, index: usize) -> Option<egui::TextureHandle> {
    let id = egui::Id::new(("big-picture", key.to_id(), index));
    if let Some(t) = ctx.data(|d| d.get_temp::<Option<egui::TextureHandle>>(id)) {
        return t;
    }
    let tex = (|| {
        let raw = s.store.as_ref()?.read_body(key)?;
        let data = mime::attachment_data(&raw, index)?;
        decode(ctx, &format!("big-{}-{index}", key.to_id()), &data, 2048)
    })();
    ctx.data_mut(|d| d.insert_temp(id, tex.clone()));
    tex
}

/// The first page of a PDF attachment (cached like pictures).
fn pdf_preview(s: &Session, st: &mut UiState, ctx: &egui::Context, key: &MessageKey, index: usize) -> Option<egui::TextureHandle> {
    if let Some(t) = st.previews.get(&(key.clone(), index)) {
        return t.clone();
    }
    if st.previews.len() > 64 {
        st.previews.clear();
    }
    let tex = (|| {
        let raw = s.store.as_ref()?.read_body(key)?;
        let data = mime::attachment_data(&raw, index)?;
        let img = crate::thumbs::pdf_first_page(&data, 260, st.fonts.clone())?;
        Some(ctx.load_texture(format!("pdf-{}-{index}", key.to_id()), img, egui::TextureOptions::LINEAR))
    })();
    st.previews.insert((key.clone(), index), tex.clone());
    tex
}

/// A frame of a video attachment, asked of the platform (which may take a moment).
fn video_preview(s: &Session, st: &mut UiState, ctx: &egui::Context, key: &MessageKey, a: &mime::AttachmentInfo) -> Option<egui::TextureHandle> {
    let id = (key.clone(), a.index);
    if let Some(t) = st.previews.get(&id) {
        return t.clone();
    }
    // The platform's decoder reads files, and the browser has none.
    if !st.has_files {
        return None;
    }
    let path = match st.video_files.get(&id) {
        Some(p) => p.clone(),
        None => {
            // The platform's decoder reads files: put the attachment in one.
            let written = (|| {
                let raw = s.store.as_ref()?.read_body(key)?;
                let data = mime::attachment_data(&raw, a.index)?;
                let dir = std::env::temp_dir().join("mailcraft-thumbs");
                std::fs::create_dir_all(&dir).ok()?;
                let name = format!("{:016x}-{}", mailcraft_engine::store::fnv64(&key.to_id()), mime::sanitize_filename(&a.name));
                let path = dir.join(name);
                std::fs::write(&path, &data).ok()?;
                Some(path)
            })();
            let Some(p) = written else {
                st.previews.insert(id, None);
                return None;
            };
            st.video_files.insert(id.clone(), p.clone());
            p
        }
    };
    let frame = st.video_frames.entry(path).or_insert(crate::VideoFrame::Pending).clone();
    match frame {
        crate::VideoFrame::Pending => {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
            None
        }
        crate::VideoFrame::Ready(png) => {
            let tex = decode(ctx, &format!("video-{}-{}", key.to_id(), a.index), &png, 480);
            st.previews.insert(id, tex.clone());
            tex
        }
        crate::VideoFrame::Unavailable => {
            st.previews.insert(id, None);
            None
        }
    }
}

/// Decode a picture from mail or the web into a texture, at most `max` pixels on a side.
/// Pictures are hostile input: size and allocation are capped before decoding.
pub(crate) fn decode(ctx: &egui::Context, name: &str, data: &[u8], max: u32) -> Option<egui::TextureHandle> {
    if data.len() > 30 << 20 {
        return None;
    }
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(12_000);
    limits.max_image_height = Some(12_000);
    limits.max_alloc = Some(256 << 20);
    let mut reader = image::ImageReader::new(std::io::Cursor::new(data)).with_guessed_format().ok()?;
    reader.limits(limits);
    let decoded = reader.decode().ok()?;
    // Shrink large pictures; never enlarge small ones (the painter scales them crisply).
    let img = if decoded.width() > max || decoded.height() > max { decoded.thumbnail(max, max) } else { decoded }.to_rgba8();
    let size = [img.width() as usize, img.height() as usize];
    let ci = egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw());
    // Pixel art and tiny swatches stay crisp when scaled up.
    let filter = if size[0].max(size[1]) < 96 { egui::TextureOptions::NEAREST } else { egui::TextureOptions::LINEAR };
    Some(ctx.load_texture(name, ci, filter))
}

/// Above an HTML message: whether its web pictures are hidden, and a switch to plain text.
fn html_bar(s: &Session, t: &Tokens, ui: &mut Ui, p: &mime::Parsed, acts: &mut Vec<Action>) {
    let web = s.remote_pictures();
    let formatted = s.prefs.show_html && !p.rich.is_empty();
    if !web.is_empty() && !s.pictures_allowed() && formatted {
        egui::Frame::new().fill(t.chrome).stroke(egui::Stroke::new(1.0, t.border)).corner_radius(6).inner_margin(Margin::symmetric(10, 6)).show(
            ui,
            |ui| {
                ui.horizontal_wrapped(|ui| {
                    let (ir, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
                    icons::paint(ui.painter(), ir, Icon::Image, t.dim);
                    ui.label(RichText::new(tl!("Pictures from the web are hidden to protect your privacy.")).color(t.dim));
                    if ui.link(tl!("Show pictures")).clicked() {
                        acts.push(Action::Run("message.showPictures", json!({})));
                    }
                    let who = p.from.email.clone();
                    if !who.is_empty() && ui.link(tf!("Always show from {sender}", sender = who)).clicked() {
                        acts.push(Action::Run("message.showPictures", json!({"always": true})));
                    }
                });
            },
        );
        ui.add_space(6.0);
    }
    ui.horizontal(|ui| {
        if formatted {
            if ui.link(RichText::new(tl!("Show as plain text")).small()).clicked() {
                acts.push(Action::Run("view.html", json!({"on": false})));
            }
            if !web.is_empty() && s.pictures_allowed() && s.prefs.trusted_senders.contains(&p.from.email.to_ascii_lowercase()) {
                ui.label(RichText::new("·").color(t.faint).small());
                if ui.link(RichText::new(tl!("Stop showing pictures from this sender")).small()).clicked() {
                    acts.push(Action::Run("message.blockPictures", json!({})));
                }
            }
        } else if !p.rich.is_empty() && ui.link(RichText::new(tl!("Show formatted")).small()).clicked() {
            acts.push(Action::Run("view.html", json!({"on": true})));
        } else if p.rich.is_empty() {
            ui.label(RichText::new(tl!("Shown as text.")).color(t.faint).small());
        }
    });
    ui.add_space(6.0);
}

/// An HTML message, formatted: emphasis, code, links (opened in the browser on click) and
/// pictures: those inside the message always, those on the web only once allowed. Mail laid out
/// with tables keeps its columns ([`mime::layout`]).
fn rich_body(s: &Session, st: &mut UiState, ui: &mut Ui, key: &MessageKey, p: &mime::Parsed, acts: &mut Vec<Action>) {
    let t = st.tokens;
    let allowed = s.pictures_allowed();
    let mut shown: Vec<String> = Vec::new();
    let width = ui.available_width();
    let mut body = Body { s, key, p, allowed, shown: &mut shown, acts };
    if p.layout.is_empty() {
        body.lines(st, ui, &p.rich, width, theme::surface_rgb(&t), Align::Min);
    } else {
        body.blocks(st, ui, &p.layout, width, theme::surface_rgb(&t), mime::layout::Align::Left, 0);
    }
    // Pictures without a description have no place in the text: show them after it.
    if allowed {
        let rest: Vec<&String> = p.images.iter().filter(|u| !shown.contains(u)).collect();
        if !rest.is_empty() {
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                for src in rest {
                    picture(s, st, ui, key, p, src, allowed, width);
                }
            });
        }
    }
}

/// What drawing a message body needs.
struct Body<'a> {
    s: &'a Session,
    key: &'a MessageKey,
    p: &'a mime::Parsed,
    allowed: bool,
    /// Pictures drawn in place.
    shown: &'a mut Vec<String>,
    acts: &'a mut Vec<Action>,
}

/// Narrower than this, a table's columns go one under the other.
const MIN_COLUMN: f32 = 140.0;

fn span_font(sp: &mime::Span) -> egui::FontId {
    if sp.heading > 0 {
        theme::bold(match sp.heading {
            1 => 22.0,
            2 => 19.0,
            3 => 17.0,
            _ => 15.5,
        })
    } else if sp.code {
        egui::FontId::monospace(13.5)
    } else if sp.bold {
        theme::bold(14.5)
    } else {
        theme::regular(14.5)
    }
}

impl Body<'_> {
    /// Formatted lines `width` wide, on a background of `page` (for the text's contrast).
    fn lines(&mut self, st: &mut UiState, ui: &mut Ui, lines: &mime::RichText, width: f32, page: [u8; 3], align: Align) {
        let t = st.tokens;
        for line in lines {
            if line.is_empty() {
                ui.add_space(9.0);
                continue;
            }
            // A line with right-to-left text is laid out whole, in order (its links show, but as text).
            if line.iter().any(|sp| crate::bidi::has_rtl(&sp.text)) && line.iter().all(|sp| sp.image.is_none()) {
                let mut job = egui::text::LayoutJob::default();
                for sp in line {
                    let (fg, bg) = theme::mail_colors_on(sp.color, sp.background, page, &t, if sp.link.is_some() { t.accent } else { t.text });
                    let format = egui::TextFormat {
                        font_id: span_font(sp),
                        color: fg,
                        background: bg.unwrap_or(Color32::TRANSPARENT),
                        italics: sp.italic,
                        strikethrough: if sp.strike { egui::Stroke::new(1.0, fg) } else { egui::Stroke::NONE },
                        underline: if sp.link.is_some() { egui::Stroke::new(1.0, fg) } else { egui::Stroke::NONE },
                        ..Default::default()
                    };
                    job.append(&sp.text, 0.0, format);
                }
                job.wrap.max_width = width;
                let job = ui.fonts_mut(|f| crate::bidi::job(f, job));
                let r = ui.add(egui::Label::new(job).selectable(false));
                let written: String = line.iter().map(|sp| sp.text.as_str()).collect();
                rtl_copy_menu(&r, Some(&written), &self.p.text);
                continue;
            }
            // Centred or right-aligned: pictures in the middle (or at the end), and a line that
            // fits moved over (a wrapped line keeps to the left).
            if align != Align::Min {
                if line.iter().all(|sp| sp.image.is_some() || sp.text.trim().is_empty()) {
                    ui.allocate_ui_with_layout(vec2(width, 0.0), egui::Layout::top_down(align), |ui| {
                        ui.set_width(width);
                        for sp in line {
                            self.span(st, ui, sp, width, page);
                        }
                    });
                    continue;
                }
                let line_w: f32 =
                    line.iter().map(|sp| ui.fonts_mut(|f| f.layout_no_wrap(sp.text.clone(), span_font(sp), Color32::WHITE).size().x)).sum();
                if line.iter().all(|sp| sp.image.is_none()) && line_w < width {
                    let space = if align == Align::Center { (width - line_w) / 2.0 } else { width - line_w };
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        ui.add_space(space.max(0.0));
                        for sp in line {
                            self.span(st, ui, sp, width, page);
                        }
                    });
                    continue;
                }
            }
            let layout = egui::Layout::left_to_right(Align::BOTTOM).with_main_wrap(true);
            ui.allocate_ui_with_layout(vec2(width, 0.0), layout, |ui| {
                ui.set_max_width(width);
                ui.spacing_mut().item_spacing.x = 0.0;
                for sp in line {
                    self.span(st, ui, sp, width, page);
                }
            });
        }
    }

    /// One span: text (a link when it has one) or a picture.
    fn span(&mut self, st: &mut UiState, ui: &mut Ui, sp: &mime::Span, width: f32, page: [u8; 3]) {
        let t = st.tokens;
        if let Some(src) = &sp.image {
            self.shown.push(src.clone());
            if !picture(self.s, st, ui, self.key, self.p, src, self.allowed, width) {
                let alt = if sp.text.trim().is_empty() { "picture" } else { sp.text.trim() };
                ui.label(RichText::new(format!("[{alt}]")).color(t.faint));
            }
            return;
        }
        let mut rt = RichText::new(&sp.text).font(span_font(sp));
        if sp.italic {
            rt = rt.italics();
        }
        if sp.strike {
            rt = rt.strikethrough();
        }
        let (fg, bg) = theme::mail_colors_on(sp.color, sp.background, page, &t, if sp.link.is_some() { t.accent } else { t.text });
        if let Some(bg) = bg {
            rt = rt.background_color(bg);
        }
        match &sp.link {
            Some(link) => {
                let r = ui.add(egui::Link::new(rt.color(fg))).on_hover_text(link);
                if r.clicked() {
                    self.acts.push(Action::OpenUrl(link.clone()));
                }
            }
            None => {
                ui.label(rt.color(fg));
            }
        }
    }

    /// Laid-out parts `width` wide.
    #[allow(clippy::too_many_arguments)]
    fn blocks(
        &mut self,
        st: &mut UiState,
        ui: &mut Ui,
        blocks: &[mime::layout::Block],
        width: f32,
        page: [u8; 3],
        align: mime::layout::Align,
        depth: usize,
    ) {
        use mime::layout::{Align as A, Block, Width};
        let main_align = |a: A| match a {
            A::Left => Align::Min,
            A::Center => Align::Center,
            A::Right => Align::Max,
        };
        for b in blocks {
            match b {
                Block::Text(lines) => self.lines(st, ui, lines, width, page, main_align(align)),
                Block::Table(table) if depth < 16 => {
                    let tw = match table.width {
                        Some(Width::Px(px)) => (px as f32).min(width),
                        Some(Width::Percent(pc)) => width * f32::from(pc) / 100.0,
                        None => width,
                    }
                    .max(1.0);
                    let indent = match table.align {
                        A::Center => (width - tw) / 2.0,
                        A::Right => width - tw,
                        A::Left => 0.0,
                    }
                    .max(0.0);
                    let tbg = theme::box_color(table.background, page, &st.tokens);
                    let page = tbg.unwrap_or(page);
                    let behind = table.background_image.as_deref().and_then(|src| self.background(st, ui.ctx(), src));
                    ui.horizontal_top(|ui| {
                        ui.add_space(indent);
                        ui.allocate_ui_with_layout(vec2(tw, 0.0), egui::Layout::top_down(Align::Min), |ui| {
                            ui.set_width(tw);
                            let fill = tbg.map_or(Color32::TRANSPARENT, |c| Color32::from_rgb(c[0], c[1], c[2]));
                            let framed = egui::Frame::new().fill(fill).show(ui, |ui| {
                                // Its place in the drawing: above the fill, under the content.
                                let slot = ui.painter().add(egui::Shape::Noop);
                                ui.spacing_mut().item_spacing = vec2(0.0, 4.0);
                                let columns = column_widths(&table.rows);
                                for row in &table.rows {
                                    self.row(st, ui, row, &columns, tw, page, depth);
                                }
                                slot
                            });
                            if let Some(tex) = &behind {
                                ui.painter().set(framed.inner, cover(tex, framed.response.rect));
                            }
                        });
                    });
                }
                Block::Table(_) => {}
            }
        }
    }

    /// A picture to put behind a box, when it can be shown (one inside the message, or one on the
    /// web once pictures are allowed); counted as shown either way, so it isn't added after the text.
    fn background(&mut self, st: &mut UiState, ctx: &egui::Context, src: &str) -> Option<egui::TextureHandle> {
        if !self.shown.iter().any(|x| x == src) {
            self.shown.push(src.to_string());
        }
        match picture_texture(self.s, st, ctx, self.key, self.p, src, self.allowed) {
            Pic::Ready(tex) => Some(tex),
            Pic::Loading | Pic::Missing => None,
        }
    }

    /// A box placed over `over` (its container's rectangle); where it ends at the bottom.
    fn overlay(&mut self, st: &mut UiState, ui: &mut Ui, o: &mime::layout::Overlay, over: egui::Rect, page: [u8; 3], depth: usize) -> f32 {
        use mime::layout::Width;
        const PAD: f32 = 4.0;
        let x = o.x as f32;
        let w = match o.width {
            Some(Width::Px(px)) => px as f32,
            Some(Width::Percent(pc)) => over.width() * f32::from(pc) / 100.0,
            // As wide as its text (CSS's shrink-to-fit), at most what's left of the box.
            None => natural_width(ui, &o.blocks).map_or(over.width() - x, |n| n + 2.0 * PAD + 1.0).min(over.width() - x).max(24.0),
        }
        .clamp(1.0, over.width().max(1.0));
        let left = if o.from_right { over.right() - x - w } else { over.left() + x }.max(over.left() - w);
        let bg = theme::box_color(o.background, page, &st.tokens);
        let fill = bg.map_or(Color32::TRANSPARENT, |b| Color32::from_rgb(b[0], b[1], b[2]));
        let draw = |this: &mut Self, st: &mut UiState, ui: &mut Ui, top: f32, measure: bool| -> f32 {
            let mut b = egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(egui::pos2(left, top), vec2(w, f32::INFINITY)));
            if measure {
                b = b.sizing_pass().invisible();
            }
            let mut child = ui.new_child(b);
            egui::Frame::new().fill(fill).inner_margin(PAD).show(&mut child, |ui| {
                let inner = (w - 2.0 * PAD).max(1.0);
                ui.set_width(inner);
                this.blocks(st, ui, &o.blocks, inner, bg.unwrap_or(page), mime::layout::Align::Left, depth + 1);
            });
            child.min_rect().height()
        };
        // From the bottom: its height first, out of sight.
        let top = if o.from_bottom {
            let h = draw(self, st, ui, over.top(), true);
            (over.bottom() - o.y as f32 - h).max(over.top())
        } else {
            over.top() + o.y as f32
        };
        let h = draw(self, st, ui, top, false);
        top + h
    }

    /// A table row: its cells side by side, or one under the other when there's no room.
    /// `columns`: the widths the table's columns ask for (a width set in one row is the whole
    /// column's, as HTML sizes columns).
    #[allow(clippy::too_many_arguments)]
    fn row(
        &mut self,
        st: &mut UiState,
        ui: &mut Ui,
        row: &[mime::layout::Cell],
        columns: &[Option<mime::layout::Width>],
        width: f32,
        page: [u8; 3],
        depth: usize,
    ) {
        use mime::layout::Width;
        const PAD: f32 = 6.0;
        let px = |w: Width| match w {
            Width::Px(px) => px as f32,
            Width::Percent(pc) => width * f32::from(pc) / 100.0,
        };
        // Widths: what cells (or their columns) ask for, the rest shared by the others.
        let mut col = 0usize;
        let asked: Vec<Option<f32>> = row
            .iter()
            .map(|c| {
                let span = usize::from(c.span.max(1));
                let from_columns = || {
                    let parts = columns.get(col..col + span)?;
                    (parts.len() == span && parts.iter().all(Option::is_some)).then(|| parts.iter().flatten().map(|w| px(*w)).sum::<f32>())
                };
                let w = c.width.map(px).or_else(from_columns);
                col += span;
                w
            })
            .collect();
        let fixed: f32 = asked.iter().flatten().sum();
        let free = asked.iter().filter(|a| a.is_none()).count();
        let mut widths: Vec<f32> = if fixed > width || (free > 0 && width - fixed < MIN_COLUMN * free as f32) {
            // Asked for more than there is: shared out in proportion.
            let total: f32 = asked.iter().map(|a| a.unwrap_or(MIN_COLUMN)).sum::<f32>().max(1.0);
            asked.iter().map(|a| a.unwrap_or(MIN_COLUMN) / total * width).collect()
        } else {
            let each = if free > 0 { (width - fixed) / free as f32 } else { 0.0 };
            asked.iter().map(|a| a.unwrap_or(each)).collect()
        };
        // Cells that ask for nothing extra (spacers) next to wider ones.
        let used: f32 = widths.iter().sum();
        if used < width
            && let Some(last) = widths.last_mut()
        {
            *last += width - used;
        }
        let stack = row.len() > 1 && widths.iter().zip(row).any(|(w, c)| *w < MIN_COLUMN && !c.blocks.is_empty());
        let cell = |this: &mut Self, st: &mut UiState, ui: &mut Ui, c: &mime::layout::Cell, w: f32| {
            let bg = theme::box_color(c.background, page, &st.tokens);
            let fill = bg.map_or(Color32::TRANSPARENT, |b| Color32::from_rgb(b[0], b[1], b[2]));
            let behind = c.background_image.as_deref().and_then(|src| this.background(st, ui.ctx(), src));
            ui.allocate_ui_with_layout(vec2(w, 0.0), egui::Layout::top_down(Align::Min), |ui| {
                ui.set_width(w);
                let framed = egui::Frame::new().fill(fill).inner_margin(PAD).show(ui, |ui| {
                    let slot = ui.painter().add(egui::Shape::Noop);
                    let inner = (w - 2.0 * PAD).max(1.0);
                    ui.set_width(inner);
                    this.blocks(st, ui, &c.blocks, inner, bg.unwrap_or(page), c.align, depth + 1);
                    slot
                });
                let rect = framed.response.rect;
                if let Some(tex) = &behind {
                    ui.painter().set(framed.inner, cover(tex, rect));
                }
                // Boxes placed over it; the cell grows to hold any that reach past its end.
                let mut bottom = rect.bottom();
                for o in &c.overlays {
                    bottom = bottom.max(this.overlay(st, ui, o, rect, bg.unwrap_or(page), depth));
                }
                if bottom > rect.bottom() {
                    ui.add_space(bottom - rect.bottom());
                }
            });
        };
        if stack {
            for c in row.iter().filter(|c| !c.blocks.is_empty()) {
                cell(self, st, ui, c, width);
            }
        } else {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                for (c, w) in row.iter().zip(&widths) {
                    cell(self, st, ui, c, *w);
                }
            });
        }
    }
}

/// Draw one picture of an HTML message; `false` when it can't be shown (yet).
#[allow(clippy::too_many_arguments)]
fn picture(s: &Session, st: &mut UiState, ui: &mut Ui, key: &MessageKey, p: &mime::Parsed, src: &str, allowed: bool, width: f32) -> bool {
    let tex = match picture_texture(s, st, ui.ctx(), key, p, src, allowed) {
        Pic::Ready(tex) => tex,
        Pic::Loading => {
            ui.spinner();
            return true;
        }
        Pic::Missing => return false,
    };
    let sz = tex.size_vec2();
    // Tracking pixels and spacers: nothing to see.
    if sz.x <= 3.0 && sz.y <= 3.0 {
        return true;
    }
    let scale = (width.min(900.0) / sz.x).min(1.0);
    ui.add(egui::Image::new(&tex).fit_to_exact_size(sz * scale));
    true
}

/// How wide formatted text is unwrapped (its widest line); `None` for anything but text.
fn natural_width(ui: &Ui, blocks: &[mime::layout::Block]) -> Option<f32> {
    let mut widest: f32 = 0.0;
    for b in blocks {
        let mime::layout::Block::Text(lines) = b else { return None };
        for line in lines {
            if line.iter().any(|sp| sp.image.is_some()) {
                return None;
            }
            let w: f32 = line.iter().map(|sp| ui.fonts_mut(|f| f.layout_no_wrap(sp.text.clone(), span_font(sp), Color32::WHITE).size().x)).sum();
            widest = widest.max(w);
        }
    }
    Some(widest)
}

/// Right-click on right-to-left text (which can't be selected): copy the line, or the whole
/// message, as written.
fn rtl_copy_menu(r: &egui::Response, line: Option<&str>, whole: &str) {
    r.interact(Sense::click()).context_menu(|ui| {
        if let Some(line) = line
            && ui.button(tl!("Copy")).clicked()
        {
            ui.ctx().copy_text(line.trim().to_string());
            ui.close();
        }
        if ui.button(tl!("Copy the whole message")).clicked() {
            ui.ctx().copy_text(whole.trim().to_string());
            ui.close();
        }
    });
}

/// A message's picture, as far as it can be had now.
enum Pic {
    Ready(egui::TextureHandle),
    /// Being downloaded.
    Loading,
    /// Not in the message, not allowed yet, or not a picture.
    Missing,
}

/// The width each column of a table asks for: the first cell of one column that sets one, in any
/// row (HTML sizes a whole column so; only the first row set it in a report whose other rows then
/// lined up elsewhere).
fn column_widths(rows: &[Vec<mime::layout::Cell>]) -> Vec<Option<mime::layout::Width>> {
    const MAX_COLUMNS: usize = 64;
    let mut out: Vec<Option<mime::layout::Width>> = Vec::new();
    for row in rows {
        let mut col = 0usize;
        for c in row {
            let span = usize::from(c.span.max(1));
            if span == 1 && col < MAX_COLUMNS {
                if out.len() <= col {
                    out.resize(col + 1, None);
                }
                if let (Some(slot), Some(w)) = (out.get_mut(col), c.width)
                    && slot.is_none()
                {
                    *slot = Some(w);
                }
            }
            col = col.saturating_add(span);
        }
    }
    out
}

/// A picture of an HTML message: one inside it (`cid:`) always, one on the web once allowed.
fn picture_texture(s: &Session, st: &mut UiState, ctx: &egui::Context, key: &MessageKey, p: &mime::Parsed, src: &str, allowed: bool) -> Pic {
    // One sent with the message: by `cid:`, or by the name or address it was sent under.
    let lower = src.to_ascii_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://")) {
        return match mime::picture_attachment(p, src) {
            Some(index) => preview(s, st, ctx, key, index).map_or(Pic::Missing, Pic::Ready),
            None => Pic::Missing,
        };
    }
    if !allowed {
        return Pic::Missing;
    }
    match s.pictures.get(src) {
        Some(mailcraft_engine::Picture::Ready(bytes)) => {
            if let Some(t) = st.web_pictures.get(src) {
                return t.clone().map_or(Pic::Missing, Pic::Ready);
            }
            if st.web_pictures.len() > 200 {
                st.web_pictures.clear();
            }
            let t = decode(ctx, &format!("web-{src}"), bytes, 1200);
            st.web_pictures.insert(src.to_string(), t.clone());
            t.map_or(Pic::Missing, Pic::Ready)
        }
        Some(mailcraft_engine::Picture::Loading) => Pic::Loading,
        _ => Pic::Missing,
    }
}

/// A picture covering `rect`, cropped to keep its shape (CSS's `background-size: cover`).
fn cover(tex: &egui::TextureHandle, rect: egui::Rect) -> egui::Shape {
    let sz = tex.size_vec2();
    if sz.x < 1.0 || sz.y < 1.0 || rect.width() < 1.0 || rect.height() < 1.0 {
        return egui::Shape::Noop;
    }
    let scale = (rect.width() / sz.x).max(rect.height() / sz.y);
    let part = vec2((rect.width() / (sz.x * scale)).min(1.0), (rect.height() / (sz.y * scale)).min(1.0));
    let uv = egui::Rect::from_center_size(egui::pos2(0.5, 0.5), part);
    egui::Shape::image(tex.id(), rect, uv, Color32::WHITE)
}

/// An attachment as Outlook shows one: a small chip with a thumbnail (or the kind's icon), the
/// name and size, and a ▾ menu. Double-click opens it; right-click gives the menu too; dragging it
/// out (once the pointer moves) copies the file to another app. The chip senses the pointer under
/// its button, so a press on ▾ or the chip never starts a drag by itself.
fn attachment_card(s: &Session, st: &mut UiState, ui: &mut Ui, key: &MessageKey, a: &mime::AttachmentInfo, w: f32, acts: &mut Vec<Action>) {
    let t = st.tokens;
    let kind = a.kind();
    let thumb = match kind {
        AttachmentKind::Image => preview(s, st, ui.ctx(), key, a.index),
        AttachmentKind::Pdf => pdf_preview(s, st, ui.ctx(), key, a.index),
        AttachmentKind::Video => video_preview(s, st, ui.ctx(), key, a),
        _ => None,
    };
    // The Craft app for the file, when it's installed.
    let craft = mime::craft_app_for(&a.mime, &a.name).and_then(|(cmd, name)| st.craft_apps.iter().any(|(c, _)| *c == cmd).then_some((cmd, name)));
    let text_preview = mime::text_preview_kind(&a.mime, &a.name).is_some();
    let menu = |ui: &mut Ui, acts: &mut Vec<Action>| {
        if kind == AttachmentKind::Image && ui.button(tl!("Preview")).clicked() {
            acts.push(Action::Dialog(crate::Dialog::PicturePreview { key: key.clone(), index: a.index, name: a.name.clone() }));
            ui.close();
        }
        if text_preview && ui.button(tl!("Preview")).clicked() {
            acts.push(Action::Run("attachment.preview", json!({"index": a.index})));
            ui.close();
        }
        if ui.button(tl!("Open")).clicked() {
            acts.push(Action::OpenAttachment(a.index));
            ui.close();
        }
        if let Some((cmd, name)) = craft
            && ui.button(tf!("Open in {app}", app = name)).clicked()
        {
            acts.push(Action::OpenInCraft(a.index, cmd));
            ui.close();
        }
        ui.separator();
        if ui.button(tl!("Save As…")).clicked() {
            acts.push(Action::SaveAttachment(a.index));
            ui.close();
        }
        if ui.button(tl!("Save All Attachments…")).clicked() {
            acts.push(Action::SaveAll);
            ui.close();
        }
    };
    // The chip: the body (double-click opens, drag copies out) and the ▾ at its right, each lit
    // when the pointer is over it, as Outlook's are. A click selects the chip.
    let (rect, resp) = ui.allocate_exact_size(vec2(w, CHIP_H), Sense::click_and_drag());
    let more_rect = Rect::from_min_max(egui::pos2(rect.right() - 30.0, rect.top()), rect.max).shrink(2.0);
    let more = ui.interact(more_rect, resp.id.with("more"), Sense::click()).on_hover_text(tl!("More options"));
    // egui counts the chip as hovered under the ▾ too (it senses drags); the ▾ wins there.
    let on_more = more.contains_pointer();
    let body_hovered = resp.hovered() && !on_more;
    let sel_id = egui::Id::new(("attachment-selected", key.to_id()));
    if resp.clicked() || resp.secondary_clicked() || more.clicked() {
        ui.ctx().data_mut(|d| d.insert_temp(sel_id, a.index));
    }
    // A click shows it in place of the message's text, as Outlook previews attachments.
    if resp.clicked() && !on_more {
        st.attachment_view = Some((key.clone(), a.index));
    }
    let previewed = st.attachment_view.as_ref().is_some_and(|(k, i)| k == key && *i == a.index);
    let selected = previewed || ui.ctx().data(|d| d.get_temp::<usize>(sel_id)) == Some(a.index);
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        p.rect(
            rect,
            4.0,
            t.surface,
            egui::Stroke::new(1.0, if selected { t.accent } else { t.faint.gamma_multiply(0.55) }),
            egui::StrokeKind::Inside,
        );
        let body = Rect::from_min_max(rect.min, egui::pos2(more_rect.left() - 2.0, rect.bottom())).shrink(2.0);
        if selected || body_hovered {
            p.rect_filled(body, 3.0, if selected { t.accent_soft } else { t.hover_row });
        }
        let menu_open = egui::Popup::is_id_open(ui.ctx(), more.id.with("menu"));
        if on_more || menu_open {
            p.rect_filled(more_rect, 3.0, t.hover_row);
        }
        icons::paint(p, icons::centered(more_rect, 13.0), Icon::ChevronDown, if on_more { t.text } else { t.dim });
        let r = Rect::from_min_size(egui::pos2(rect.left() + 6.0, rect.center().y - 15.0), vec2(30.0, 30.0));
        match &thumb {
            Some(tex) => {
                let sz = tex.size_vec2();
                let scale = (30.0 / sz.x).min(30.0 / sz.y);
                p.rect_filled(r, 3.0, t.chrome);
                let img = Rect::from_center_size(r.center(), sz * scale);
                p.image(tex.id(), img, Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
            }
            None => {
                p.rect_filled(r, 4.0, t.accent_soft);
                icons::paint(p, icons::centered(r, 17.0), kind_icon(kind), t.accent);
            }
        }
        let text_x = r.right() + 8.0;
        let text_w = more_rect.left() - 4.0 - text_x;
        let name = widgets::one_line(ui, &a.name, theme::regular(12.5), t.text, text_w);
        let size = widgets::one_line(ui, &widgets::human_size(a.size), theme::regular(11.5), t.dim, text_w);
        p.galley(egui::pos2(text_x, rect.center().y - name.size().y - 1.0), name, t.text);
        p.galley(egui::pos2(text_x, rect.center().y + 1.0), size, t.dim);
    }
    egui::Popup::menu(&more).id(more.id.with("menu")).show(|ui| menu(ui, acts));
    if resp.double_clicked() && !on_more {
        acts.push(Action::OpenAttachment(a.index));
    }
    resp.context_menu(|ui| menu(ui, acts));
    // Drag it onto the desktop or another app to copy the file there.
    if st.can_drag_out && resp.drag_started() && !on_more {
        acts.push(Action::DragOut(a.index));
    }
    if !on_more {
        resp.on_hover_text(a.name.clone());
    }
}

/// Above a previewed attachment: back to the message, the file's name and size, and Open and
/// Save As (Outlook's Back to message and its Attachments tab).
fn preview_header(t: &Tokens, ui: &mut Ui, a: &mime::AttachmentInfo, acts: &mut Vec<Action>, view: &mut Option<(MessageKey, usize)>) {
    ui.horizontal(|ui| {
        let back = ui.add(egui::Button::new(RichText::new(format!("← {}", tl!("Back to message"))).color(t.accent)).frame_when_inactive(false));
        if back.on_hover_text(tl!("Esc")).clicked() {
            *view = None;
        }
    });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::hover());
        icons::paint(ui.painter(), icons::centered(r, 22.0), kind_icon(a.kind()), t.accent);
        ui.vertical(|ui| {
            ui.add(egui::Label::new(RichText::new(&a.name).font(theme::bold(16.0)).color(t.text)).truncate());
            ui.label(RichText::new(widgets::human_size(a.size)).color(t.dim).small());
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            if widgets::pill_button(ui, Icon::File, "Save As…", t).clicked() {
                acts.push(Action::SaveAttachment(a.index));
            }
            if widgets::pill_button(ui, Icon::PopOut, "Open", t).clicked() {
                acts.push(Action::OpenAttachment(a.index));
            }
        });
    });
    ui.add_space(10.0);
}

/// What an attachment is read as, kept while it's shown.
#[derive(Clone)]
enum Shown {
    Text { header: Vec<(String, String)>, text: String },
    Office(mime::office::Preview),
    Pdf,
    Picture,
    Nothing,
}

/// The attachment, as the reading pane previews it: a picture at the pane's width, a PDF's
/// pages, text, an attached email, a Word document, an Excel workbook or a PowerPoint
/// presentation's slides; other files say there's no preview, with Open and Save As.
fn attachment_preview(s: &Session, st: &mut UiState, ui: &mut Ui, key: &MessageKey, a: &mime::AttachmentInfo, acts: &mut Vec<Action>) {
    let t = st.tokens;
    let id = egui::Id::new(("attachment-shown", key.to_id(), a.index));
    let shown = match ui.ctx().data(|d| d.get_temp::<std::sync::Arc<Shown>>(id)) {
        Some(x) => x,
        None => {
            let data = s.store.as_ref().and_then(|st| st.read_body(key)).and_then(|raw| mime::attachment_data(&raw, a.index));
            let x = std::sync::Arc::new(read_shown(a, data.as_deref()));
            ui.ctx().data_mut(|d| d.insert_temp(id, x.clone()));
            x
        }
    };
    let width = ui.available_width();
    match shown.as_ref() {
        Shown::Picture => match big_picture(s, ui.ctx(), key, a.index) {
            Some(tex) => {
                let sz = tex.size_vec2();
                let scale = (width / sz.x).min(1.0);
                ui.add(egui::Image::new(&tex).fit_to_exact_size(sz * scale));
            }
            None => no_preview(&t, ui, a, acts),
        },
        Shown::Pdf => pdf_pages(s, st, ui, key, a, width, acts),
        Shown::Text { header, text } => {
            if !header.is_empty() {
                egui::Grid::new(("att-header", a.index)).num_columns(2).spacing(vec2(10.0, 4.0)).show(ui, |ui| {
                    for (k, v) in header {
                        ui.label(RichText::new(k).color(t.dim));
                        ui.label(RichText::new(v).color(t.text));
                        ui.end_row();
                    }
                });
                ui.add_space(10.0);
            }
            let mut text = text.as_str();
            ui.add(
                egui::TextEdit::multiline(&mut text)
                    .frame(egui::Frame::NONE)
                    .desired_width(f32::INFINITY)
                    .font(theme::regular(14.0))
                    .text_color(t.text),
            );
        }
        Shown::Office(p) => office_preview(&t, ui, p, a.index),
        Shown::Nothing => no_preview(&t, ui, a, acts),
    }
}

/// Read an attachment once for its preview.
fn read_shown(a: &mime::AttachmentInfo, data: Option<&[u8]>) -> Shown {
    let Some(data) = data else { return Shown::Nothing };
    let kind = a.kind();
    if kind == AttachmentKind::Image {
        return Shown::Picture;
    }
    if kind == AttachmentKind::Pdf {
        return Shown::Pdf;
    }
    if let Some(p) = mime::office::preview(&a.name, &a.mime, data) {
        return Shown::Office(p);
    }
    match mime::text_preview_kind(&a.mime, &a.name) {
        Some("message") => match mime::parse(data) {
            Some(m) => {
                let mut header = vec![(tl!("From").to_string(), m.from.to_string())];
                if !m.to.is_empty() {
                    header.push((tl!("To").to_string(), addr::format_list(&m.to)));
                }
                header.push((tl!("Subject").to_string(), m.subject.clone()));
                Shown::Text { header, text: m.text.chars().take(300_000).collect() }
            }
            None => Shown::Nothing,
        },
        Some(_) => {
            Shown::Text { header: Vec::new(), text: String::from_utf8_lossy(data.get(..data.len().min(300_000)).unwrap_or_default()).into_owned() }
        }
        None => Shown::Nothing,
    }
}

fn no_preview(t: &Tokens, ui: &mut Ui, a: &mime::AttachmentInfo, acts: &mut Vec<Action>) {
    ui.add_space(30.0);
    ui.vertical_centered(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(48.0, 48.0), Sense::hover());
        icons::paint(ui.painter(), r, kind_icon(a.kind()), t.faint);
        ui.add_space(8.0);
        ui.label(RichText::new(tl!("There's no preview for this file.")).color(t.dim));
        ui.label(RichText::new(tl!("Open it in its app, or save it.")).color(t.faint).small());
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let w = 180.0;
            ui.add_space(((ui.available_width() - w) / 2.0).max(0.0));
            if widgets::pill_button(ui, Icon::PopOut, "Open", t).clicked() {
                acts.push(Action::OpenAttachment(a.index));
            }
            if widgets::pill_button(ui, Icon::File, "Save As…", t).clicked() {
                acts.push(Action::SaveAttachment(a.index));
            }
        });
    });
}

/// Most pages of a PDF shown in the reading pane (the rest: Open it).
const PDF_PAGES: usize = 60;

/// A PDF's pages one under the other, the pane's width, each drawn as it's first needed (one
/// a frame, so the pane stays responsive).
fn pdf_pages(s: &Session, st: &UiState, ui: &mut Ui, key: &MessageKey, a: &mime::AttachmentInfo, width: f32, acts: &mut Vec<Action>) {
    let t = st.tokens;
    let px = (width * ui.ctx().pixels_per_point()).clamp(200.0, 2400.0) as u32;
    let count_id = egui::Id::new(("pdf-count", key.to_id(), a.index));
    let mut rendered_one = false;
    let mut n = 0usize;
    loop {
        let count = ui.ctx().data(|d| d.get_temp::<usize>(count_id));
        if count.is_some_and(|c| n >= c) || n >= PDF_PAGES {
            break;
        }
        let pid = egui::Id::new(("pdf-page", key.to_id(), a.index, n, px / 100));
        let tex = match ui.ctx().data(|d| d.get_temp::<Option<egui::TextureHandle>>(pid)) {
            Some(t) => t,
            None if rendered_one => {
                ui.ctx().request_repaint();
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(RichText::new(tf!("Page {n}", n = n + 1)).color(t.dim).small());
                });
                break;
            }
            None => {
                rendered_one = true;
                let data = s.store.as_ref().and_then(|st| st.read_body(key)).and_then(|raw| mime::attachment_data(&raw, a.index));
                let page = data.and_then(|d| crate::thumbs::pdf_page(&d, n, crate::thumbs::Fit::Width(px), st.fonts.clone()));
                let tex = page.map(|(img, total)| {
                    ui.ctx().data_mut(|d| d.insert_temp(count_id, total));
                    ui.ctx().load_texture(format!("pdf-{}-{}-{n}", key.to_id(), a.index), img, egui::TextureOptions::LINEAR)
                });
                ui.ctx().data_mut(|d| d.insert_temp(pid, tex.clone()));
                tex
            }
        };
        let Some(tex) = tex else {
            if n == 0 {
                no_preview(&t, ui, a, acts);
            }
            break;
        };
        let sz = tex.size_vec2();
        let scale = width / sz.x;
        let r = ui.add(egui::Image::new(&tex).fit_to_exact_size(sz * scale)).rect;
        ui.painter().rect_stroke(r, 0.0, egui::Stroke::new(1.0, t.border), egui::StrokeKind::Outside);
        ui.add_space(12.0);
        n += 1;
    }
    if let Some(c) = ui.ctx().data(|d| d.get_temp::<usize>(count_id)).filter(|c| *c > PDF_PAGES) {
        ui.label(
            RichText::new(tf!("The first {shown} of {total} pages: open it to read the rest.", shown = PDF_PAGES, total = c)).color(t.dim).small(),
        );
    }
}

/// A Word document, workbook or presentation, as its preview reads it.
fn office_preview(t: &Tokens, ui: &mut Ui, p: &mime::office::Preview, index: usize) {
    use mime::office::{Block, Preview};
    match p {
        Preview::Document(blocks) => {
            if blocks.is_empty() {
                ui.label(RichText::new(tl!("The document is empty.")).color(t.dim));
            }
            for (k, b) in blocks.iter().enumerate() {
                match b {
                    Block::Paragraph { runs, heading, list } => {
                        let size = if *heading { 18.0 } else { 14.5 };
                        let mut job = egui::text::LayoutJob::default();
                        job.wrap.max_width = ui.available_width();
                        if *list {
                            job.append("•  ", 0.0, egui::TextFormat { font_id: theme::regular(size), color: t.text, ..Default::default() });
                        }
                        for r in runs {
                            let font = if r.bold || *heading { theme::bold(size) } else { theme::regular(size) };
                            job.append(&r.text, 0.0, egui::TextFormat { font_id: font, color: t.text, italics: r.italic, ..Default::default() });
                        }
                        if runs.is_empty() {
                            ui.add_space(8.0);
                        } else {
                            ui.label(job);
                            ui.add_space(4.0);
                        }
                    }
                    Block::Table(rows) => {
                        table(t, ui, egui::Id::new(("doc-table", index, k)), rows, false);
                        ui.add_space(8.0);
                    }
                }
            }
        }
        Preview::Workbook(sheets) => {
            let sel_id = egui::Id::new(("sheet-tab", index));
            let mut sel = ui.ctx().data(|d| d.get_temp::<usize>(sel_id)).unwrap_or(0).min(sheets.len().saturating_sub(1));
            if sheets.len() > 1 {
                ui.horizontal_wrapped(|ui| {
                    for (i, (name, _)) in sheets.iter().enumerate() {
                        if ui.selectable_label(i == sel, name).clicked() {
                            sel = i;
                        }
                    }
                });
                ui.ctx().data_mut(|d| d.insert_temp(sel_id, sel));
                ui.add_space(6.0);
            }
            match sheets.get(sel) {
                Some((_, rows)) if !rows.is_empty() => {
                    egui::ScrollArea::horizontal()
                        .id_salt(("sheet", index, sel))
                        .show(ui, |ui| table(t, ui, egui::Id::new(("sheet-grid", index, sel)), rows, true));
                }
                _ => {
                    ui.label(RichText::new(tl!("The sheet is empty.")).color(t.dim));
                }
            }
        }
        Preview::Slides(slides) => {
            let w = ui.available_width().min(760.0);
            for (k, paras) in slides.iter().enumerate() {
                ui.label(RichText::new(tf!("Slide {n}", n = k + 1)).color(t.faint).small());
                egui::Frame::new().fill(t.surface).stroke(egui::Stroke::new(1.0, t.border)).corner_radius(6).inner_margin(Margin::same(18)).show(
                    ui,
                    |ui| {
                        ui.set_width(w - 38.0);
                        ui.set_min_height((w - 38.0) * 9.0 / 16.0 * 0.6);
                        for (i, para) in paras.iter().enumerate() {
                            let font = if i == 0 { theme::bold(18.0) } else { theme::regular(14.5) };
                            ui.label(RichText::new(para).font(font).color(t.text));
                            ui.add_space(if i == 0 { 8.0 } else { 3.0 });
                        }
                    },
                );
                ui.add_space(12.0);
            }
        }
    }
}

/// Rows of cells as a ruled grid; a workbook's gets column letters and row numbers.
fn table(t: &Tokens, ui: &mut Ui, id: egui::Id, rows: &[Vec<String>], sheet: bool) {
    let cols = rows.iter().map(Vec::len).max().unwrap_or(0);
    egui::Frame::new().stroke(egui::Stroke::new(1.0, t.border)).show(ui, |ui| {
        egui::Grid::new(id).striped(true).spacing(vec2(14.0, 6.0)).show(ui, |ui| {
            if sheet {
                ui.label("");
                for c in 0..cols {
                    ui.label(RichText::new(column_name(c)).color(t.faint).small());
                }
                ui.end_row();
            }
            for (r, row) in rows.iter().enumerate() {
                if sheet {
                    ui.label(RichText::new((r + 1).to_string()).color(t.faint).small());
                }
                for c in 0..cols {
                    let cell = row.get(c).map_or("", String::as_str);
                    ui.label(RichText::new(widgets::shorten(cell, 120)).color(t.text));
                }
                ui.end_row();
            }
        });
    });
}

/// A spreadsheet's column name: 0 → A, 26 → AA.
fn column_name(mut c: usize) -> String {
    let mut out = Vec::new();
    loop {
        out.push(char::from(b'A' + u8::try_from(c % 26).unwrap_or(0)));
        if c < 26 {
            break;
        }
        c = c / 26 - 1;
    }
    out.iter().rev().collect()
}

/// Height of an attachment chip.
const CHIP_H: f32 = 42.0;

/// When to send a held-back message: in an hour, tomorrow or Monday morning, a time of one's
/// own, or at once again.
fn delay_menu(ui: &mut Ui, t: &Tokens, c: &mut mailcraft_engine::Compose, clock: &dates::Clock) {
    let today = dates::today(clock);
    let at = |day: i64, minutes: i64| day.saturating_mul(86_400) - i64::from(clock.offset) + minutes * 60;
    let weekday = (today + 3).rem_euclid(7);
    let presets = [
        ("In 1 hour", (clock.now + 3600) / 60 * 60),
        ("Tomorrow morning", at(today + 1, 8 * 60)),
        ("Monday morning", at(today + 7 - weekday, 8 * 60)),
    ];
    for (label, when) in presets {
        let clicked = ui
            .horizontal(|ui| {
                let r = ui.selectable_label(c.send_after == Some(when), tl!(label));
                ui.label(RichText::new(crate::i18n::date(&dates::full_time(when, clock))).color(t.faint).small());
                r.clicked()
            })
            .inner;
        if clicked {
            c.send_after = Some(when);
            ui.close();
        }
    }
    ui.separator();
    // A day and time of one's own (local), kept while the menu is open.
    let id = egui::Id::new(("delay-custom", c.id));
    let (mut day, mut time) = ui.data(|d| d.get_temp::<(String, String)>(id)).unwrap_or_else(|| {
        let c0 = dates::civil(c.send_after.unwrap_or(clock.now + 3600), clock);
        (format!("{:04}-{:02}-{:02}", c0.year, c0.month, c0.day), format!("{:02}:{:02}", c0.hour, c0.minute))
    });
    ui.label(RichText::new(tl!("Don't send before")).color(t.dim).small());
    let mut chosen = None;
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(&mut day).desired_width(90.0).hint_text("YYYY-MM-DD"));
        ui.add(egui::TextEdit::singleline(&mut time).desired_width(50.0).hint_text("HH:MM"));
        let parsed = parse_local(&day, &time).map(|(d, m)| at(d, m));
        if ui.add_enabled(parsed.is_some_and(|w| w > clock.now), egui::Button::new(tl!("Set"))).clicked() {
            chosen = parsed;
        }
    });
    ui.data_mut(|d| d.insert_temp(id, (day, time)));
    if let Some(w) = chosen {
        c.send_after = Some(w);
        ui.close();
    }
    if c.send_after.is_some() {
        ui.separator();
        if ui.button(tl!("Send right away")).clicked() {
            c.send_after = None;
            ui.close();
        }
    }
}

/// A local `YYYY-MM-DD` and `HH:MM` as (day, minutes into it).
pub(crate) fn parse_local(day: &str, time: &str) -> Option<(i64, i64)> {
    if !mailcraft_engine::model::job::is_day(day.trim()) {
        return None;
    }
    let d = day.trim();
    let num = |a: usize, z: usize| d.get(a..z).and_then(|x| x.parse::<u32>().ok());
    let days = dates::days_from_civil(i64::from(num(0, 4)?), num(5, 7)?, num(8, 10)?);
    let (h, m) = time.trim().split_once(':')?;
    let (h, m) = (h.trim().parse::<i64>().ok()?, m.trim().parse::<i64>().ok()?);
    ((0..24).contains(&h) && (0..60).contains(&m)).then_some((days, h * 60 + m))
}

/// Width of the labels and label buttons left of the message's fields.
const LABEL_W: f32 = 76.0;
const ROW_H: f32 = 30.0;

/// A field box drawn the way Outlook's are: no frame, a line under it (the accent while typing).
fn underlined(ui: &mut Ui, t: &Tokens, edit: egui::TextEdit<'_>) -> egui::Response {
    let r = ui.add_sized(vec2(ui.available_width(), ROW_H - 4.0), edit.frame(egui::Frame::NONE).vertical_align(egui::Align::Center));
    let y = r.rect.bottom();
    let color = if r.has_focus() { t.accent } else { t.border };
    ui.painter().line_segment([egui::pos2(r.rect.left(), y), egui::pos2(r.rect.right(), y)], egui::Stroke::new(1.0, color));
    r
}

/// A small framed button naming a field (To, Cc…), as in Outlook's message window.
fn label_button(ui: &mut Ui, t: &Tokens, label: &str, enabled: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(LABEL_W - 8.0, ROW_H - 4.0), if enabled { Sense::click() } else { Sense::hover() });
    let fill = if resp.hovered() && enabled { t.hover } else { t.surface };
    ui.painter().rect(rect, 3.0, fill, egui::Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, tl!(label), theme::regular(13.0), if enabled { t.text } else { t.dim });
    ui.add_space(8.0);
    resp
}

/// Outlook's tall Send button beside the address rows.
fn send_button(ui: &mut Ui, t: &Tokens, height: f32, sending: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(66.0, height), if sending { Sense::hover() } else { Sense::click() });
    let fill = if resp.hovered() && !sending { t.hover } else { t.surface };
    ui.painter().rect(rect, 4.0, fill, egui::Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
    let icon = egui::Rect::from_center_size(rect.center() - vec2(0.0, 10.0), vec2(24.0, 24.0));
    icons::paint(ui.painter(), icon, Icon::Send, if sending { t.dim } else { t.accent });
    let label = if sending { tl!("Sending…") } else { tl!("Send") };
    ui.painter().text(rect.center() + vec2(0.0, 16.0), egui::Align2::CENTER_CENTER, label, theme::regular(12.5), t.text);
    resp.on_hover_text(tl!("Send (Ctrl+Enter)"))
}

fn field(ui: &mut Ui, t: &Tokens, label: &str, value: &mut String, hint: &str, check_addresses: bool, focus: bool) {
    ui.horizontal(|ui| {
        ui.add_sized(vec2(LABEL_W, ROW_H), egui::Label::new(RichText::new(tl!(label)).color(t.dim)));
        let r = underlined(ui, t, egui::TextEdit::singleline(value).hint_text(tl!(hint)));
        if focus {
            r.request_focus();
        }
    });
    address_check(ui, t, value, check_addresses);
}

/// A recipient box (To, Cc, Bcc) that suggests people from mail already seen: ↑/↓ choose,
/// Enter or Tab takes the highlighted one, a click takes any.
#[allow(clippy::too_many_arguments)]
fn recipient_field(
    ui: &mut Ui,
    st: &mut UiState,
    name: &'static str,
    label: &str,
    value: &mut String,
    hint: &str,
    focus: bool,
    suggestions: &[Contact],
) -> bool {
    let t = st.tokens;
    let showing = st.suggest_field == Some(name) && !suggestions.is_empty();
    let mut take: Option<usize> = None;
    if showing {
        let n = suggestions.len();
        // Before the text box sees them: arrows move the highlight, Enter/Tab accept.
        ui.input_mut(|i| {
            if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) {
                st.suggest_index = (st.suggest_index + 1) % n;
            }
            if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) {
                st.suggest_index = (st.suggest_index + n - 1) % n;
            }
            if i.consume_key(egui::Modifiers::NONE, egui::Key::Enter) || i.consume_key(egui::Modifiers::NONE, egui::Key::Tab) {
                take = Some(st.suggest_index.min(n - 1));
            }
            if i.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
                st.suggest_dismissed = Some(value.clone());
            }
        });
    }
    // Backspace right after a whole recipient takes all of it, as Outlook does with a resolved
    // name (a half-typed one loses a character as usual).
    let edit_id = ui.id().with(("recipient", name));
    if ui.memory(|m| m.has_focus(edit_id))
        && let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), edit_id)
        && let Some(range) = state.cursor.char_range().filter(|r| r.is_empty())
        && let Some((rest, at)) = mailcraft_engine::model::contacts::remove_recipient(value, range.primary.index.0)
        && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Backspace))
    {
        *value = rest;
        state.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(at))));
        state.store(ui.ctx(), edit_id);
        st.suggest_dismissed = Some(value.clone());
    }
    let (pressed, r) = ui
        .horizontal(|ui| {
            // The field's button opens the address book (the caller's to show).
            let b = label_button(ui, &t, label, true);
            (b.clicked(), underlined(ui, &t, egui::TextEdit::singleline(value).id(edit_id).hint_text(tl!(hint))))
        })
        .inner;
    if focus {
        r.request_focus();
    }
    if r.changed() {
        st.suggest_index = 0;
        st.suggest_dismissed = None;
    }
    if showing && take.is_none() {
        let area = egui::Area::new(ui.id().with(("suggest", name))).order(egui::Order::Foreground).fixed_pos(r.rect.left_bottom() + vec2(0.0, 2.0));
        area.show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).inner_margin(egui::Margin::same(4)).show(ui, |ui| {
                // As wide as the longest suggestion (within the box's width), every row taking
                // a click anywhere on it: the name, and the address dimmer beside it.
                let name_font = theme::regular(13.5);
                let mail_font = theme::regular(12.5);
                let text_w = |text: &str, font: &egui::FontId| ui.fonts_mut(|f| f.layout_no_wrap(text.to_string(), font.clone(), t.text).size().x);
                let widest = suggestions
                    .iter()
                    .map(|c| {
                        let name = if c.address.name.is_empty() { 0.0 } else { text_w(&c.address.name, &name_font) + 12.0 };
                        name + text_w(&c.address.email, &mail_font)
                    })
                    .fold(0.0_f32, f32::max);
                let w = (widest + 24.0).clamp(200.0, r.rect.width().clamp(200.0, 520.0));
                ui.spacing_mut().item_spacing.y = 0.0;
                for (i, c) in suggestions.iter().enumerate() {
                    let on = i == st.suggest_index;
                    let (row, resp) = ui.allocate_exact_size(vec2(w, 28.0), egui::Sense::click());
                    if on || resp.hovered() {
                        ui.painter().rect_filled(row, 4.0, if on { t.accent_soft } else { t.hover_row });
                    }
                    let clip = ui.painter().with_clip_rect(row.shrink2(vec2(8.0, 0.0)));
                    let mut x = row.left() + 10.0;
                    if !c.address.name.is_empty() {
                        let g = clip.layout_no_wrap(c.address.name.clone(), name_font.clone(), t.text);
                        let gw = g.size().x;
                        clip.galley(egui::pos2(x, row.center().y - g.size().y / 2.0), g, t.text);
                        x += gw + 12.0;
                    }
                    let g = clip.layout_no_wrap(c.address.email.clone(), mail_font.clone(), t.dim);
                    clip.galley(egui::pos2(x, row.center().y - g.size().y / 2.0), g, t.dim);
                    // On press: the box loses focus on the same click, before a release would count.
                    if resp.hovered() && ui.input(|i| i.pointer.primary_pressed()) {
                        take = Some(i);
                    }
                }
            });
        });
    }
    if let Some(i) = take
        && let Some(c) = suggestions.get(i)
    {
        *value = mailcraft_engine::model::contacts::complete(value, &c.address);
        st.suggest_index = 0;
        r.request_focus();
        if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), r.id) {
            let end = egui::text::CCursor::new(value.chars().count());
            state.cursor.set_char_range(Some(egui::text::CCursorRange::one(end)));
            state.store(ui.ctx(), r.id);
        }
    }
    if r.has_focus() || take.is_some() {
        st.suggest_field = Some(name);
    } else if st.suggest_field == Some(name) {
        st.suggest_field = None;
    }
    // Half-typed addresses are fine while typing; check once the box is left.
    address_check(ui, &t, value, !r.has_focus());
    pressed
}

fn address_check(ui: &mut Ui, t: &Tokens, value: &str, check_addresses: bool) {
    if check_addresses
        && !value.trim().is_empty()
        && let Err(bad) = addr::parse_list(value)
    {
        ui.horizontal(|ui| {
            ui.add_space(LABEL_W);
            ui.label(RichText::new(tf!("Not an email address: {address}", address = bad)).color(t.danger).small());
        });
    }
}

/// The message being written, in a window of its own.
pub fn compose_window(s: &mut Session, st: &mut UiState, ui: &mut Ui, acts: &mut Vec<Action>) {
    compose(s, st, ui, acts, true);
}

fn compose(s: &mut Session, st: &mut UiState, ui: &mut Ui, acts: &mut Vec<Action>, in_window: bool) {
    let t = st.tokens;
    let accounts: Vec<(String, String)> =
        s.accounts.iter().map(|a| (a.config.id.clone(), EmailAddress::new(a.config.name.clone(), a.config.email.clone()).to_string())).collect();
    // Suggestions for the recipient box being typed in (computed before the form borrows it).
    let suggestions: Vec<Contact> = match (st.suggest_field, s.compose.as_ref()) {
        (Some(f), Some(c)) => {
            let text = match f {
                "to" => c.to.clone(),
                "cc" => c.cc.clone(),
                _ => c.bcc.clone(),
            };
            let token = mailcraft_engine::model::contacts::last_token(&text).to_string();
            if token.chars().count() < 1 || st.suggest_dismissed.as_deref() == Some(text.as_str()) {
                Vec::new()
            } else {
                let present = text.to_lowercase();
                s.suggest_contacts(&token, 8).into_iter().filter(|x| !present.contains(&x.address.email.to_lowercase())).take(6).collect()
            }
        }
        _ => Vec::new(),
    };
    let clock = s.clock;
    // Delay Delivery holds mail in the Outbox, which only a server account's sends from.
    let can_delay = s.compose.as_ref().and_then(|c| s.account(&c.account)).is_some_and(|a| !a.is_local());
    let signature = s.compose.as_ref().and_then(|c| s.account(&c.account)).map(|a| a.config.signature.clone()).unwrap_or_default();
    let Some(c) = s.compose.as_mut() else { return };
    // The editor's state belongs to one message.
    if st.editor.as_ref().is_none_or(|e| e.id != c.id) {
        st.editor = Some(rich_edit::Editor::new(c.id));
    }
    let Some(mut ed) = st.editor.take() else { return };
    // Why it couldn't go: over the message, in its own window when it has one.
    if let Some(msg) = st.compose_alert.clone() {
        let modal = egui::Modal::new(egui::Id::new(("compose-alert", c.id))).frame(crate::dialogs::dialog_frame(&t)).show(ui.ctx(), |ui| {
            ui.set_width(380.0);
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::hover());
                icons::paint(ui.painter(), r, Icon::Warning, t.warn);
                ui.add_space(6.0);
                ui.vertical(|ui| {
                    ui.label(RichText::new(tl!("MailCraft")).font(theme::bold(16.0)).color(t.text));
                    let mut text = if msg == "add at least one recipient" {
                        tl!("Add at least one name in To, Cc or Bcc before sending.").to_string()
                    } else {
                        msg.clone()
                    };
                    if let Some(first) = text.get(..1) {
                        text = first.to_uppercase() + text.get(1..).unwrap_or_default();
                    }
                    if !text.ends_with('.') {
                        text.push('.');
                    }
                    ui.add(egui::Label::new(RichText::new(text).color(t.text)).wrap());
                });
            });
            ui.add_space(10.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.button(tl!("OK")).clicked() || ui.input(|i| i.key_pressed(egui::Key::Enter))
            })
            .inner
        });
        if modal.inner || modal.should_close() {
            st.compose_alert = None;
        }
    }
    // A newly opened message: start in To (new mail) or the body (replies).
    let fresh = st.compose_shown != Some(c.id);
    st.compose_shown = Some(c.id);
    let focus_to = fresh && c.to.trim().is_empty();
    let focus_body = fresh && !focus_to;
    let send = |c: &mailcraft_engine::Compose, acts: &mut Vec<Action>| {
        if c.subject.trim().is_empty() {
            acts.push(Action::Dialog(crate::Dialog::EmptySubject));
        } else {
            acts.push(Action::Run("compose.send", json!({})));
        }
    };
    // Alt+S sends, as in Outlook; in a window of its own the main window's shortcuts don't reach
    // it, so Ctrl+Enter and Ctrl+S are the message's own there.
    if !c.sending {
        let (alt_s, send_key, save_key) = ui.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::ALT, egui::Key::S),
                in_window && i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter),
                in_window && i.consume_key(egui::Modifiers::COMMAND, egui::Key::S),
            )
        });
        if alt_s || send_key {
            send(c, acts);
        }
        if save_key {
            acts.push(Action::Run("compose.saveDraft", json!({})));
        }
    }
    egui::Frame::new().inner_margin(Margin { left: 16, right: 16, top: 8, bottom: 12 }).show(ui, |ui| {
        // The Message ribbon.
        egui::Frame::new().fill(t.chrome).corner_radius(6).inner_margin(Margin::symmetric(8, 4)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            rich_edit::ribbon(ui, &t, &mut ed, c, &signature, acts);
        });
        ui.add_space(4.0);
        // Drafts, Bcc and the format at the left (wrapping onto a second line in a narrow pane);
        // Discard and Pop out (or in) kept at the right, as in Outlook.
        ui.horizontal(|ui| {
            let (icon, label, id) = if in_window { (Icon::PopIn, "Pop in", "compose.popIn") } else { (Icon::PopOut, "Pop out", "compose.popOut") };
            // Where windows can't open (the browser), messages stay in the reading pane.
            let can_pop = !ui.ctx().embed_viewports();
            let width_of = |ui: &Ui, l: &str| ui.painter().layout_no_wrap(tl!(l).to_string(), theme::regular(13.0), t.text).size().x + 36.0;
            let gap = ui.spacing().item_spacing.x;
            let right = width_of(ui, "Discard") + if can_pop { width_of(ui, label) + gap } else { 0.0 } + 2.0 * gap;
            let left_w = (ui.available_width() - right).max(120.0);
            ui.allocate_ui_with_layout(vec2(left_w, 30.0), egui::Layout::left_to_right(egui::Align::Center).with_main_wrap(true), |ui| {
                if widgets::tool_button(ui, Icon::Drafts, "Save draft", &t, !c.sending).on_hover_text(tl!("Save draft (Ctrl+S)")).clicked() {
                    acts.push(Action::Run("compose.saveDraft", json!({})));
                }
                if widgets::tool_button(ui, Icon::Person, if c.show_cc { "Hide Bcc" } else { "Bcc" }, &t, !c.sending).clicked() {
                    c.show_cc = !c.show_cc;
                }
                let mut plain = c.plain;
                if ui.checkbox(&mut plain, RichText::new(tl!("Plain text")).small()).on_hover_text(tl!("Send without formatting")).changed() {
                    c.plain = plain;
                }
                // Importance and a read receipt, as Outlook's Tags and Options.
                ui.add_space(6.0);
                if ui.selectable_label(c.importance > 0, RichText::new("!").strong().color(t.danger)).on_hover_text(tl!("High importance")).clicked()
                {
                    c.importance = if c.importance > 0 { 0 } else { 1 };
                }
                if ui.selectable_label(c.importance < 0, RichText::new("↓").strong().color(t.accent)).on_hover_text(tl!("Low importance")).clicked()
                {
                    c.importance = if c.importance < 0 { 0 } else { -1 };
                }
                let mut receipt = c.read_receipt;
                if ui
                    .checkbox(&mut receipt, RichText::new(tl!("Read receipt")).small())
                    .on_hover_text(tl!("Ask the recipients to confirm they've read it (their mail app may not, or may ask them)"))
                    .changed()
                {
                    c.read_receipt = receipt;
                }
                // Outlook's Delay Delivery and Direct Replies To.
                if can_delay {
                    let label = match c.send_after {
                        Some(at) => tf!("Sends {when}", when = crate::i18n::date(&dates::full_time(at, &clock))),
                        None => tl!("Delay delivery").to_string(),
                    };
                    let r = widgets::tool_toggle(ui, Icon::Clock, &label, &t, c.send_after.is_some())
                        .on_hover_text(tl!("Keep it in the Outbox and send it later (MailCraft has to be running then)"));
                    egui::Popup::menu(&r).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| delay_menu(ui, &t, c, &clock));
                }
                if widgets::tool_toggle(ui, Icon::Reply, "Replies to", &t, c.show_reply_to || !c.reply_to.is_empty())
                    .on_hover_text(tl!("Have replies sent to another address"))
                    .clicked()
                {
                    c.show_reply_to = !c.show_reply_to;
                }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                if can_pop
                    && widgets::tool_button(ui, icon, label, &t, true)
                        .on_hover_text(tl!(if in_window { "Back to the reading pane" } else { "Write it in a window of its own" }))
                        .clicked()
                {
                    acts.push(Action::Run(id, json!({})));
                }
                if widgets::tool_button(ui, Icon::Delete, "Discard", &t, true).clicked() {
                    acts.push(Action::Run("compose.discard", json!({})));
                }
            });
        });
        if let Some(e) = &c.error {
            ui.add_space(4.0);
            egui::Frame::new().fill(t.danger.gamma_multiply(0.12)).corner_radius(6).inner_margin(Margin::symmetric(10, 6)).show(ui, |ui| {
                ui.label(RichText::new(tf!("Not sent: {error}", error = e)).color(t.danger));
            });
        }
        ui.add_space(6.0);
        ui.add_enabled_ui(!c.sending, |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            // Send beside the address rows (From, To, Cc and Bcc when shown).
            let rows = if c.show_cc { 4.0 } else { 3.0 };
            let height = rows * ROW_H + (rows - 1.0) * 6.0;
            ui.horizontal_top(|ui| {
                if send_button(ui, &t, height, c.sending).clicked() {
                    send(c, acts);
                }
                ui.add_space(10.0);
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        let current = accounts.iter().find(|(id, _)| *id == c.account).map(|(_, l)| l.clone()).unwrap_or_default();
                        let many = accounts.len() > 1;
                        let b = label_button(ui, &t, if many { "From ▾" } else { "From" }, many);
                        if many {
                            egui::Popup::menu(&b).show(|ui| {
                                for (id, label) in &accounts {
                                    ui.selectable_value(&mut c.account, id.clone(), label);
                                }
                            });
                        }
                        ui.allocate_ui_with_layout(vec2(ui.available_width(), ROW_H), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            ui.add(egui::Label::new(RichText::new(current).color(t.dim)).truncate());
                        });
                    });
                    // To, Cc and Bcc open the address book, as in Outlook.
                    for (name, label, hint) in
                        [("to", "To", "Names <addresses>, separated by ; or ,"), ("cc", "Cc", ""), ("bcc", "Bcc", "Hidden from the other recipients")]
                    {
                        if name == "bcc" && !c.show_cc {
                            continue;
                        }
                        let value = match name {
                            "to" => &mut c.to,
                            "cc" => &mut c.cc,
                            _ => &mut c.bcc,
                        };
                        if recipient_field(ui, st, name, label, value, hint, name == "to" && focus_to, &suggestions) {
                            let d = crate::Dialog::select_names(c, name);
                            acts.push(Action::Dialog(d));
                        }
                    }
                });
            });
            if c.show_reply_to || !c.reply_to.is_empty() {
                ui.horizontal(|ui| {
                    ui.add_space(76.0);
                    ui.vertical(|ui| field(ui, &t, "Replies to", &mut c.reply_to, "Where replies go instead of to you", true, false));
                });
            }
            ui.horizontal(|ui| {
                ui.add_space(76.0);
                ui.vertical(|ui| field(ui, &t, "Subject", &mut c.subject, "Add a subject", false, false));
            });
            if !c.attachments.is_empty() {
                ui.horizontal(|ui| {
                    ui.add_space(76.0 + LABEL_W);
                    let mut remove = None;
                    ui.vertical(|ui| {
                        // Wrapped into rows, scrolling past two, as Outlook's are.
                        let n = c.attachments.len();
                        widgets::chip_grid(ui, egui::Id::new(("compose-attachments", c.id)), n, 220.0, 320.0, 28.0, 2, |ui, i, w| {
                            let Some(a) = c.attachments.get(i) else { return };
                            let (rect, resp) = ui.allocate_exact_size(vec2(w, 28.0), Sense::hover());
                            let x_rect = Rect::from_center_size(egui::pos2(rect.right() - 15.0, rect.center().y), vec2(22.0, 22.0));
                            let x = ui.interact(x_rect, resp.id.with("remove"), Sense::click()).on_hover_text(tl!("Remove"));
                            let p = ui.painter();
                            p.rect(
                                rect,
                                4.0,
                                if resp.hovered() { t.hover_row } else { t.surface },
                                egui::Stroke::new(1.0, t.faint.gamma_multiply(0.55)),
                                egui::StrokeKind::Inside,
                            );
                            if x.hovered() {
                                p.rect_filled(x_rect, 3.0, t.hover);
                            }
                            icons::paint(p, icons::centered(x_rect, 11.0), Icon::Close, if x.hovered() { t.text } else { t.dim });
                            let ir = Rect::from_center_size(egui::pos2(rect.left() + 15.0, rect.center().y), vec2(14.0, 14.0));
                            icons::paint(p, ir, Icon::Attach, t.dim);
                            let text = format!("{} ({})", a.name, widgets::human_size(a.data.len()));
                            let g = widgets::one_line(ui, &text, theme::regular(12.0), t.text, x_rect.left() - 4.0 - (ir.right() + 6.0));
                            p.galley(egui::pos2(ir.right() + 6.0, rect.center().y - g.size().y / 2.0), g, t.text);
                            resp.on_hover_text(a.name.clone());
                            if x.clicked() {
                                remove = Some(i);
                            }
                        });
                    });
                    if let Some(i) = remove {
                        acts.push(Action::Run("compose.removeAttachment", json!({"index": i})));
                    }
                });
            }
            ui.add_space(4.0);
            let sep = ui.available_rect_before_wrap();
            ui.painter().line_segment([sep.left_top(), sep.right_top()], egui::Stroke::new(1.0, t.border));
            ui.add_space(8.0);
            let body_id = rich_edit::body_id(c);
            // A picture on the clipboard is pasted as a picture.
            if ui.memory(|m| m.has_focus(body_id))
                && ui.input(|i| {
                    i.events.iter().any(|e| matches!(e, egui::Event::Key { key: egui::Key::V, pressed: true, modifiers, .. } if modifiers.command))
                })
            {
                acts.push(Action::PastePicture);
            }
            egui::ScrollArea::vertical().auto_shrink(false).id_salt("compose-body").show(ui, |ui| {
                // Replies start above the quoted text.
                rich_edit::body(ui, &t, &mut ed, c, focus_body.then_some(0), acts);
            });
        });
    });
    st.editor = Some(ed);
}
