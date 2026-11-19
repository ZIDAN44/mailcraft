//! The open message's header: its subject and categories, the conversation, the sender card,
//! the follow-up line, the bars about the sender and the message, and what can be done with it
//! (an Outbox item's, a draft's, or Reply, Forward and the rest).

use super::*;

/// Which of the header is drawn: all of it, scrolling with the text; or, as Outlook keeps it at
/// the top of the pane while the text scrolls (issue #151), the part that stays (the subject,
/// who it's from and to, the bars about it, what can be done with it) or the part that scrolls
/// (the conversation, reactions, an invitation's card: each can be tall).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Part {
    All,
    Pinned,
    Scrolled,
}

/// The header, against the right in a right-to-left interface, its rows from there (the message
/// itself goes the way its own text does).
pub(super) fn header(s: &Session, st: &mut UiState, ui: &mut Ui, o: &Open<'_>, part: Part, acts: &mut Vec<Action>) {
    let t = st.tokens;
    let r = o.r;
    let conversation = |ui: &mut Ui, acts: &mut Vec<Action>| {
        if r.thread.len().max(1) + r.aside.len() > 1 && !o.in_window {
            thread_list(s, &t, ui, &r.thread, &r.aside, &r.key, acts);
            ui.add_space(10.0);
        }
    };
    ui.with_layout(crate::bidi::column(), |ui| {
        if part == Part::Scrolled {
            conversation(ui, acts);
        } else {
            subject_line(s, &t, ui, o, acts);
            ui.add_space(10.0);
            if part == Part::All {
                conversation(ui, acts);
            }
            sender_card(s, &t, ui, o, acts);
            ui.add_space(8.0);
            follow_up_line(s, &t, ui, o.summary);
            if let Some(w) = &r.warning {
                sender_warning(&t, ui, w);
            }
            if let Some(sec) = &r.security {
                security::security_bar(s, &t, ui, sec);
            }
            if let Some(name) = s.opened_file_name(&r.key) {
                opened_file_bar(&t, ui, name);
            }
            action_bar(s, st, ui, o, acts);
            ui.add_space(12.0);
        }
        if part != Part::Pinned {
            reactions::reactions_row(s, &t, ui, &r.key, acts);
            if !o.in_window {
                invite_card(s, st, ui, &r.key, acts);
            }
        }
    });
}

/// The subject (flagged: with a flag), and under it the message's categories.
fn subject_line(s: &Session, t: &Tokens, ui: &mut Ui, o: &Open<'_>, acts: &mut Vec<Action>) {
    // Subject.
    ui.horizontal_wrapped(|ui| {
        let subject = if o.p.subject.is_empty() { tl!("(no subject)") } else { o.p.subject.as_str() };
        subject_heading(ui, subject, t, &subject_marks(s, subject));
        if o.summary.is_some_and(|m| m.flags.flagged) {
            let (ir, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
            icons::paint(ui.painter(), ir, Icon::Flag, t.flag);
        }
    });
    if let Some(m) = o.summary {
        let cats = s.categories_of(m);
        if !cats.is_empty() {
            ui.horizontal_wrapped(|ui| {
                for c in cats {
                    let color = theme::category_color(c.color, t.dark);
                    // One not in the list (another program's tag): outlined, and a
                    // click names it and gives it a colour, keeping its keyword.
                    let listed = s.category_with_keyword(&c.keyword).is_some();
                    let frame = if listed {
                        egui::Frame::new().fill(color.gamma_multiply(0.18))
                    } else {
                        egui::Frame::new().stroke(egui::Stroke::new(1.0, color.gamma_multiply(0.6)))
                    };
                    let chip = frame.corner_radius(10).inner_margin(Margin::symmetric(8, 2)).show(ui, |ui| {
                        ui.label(RichText::new(&c.name).color(color).small());
                    });
                    if !listed {
                        let r = chip
                            .response
                            .interact(Sense::click())
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .on_hover_text(tl!("Not in the category list: click to name it and choose its colour"));
                        if r.clicked() {
                            acts.push(Action::Dialog(crate::Dialog::name_keyword(&c.keyword)));
                        }
                    }
                }
            });
        }
    }
}

/// The sender card: the avatar, who it's from (External, Add to contacts), To and Cc, and the
/// date beside them or under them.
fn sender_card(s: &Session, t: &Tokens, ui: &mut Ui, o: &Open<'_>, acts: &mut Vec<Action>) {
    let p = o.p;
    ui.horizontal_top(|ui| {
        let (ar, _) = ui.allocate_exact_size(vec2(44.0, 44.0), Sense::hover());
        widgets::avatar(ui, ar.center(), 21.0, &p.from.initials(), theme::avatar_color(&p.from.email, t.dark));
        ui.add_space(4.0);
        // The date to the right, with room kept for it (the address ran under it in a
        // narrow pane); under the sender when the pane is too narrow for both.
        let when = o.summary.map_or(p.date, |m| if p.date > 0 { p.date } else { m.date });
        // None at all (a message file without one): no date, rather than 1970's.
        let when = if when > 0 { crate::i18n::date(&dates::full_time(when, &s.clock)) } else { String::new() };
        let date = egui::WidgetText::from(RichText::new(when).color(t.dim).small()).into_galley(
            ui,
            Some(egui::TextWrapMode::Extend),
            f32::INFINITY,
            egui::TextStyle::Small,
        );
        let rest = ui.available_width() - date.size().x - 16.0;
        let beside = rest >= 200.0;
        let room = if beside { rest } else { ui.available_width() };
        ui.allocate_ui_with_layout(vec2(room, 0.0), crate::bidi::column(), |ui| {
            ui.set_max_width(room);
            ui.horizontal_wrapped(|ui| {
                let name = person_link(ui, t, RichText::new(p.from.display()).font(theme::bold(15.0)).color(t.text));
                person_menus(s, t, &name, &p.from, acts);
                if !p.from.name.is_empty() {
                    ui.label(RichText::new(format!("<{}>", p.from.email)).color(t.dim));
                }
                // From outside the organisation (issue #29, with the tag on).
                if s.is_external(&p.from.email) {
                    let _ = external_tag(ui, t);
                }
                let known = known_contact(s, &p.from.email).is_some();
                if !known && !p.from.email.is_empty() && widgets::tiny_icon_button(ui, Icon::Person, "Add to contacts", t).clicked() {
                    acts.push(Action::Dialog(crate::Dialog::contact_from(&p.from)));
                }
            });
            for (label, list) in [(tl!("To:"), &p.to), (tl!("Cc:"), &p.cc)] {
                if list.is_empty() {
                    continue;
                }
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    ui.label(RichText::new(format!("{label} ")).color(t.dim).small());
                    for (i, a) in list.iter().take(MAX_PEOPLE_SHOWN).enumerate() {
                        if i > 0 {
                            ui.label(RichText::new("; ").color(t.dim).small());
                        }
                        let r = person_link(ui, t, RichText::new(a.display()).color(t.dim).small());
                        person_menus(s, t, &r, a, acts);
                    }
                    if list.len() > MAX_PEOPLE_SHOWN {
                        let more = list.len() - MAX_PEOPLE_SHOWN;
                        ui.label(RichText::new(tf!(" and {n} more", n = more)).color(t.dim).small()).on_hover_text(addr::format_list(list));
                    }
                });
            }
            if !beside {
                ui.label(date.clone());
            }
        });
        if beside {
            ui.with_layout(crate::bidi::row_from_end(egui::Align::Min), |ui| {
                ui.label(date);
            });
        }
    });
}

/// What Outlook says above a message: its follow-up date, its importance.
fn follow_up_line(s: &Session, t: &Tokens, ui: &mut Ui, summary: Option<&Summary>) {
    let due = summary.and_then(|m| s.follow_up_due(m));
    let flagged = summary.is_some_and(|m| m.flags.flagged);
    let complete = summary.is_some_and(|m| m.flags.follow_up_complete());
    let importance = summary.map_or(0, |m| m.importance);
    if !(flagged || complete || importance != 0) {
        return;
    }
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        if complete {
            let (ir, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
            icons::paint(ui.painter(), ir, Icon::Done, t.dim);
            let text = match summary.and_then(|m| s.follow_up_done(m)) {
                Some(d) => tf!("Completed on {date}.", date = crate::i18n::date(&dates::day_text(d, &s.clock))),
                None => tl!("Completed.").to_string(),
            };
            ui.label(RichText::new(text).color(t.dim).small());
            ui.add_space(10.0);
        }
        if flagged {
            let today = dates::today(&s.clock);
            let (ir, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
            icons::paint(ui.painter(), ir, Icon::Flag, t.flag);
            let text = match due {
                Some(d) if d < today => {
                    tf!("Follow up. Overdue: it was due {date}.", date = crate::i18n::date(&dates::day_text(d, &s.clock)))
                }
                Some(d) if d == today => tl!("Follow up. Due today.").to_string(),
                Some(d) => tf!("Follow up. Due {date}.", date = crate::i18n::date(&dates::day_text(d, &s.clock))),
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

/// What a message in the Outbox is told to be waiting for. One the server refused for good
/// isn't waiting: it isn't tried again as it is (it read as one whose last try failed).
fn outbox_says(item: &mailcraft_engine::store::OutboxItem, s: &Session) -> String {
    match item.error.as_deref() {
        // MailCraft's own reason whole (it ends in where to go), a server's words shortened.
        Some(e) if item.refused => tf!("Not sent: {error}", error = crate::i18n::fixed_reason(e).unwrap_or_else(|| widgets::shorten(e, 140))),
        Some(e) if crate::i18n::fixed_reason(e).is_some() => tf!("Waiting in the Outbox. Last try: {error}", error = crate::i18n::error(e)),
        Some(e) if e != "offline" => tf!("Waiting in the Outbox. Last try: {error}", error = widgets::shorten(e, 140)),
        _ if item.not_before > s.clock.now => {
            tf!("Held in the Outbox: it will be sent {when}.", when = crate::i18n::date(&dates::full_time(item.not_before, &s.clock)))
        }
        _ => tl!("Waiting in the Outbox: it will be sent when you're back online.").to_string(),
    }
}

/// What can be done with the message: one waiting in the Outbox is sent now, edited or deleted;
/// a draft is carried on with or deleted; other mail is answered, printed, saved and the rest
/// (not in a window of its own, whose ribbon has them).
fn action_bar(s: &Session, st: &UiState, ui: &mut Ui, o: &Open<'_>, acts: &mut Vec<Action>) {
    let t = st.tokens;
    let (r, p) = (o.r, o.p);
    if let Some((_, item)) = s.outbox_entry(&r.key) {
        egui::Frame::new().fill(t.warn.gamma_multiply(0.12)).corner_radius(6).inner_margin(Margin::symmetric(10, 6)).show(ui, |ui| {
            ui.label(RichText::new(outbox_says(&item, s)).color(t.text));
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
    } else if s.view_role() == Some(mailcraft_engine::model::FolderRole::Drafts) || o.summary.is_some_and(|m| m.flags.draft) {
        // A draft: carried on with, as it was saved.
        egui::Frame::new().fill(t.accent_soft).corner_radius(6).inner_margin(Margin::symmetric(10, 6)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(tl!("Not sent yet: this is a draft.")).color(t.text));
        });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            if widgets::pill_button(ui, Icon::Drafts, "Edit", &t).clicked() {
                acts.push(Action::Run("draft.edit", json!({"key": r.key.to_id()})));
            }
            if widgets::pill_button(ui, Icon::Delete, "Delete", &t).clicked() {
                acts.push(Action::Run("mail.delete", json!({})));
            }
        });
    } else if !o.in_window {
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
            // Not to a reaction itself, a list's mail, or encrypted mail (it would go
            // unencrypted).
            if o.summary.is_none_or(|m| m.reaction.is_empty() && !m.bulk) && !p.bulk && r.security.as_ref().is_none_or(|sec| sec.lock.is_none()) {
                reactions::react_button(ui, &t, acts);
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
                if ui.button(tl!("View message source")).clicked() {
                    acts.push(Action::Run("message.source", json!({"key": r.key.to_id()})));
                    ui.close();
                }
                plain_text_check(s, st, ui, &r.key, p, acts);
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mailcraft_engine::store::OutboxItem;

    /// A message the server refused for good says it wasn't sent, not that it waits in the
    /// Outbox after a last try (it isn't tried again); one a try failed for, one held until
    /// later and one waiting for the connection say what they said.
    #[test]
    fn a_message_refused_for_good_isnt_said_to_wait() {
        let s = Session::open(None, mailcraft_engine::model::dates::Clock::new(1_760_000_000, 0));
        let item = |error: Option<&str>, refused: bool| OutboxItem { error: error.map(str::to_string), refused, ..Default::default() };
        assert_eq!(outbox_says(&item(Some("550 no such user"), true), &s), "Not sent: 550 no such user");
        assert_eq!(outbox_says(&item(Some("connection reset"), false), &s), "Waiting in the Outbox. Last try: connection reset");
        assert_eq!(outbox_says(&item(Some("offline"), false), &s), "Waiting in the Outbox: it will be sent when you're back online.");
        assert_eq!(outbox_says(&item(None, false), &s), "Waiting in the Outbox: it will be sent when you're back online.");
        let later = OutboxItem { not_before: s.clock.now + 3600, ..Default::default() };
        assert!(outbox_says(&later, &s).starts_with("Held in the Outbox: it will be sent "), "{}", outbox_says(&later, &s));
    }
}
