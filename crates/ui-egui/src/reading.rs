//! The reading pane (the open message with its conversation and attachments) and the inline
//! compose form that replaces it while a message is being written.

use egui::{Align, Color32, Margin, Rect, RichText, Sense, Ui, vec2};
use mailcraft_engine::mime::{self, AttachmentKind};
use mailcraft_engine::model::contacts::Contact;
use mailcraft_engine::model::{MessageKey, Summary, addr, dates};
use mailcraft_engine::{InviteProblem, Session, model::EmailAddress};
use serde_json::json;

use crate::icons::{self, Icon};
use crate::rich_edit;
use crate::theme::{self, Tokens};
use crate::{Action, UiState, widgets};

mod attachments;
pub(crate) mod body;
mod compose;
mod frame_lru;
mod header;
mod invite_day;
#[cfg(test)]
pub(crate) use invite_day::repeats as invite_repeats_for_tests;
mod long_text;
mod people;
mod pictures;
mod quote;
mod reactions;
mod security;
mod signature_card;
pub(crate) mod text_selection;
mod videos;
#[cfg(test)]
pub(crate) use attachments::Shown;
pub(crate) use attachments::ShownAttachment;
pub(crate) use attachments::VIDEO_THUMBS;
pub(crate) use attachments::bound_video_frames;
pub(crate) use attachments::decode;
pub(crate) use attachments::decode_pixels as picture_pixels;
#[cfg(test)]
pub(crate) use attachments::previewed_text;
use attachments::*;
pub(crate) use body::WEB_PICTURES;
#[cfg(test)]
pub(crate) use body::picture_menu_for_tests;
use body::*;
pub(crate) use body::{Marked, marked_job, subject_marks, text_marks};
pub use compose::compose_window;
#[cfg(test)]
pub(crate) use compose::parse_local;
#[cfg(test)]
pub(crate) use compose::send_block_text;
use compose::*;
pub(crate) use compose::{ComposeAlert, Suggested};
pub(crate) use frame_lru::FrameLru;
pub(crate) use long_text::LongText;
#[cfg(test)]
pub(crate) use long_text::{CHUNK, chunks};
use people::*;
pub(crate) use pictures::{Got, Pictures, Want};
pub(crate) use quote::QuoteEditors;
#[cfg(test)]
pub(crate) use quote::cell_id;
pub(crate) use signature_card::SignatureCards;
pub(crate) use text_selection::TextSelection;
pub(crate) use videos::Videos;

pub fn show(s: &mut Session, st: &mut UiState, ui: &mut Ui, acts: &mut Vec<Action>) {
    if s.compose.as_ref().is_some_and(|c| crate::compose_inline(c, ui.ctx())) {
        compose(s, st, ui, acts, false);
        return;
    }
    let t = st.tokens;
    if s.reading.is_none() {
        // Several selected: how many, as Outlook's pane says (issue #26).
        let n = s.selection.len();
        if n > 1 {
            empty(ui, &t, &tf!("{n} messages selected", n = n));
        } else {
            empty(ui, &t, if s.list.is_empty() { "" } else { tl!("Select a message to read it") });
        }
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
    if mine.is_empty() {
        return;
    }
    // An opened file is in no list to open it from: it stands in as the open message instead.
    if s.opened_file_name(&key).is_some() {
        acts.extend(mine.into_iter().map(|a| Action::InWindow(key.clone(), Box::new(a))));
    } else {
        acts.push(Action::Open(key));
        acts.extend(mine);
    }
}

/// One message's "Show as plain text", in its More actions (⋯): here rather than over the text,
/// where it was clicked by mistake, and for this message only (File › Options has it for all).
pub(crate) fn plain_text_check(s: &Session, st: &UiState, ui: &mut Ui, key: &MessageKey, p: &mime::Parsed, acts: &mut Vec<Action>) {
    if p.had_html && !p.rich.is_empty() {
        let mut plain = !shown_formatted(s, st, key, p);
        if ui.checkbox(&mut plain, tl!("Show as plain text")).clicked() {
            acts.push(Action::ShowFormatted(key.clone(), !plain));
            ui.close();
        }
    }
}

/// The least height of a pane whose message header stays at its top while the text scrolls.
const PINNED_PANE: f32 = 360.0;

/// The open message, its parts in hand, as the pieces of the reading pane take it.
struct Open<'a> {
    r: &'a mailcraft_engine::Reading,
    p: &'a mime::Parsed,
    summary: Option<&'a Summary>,
    /// In a window of its own (its ribbon has the actions, it has no conversation list).
    in_window: bool,
}

fn message(s: &mut Session, st: &mut UiState, ui: &mut Ui, acts: &mut Vec<Action>, in_window: bool) {
    let t = st.tokens;
    let Some(r) = s.reading.clone() else { return };
    let summary = s.summary(&r.key).cloned();
    // Being read and parsed on the reading lane (a frame or a few): its subject and a spinner.
    let getting_ready = s.body_waiting(&r.key);
    // A chip clicked a moment ago: its preview, unless that click became a double-click.
    settle_attachment_click(st, ui);
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
    let locked = r.security.as_ref().is_some_and(security::still_locked);
    // The header stays at the top while the text scrolls, as Outlook's (issue #151), in a pane
    // with room for it: half of it at most, as it was last drawn; else all of it scrolls.
    let head_id = egui::Id::unique(("reading-head", in_window, r.key.to_id()));
    let head_was: f32 = ui.ctx().data(|d| d.get_temp(head_id)).unwrap_or(0.0);
    let room = ui.available_height();
    let pinned = r.parsed.as_ref().filter(|_| viewing.is_none() && !locked && room >= PINNED_PANE && head_was <= room / 2.0);
    if let Some(p) = pinned {
        let o = Open { r: &r, p, summary: summary.as_ref(), in_window };
        let top = ui.cursor().top();
        egui::Frame::new().inner_margin(Margin { left: 24, right: 24, top: 18, bottom: 0 }).show(ui, |ui| {
            header::header(s, st, ui, &o, header::Part::Pinned, acts);
            attachments_strip(s, st, ui, &o, shown_formatted(s, st, &r.key, p), acts);
        });
        let edge = ui.cursor().top();
        ui.ctx().data_mut(|d| d.insert_temp(head_id, edge - top));
        let line = ui.available_rect_before_wrap();
        ui.painter().line_segment([egui::pos2(line.left() + 24.0, edge), egui::pos2(line.right() - 24.0, edge)], egui::Stroke::new(1.0, t.border));
    }
    egui::ScrollArea::vertical().auto_shrink(false).id_salt(("reading", in_window, r.key.to_id(), viewing)).show(ui, |ui| {
        let top = if pinned.is_some() { 12 } else { 18 };
        egui::Frame::new().inner_margin(Margin { left: 24, right: 24, top, bottom: 24 }).show(ui, |ui| {
            let Some(p) = &r.parsed else {
                not_ready(ui, &t, &r, summary.as_ref(), getting_ready, acts);
                return;
            };
            let o = Open { r: &r, p, summary: summary.as_ref(), in_window };
            if pinned.is_some() {
                header::header(s, st, ui, &o, header::Part::Scrolled, acts);
                message_text(s, st, ui, &o, shown_formatted(s, st, &r.key, p), acts);
                return;
            }
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
                header::header(s, st, ui, &o, header::Part::All, acts);
            }

            // An encrypted message not decrypted: nothing of it (its smime.p7m) is shown.
            if r.security.as_ref().is_some_and(security::still_locked) {
                return;
            }

            let formatted = shown_formatted(s, st, &r.key, p);
            attachments_strip(s, st, ui, &o, formatted, acts);
            if let Some(index) = viewing
                && let Some(a) = p.attachments.iter().find(|a| a.index == index)
            {
                let sep = ui.available_rect_before_wrap();
                ui.painter().line_segment([sep.left_top(), sep.right_top()], egui::Stroke::new(1.0, t.border));
                ui.add_space(12.0);
                attachment_preview(s, st, ui, &r.key, p, a, acts);
                return;
            }
            message_text(s, st, ui, &o, formatted, acts);
        });
    });
}

/// A message not read yet: why it couldn't be (and Try again), or its subject and a spinner while
/// it's read and parsed, or downloaded.
fn not_ready(ui: &mut Ui, t: &Tokens, r: &mailcraft_engine::Reading, summary: Option<&Summary>, getting_ready: bool, acts: &mut Vec<Action>) {
    if !getting_ready {
        ui.add_space(30.0);
    }
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
        None if getting_ready => {
            if let Some(m) = summary {
                let subject = if m.subject.is_empty() { tl!("(no subject)") } else { m.subject.as_str() };
                subject_heading(ui, subject, t, &[]);
                ui.add_space(8.0);
            }
            ui.spinner();
        }
        None => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new(tl!("Downloading the message…")).color(t.dim));
            });
        }
    }
}

/// The attachments, as chips in rows, with how many there are and Save all; not the pictures
/// the formatted message shows in place, nor an invitation shown as its card.
fn attachments_strip(s: &Session, st: &mut UiState, ui: &mut Ui, o: &Open<'_>, formatted: bool, acts: &mut Vec<Action>) {
    let t = st.tokens;
    let (r, p) = (o.r, o.p);
    // Attachments.
    // Pictures the formatted message shows in place aren't listed again as files.
    let shown: Vec<usize> = if formatted { p.images.iter().filter_map(|i| mime::picture_attachment(p, i)).collect() } else { Vec::new() };
    let in_place = |a: &mime::AttachmentInfo| shown.contains(&a.index);
    // An invitation is shown as its card above, not as a file.
    let invitation = st.invite.as_ref().is_some_and(|(k, _, i, _)| k == &r.key && i.is_some()) && !o.in_window;
    let is_invite_file = |a: &mime::AttachmentInfo| {
        invitation && (a.mime.to_ascii_lowercase().starts_with("text/calendar") || a.name.to_ascii_lowercase().ends_with(".ics"))
    };
    let files: Vec<_> =
        p.attachments.iter().filter(|a| (!a.inline || a.kind() == AttachmentKind::Image) && !in_place(a) && !is_invite_file(a)).cloned().collect();
    if files.is_empty() {
        return;
    }
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(if files.len() == 1 { tl!("1 attachment").to_string() } else { tf!("{n} attachments", n = files.len()) })
                .color(t.dim)
                .small(),
        );
        // Its text came first (issue #24): the files are on their way.
        if r.text_first {
            ui.add(egui::Spinner::new().size(11.0));
            ui.label(RichText::new(tl!("Downloading the attachments…")).color(t.dim).small());
        } else if files.len() > 1 && ui.link(RichText::new(tl!("Save all…")).small()).clicked() {
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

/// The message's text, under the bars about it (words hidden, its formatting, its mailing list):
/// formatted, or plain, selected as one whatever it's drawn as.
fn message_text(s: &Session, st: &mut UiState, ui: &mut Ui, o: &Open<'_>, formatted: bool, acts: &mut Vec<Action>) {
    let t = st.tokens;
    let (r, p) = (o.r, o.p);
    if p.hidden_words > 0 {
        hidden_bar(&t, ui, &r.key, p);
    }
    if p.had_html {
        html_bar(s, &t, ui, &r.key, p, formatted, acts);
    }
    if let Some(u) = &p.unsubscribe {
        ui.label(RichText::new(tf!("Mailing list: unsubscribe at {address}", address = widgets::shorten(u, 80))).color(t.faint).small());
        ui.add_space(6.0);
    }
    // In the dark theme: the message on a light page or the dark one, switched here too (issue #109).
    if t.dark {
        // A row of its own, its height only (a layout of the rest took all the height left, and
        // the text went below the pane).
        ui.allocate_ui_with_layout(vec2(ui.available_width(), 22.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let (icon, tip) = if s.prefs.light_messages {
                (Icon::Moon, "Show messages on the dark background")
            } else {
                (Icon::Sun, "Show messages on a light background")
            };
            if widgets::tiny_icon_button(ui, icon, tip, &t).clicked() {
                acts.push(Action::Run("view.lightMessages", json!({})));
            }
        });
    }
    let sep = ui.available_rect_before_wrap();
    ui.painter().line_segment([sep.left_top(), sep.right_top()], egui::Stroke::new(1.0, t.border));
    ui.add_space(12.0);
    if han_missing(ui, &p.text) {
        st.han_missing = true;
    }
    if t.dark && s.prefs.light_messages {
        // The light theme's colours for the text, on its page: put back once it's drawn.
        let light = theme::Tokens::light();
        st.tokens = light;
        egui::Frame::new().fill(light.surface).corner_radius(6).inner_margin(Margin::same(14)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            // Text drawn in the visuals' colour (plain text, labels) and links too, here only.
            let v = ui.visuals_mut();
            v.override_text_color = Some(light.text);
            v.hyperlink_color = light.accent;
            v.widgets.noninteractive.fg_stroke.color = light.text;
            v.selection.bg_fill = light.accent.gamma_multiply(0.35);
            message_body(s, st, ui, o, formatted, acts);
        });
        st.tokens = t;
        return;
    }
    message_body(s, st, ui, o, formatted, acts);
}

/// The message's text itself, in the reading pane's colours (`st.tokens`).
fn message_body(s: &Session, st: &mut UiState, ui: &mut Ui, o: &Open<'_>, formatted: bool, acts: &mut Vec<Action>) {
    let t = st.tokens;
    let (r, p) = (o.r, o.p);
    // The text is selected as one, whatever it's drawn as (`text_selection`).
    text_selection::begin(st, ui, ui.scope_id().with(("message-text", r.key.to_id())), o.in_window);
    // A search on: what it found marked (issue #173), the text drawn as formatted lines.
    if let Some(m) = marked(s, st, &r.key, p, formatted) {
        rich_body(s, st, ui, &r.key, &m, acts);
        text_selection::finish(st, ui, long_text::many_lines(&m).then_some(p.text.as_str()));
        return;
    }
    if formatted {
        rich_body(s, st, ui, &r.key, p, acts);
    } else if p.text.len() > long_text::LONG {
        // Laid out by what's in view: whole, a long one took a frame of its own to lay out.
        long_text::plain(st, ui, ui.scope_id().with(("long-text", r.key.to_id())), &p.text);
    } else if crate::bidi::has_rtl(&p.text) {
        // Right-to-left text, against the right edge where it starts; selected with the
        // pointer as it reads, and right-click copies it as written too.
        let job = egui::text::LayoutJob::simple(p.text.clone(), theme::regular(14.5), t.text, ui.available_width());
        let r = text_selection::rtl_run(st, ui, job);
        rtl_copy_menu(&r, None, &p.text);
    } else {
        text_selection::run(st, ui, RichText::new(&p.text).font(theme::regular(14.5)).color(t.text), None);
    }
    // Laid out by what's in view, a long text's copy of all of it is its text.
    let long = if formatted { long_text::many_lines(p) } else { p.text.len() > long_text::LONG };
    text_selection::finish(st, ui, long.then_some(p.text.as_str()));
}

/// The External tag: a sender from outside the organisation (issue #29). One line however
/// narrow the pane: wrapped, its word stood in a column a letter wide.
pub(crate) fn external_tag(ui: &mut Ui, t: &Tokens) -> egui::Response {
    egui::Frame::new()
        .stroke(egui::Stroke::new(1.0, t.border))
        .corner_radius(4)
        .inner_margin(Margin::symmetric(5, 1))
        .show(ui, |ui| {
            ui.add(egui::Label::new(RichText::new(tl!("External")).color(t.dim).small()).wrap_mode(egui::TextWrapMode::Extend));
        })
        .response
        .on_hover_text(tl!("This sender is outside your organisation."))
}

/// A message file opened to be looked at (issue #56): where it's from, and what it's spared.
fn opened_file_bar(t: &Tokens, ui: &mut Ui, name: &str) {
    egui::Frame::new().fill(t.chrome).stroke(egui::Stroke::new(1.0, t.border)).corner_radius(6).inner_margin(Margin::symmetric(10, 6)).show(
        ui,
        |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (ir, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
                icons::paint(ui.painter(), ir, Icon::File, t.dim);
                // Wrapped in a narrow window (on one line it widened the whole message past it).
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new(tf!(
                            "Opened from {file}. It isn't in any folder, its pictures from the web are loaded only if you ask, and closing it leaves no trace.",
                            file = name
                        ))
                        .color(t.dim),
                    );
                });
            });
        },
    );
    ui.add_space(8.0);
}

/// Who the message says it's from doesn't add up: said plainly, above everything it asks for.
pub(crate) fn sender_warning(t: &Tokens, ui: &mut Ui, w: &mailcraft_engine::model::lookalike::SenderWarning) {
    use mailcraft_engine::model::lookalike::SenderWarning;
    let what = match w {
        SenderWarning::Lookalike { domain, like } => {
            tf!("Careful: this comes from {domain}, which looks like {like} but isn't it.", domain = domain, like = like)
        }
        SenderWarning::NameShowsOther { shown, real } => {
            tf!("Careful: the sender's name shows {shown}, but this comes from {real}.", shown = shown, real = real)
        }
        SenderWarning::NameOfKnownPerson { name, known, real } => {
            tf!("Careful: you know {name} as {known}, but this comes from {real}.", name = name, known = known, real = real)
        }
    };
    egui::Frame::new().fill(t.danger.gamma_multiply(0.12)).corner_radius(6).inner_margin(Margin::symmetric(10, 8)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
            icons::paint(ui.painter(), r, Icon::Warning, t.danger);
            ui.vertical(|ui| {
                ui.label(RichText::new(what).color(t.text));
                ui.label(
                    RichText::new(tl!("Don't open its links or attachments, or answer it, unless you're sure who sent it.")).color(t.dim).small(),
                );
            });
        });
    });
    ui.add_space(8.0);
}

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
fn invite_card(s: &Session, st: &mut UiState, ui: &mut Ui, key: &MessageKey, acts: &mut Vec<Action>) {
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
                    ui.label(RichText::new(invite_day::repeats(rule)).color(t.dim));
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
                    invite_day::show(s, &t, ui, e, day, acts);
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
    crate::calendar::show_day(s.clock.local_day(t), acts);
}

/// The reading pane with no message in it, saying `msg` (as shown: its callers translate it).
fn empty(ui: &mut Ui, t: &Tokens, msg: &str) {
    widgets::empty_state(ui, t, Icon::Read, msg);
}

/// The messages of the open one's conversation, the oldest first: those in this list, and the
/// user's own from Sent Items (`aside`, issue #148), dimmed with their folder's name and opened
/// in a window of their own, as Outlook shows a conversation's messages from other folders.
fn thread_list(
    s: &Session,
    t: &Tokens,
    ui: &mut Ui,
    thread: &[MessageKey],
    aside: &[mailcraft_engine::model::SharedSummary],
    open: &MessageKey,
    acts: &mut Vec<Action>,
) {
    let here: Vec<&Summary> =
        if thread.is_empty() { s.summary(open).into_iter().collect() } else { thread.iter().filter_map(|k| s.summary(k)).collect() };
    let mut rows: Vec<(&Summary, bool)> = here.into_iter().map(|m| (m, false)).chain(aside.iter().map(|m| (&**m, true))).collect();
    rows.sort_by_key(|(m, _)| m.when());
    egui::Frame::new().fill(t.chrome).stroke(egui::Stroke::new(1.0, t.border)).corner_radius(8).inner_margin(Margin::symmetric(10, 8)).show(
        ui,
        |ui| {
            ui.label(RichText::new(tf!("{n} messages in this conversation", n = rows.len())).color(t.dim).small());
            for (m, sent) in rows {
                let k = &m.key;
                let current = k == open;
                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click());
                if current || resp.hovered() {
                    ui.painter().rect_filled(rect, 4.0, if current { t.accent_soft } else { t.hover_row });
                }
                let font = if !m.flags.seen && !sent { theme::bold(13.0) } else { theme::regular(13.0) };
                let p = ui.painter_at(rect);
                // Who, the first words and when, each in its own room: the words ran on under
                // the time in a narrow pane, and a long name under the words. One from Sent
                // Items says so before its time.
                let folder = sent
                    .then(|| s.account(&k.account).and_then(|a| a.folder(mailcraft_engine::model::FolderRole::Sent)).map(crate::i18n::folder))
                    .flatten();
                let date = crate::i18n::date(&dates::list_time(m.when(), &s.clock));
                let when = p.layout_no_wrap(folder.map_or(date.clone(), |f| format!("{f} · {date}")), theme::regular(12.0), t.dim);
                let who = widgets::one_line(ui, m.from.display(), font, if sent { t.dim } else { t.text }, 154.0);
                let mid = |g: &egui::Galley| rect.center().y - g.size().y / 2.0;
                let room = rect.width() - 170.0 - when.size().x - 20.0;
                p.galley(egui::pos2(rect.left() + 8.0, mid(&who)), who, t.text);
                if room > 24.0 {
                    let words = widgets::one_line(ui, &m.preview, theme::regular(12.5), t.dim, room);
                    p.galley(egui::pos2(rect.left() + 170.0, mid(&words)), words, t.dim);
                }
                p.galley(egui::pos2(rect.right() - 8.0 - when.size().x, mid(&when)), when, t.dim);
                if resp.clicked() && sent {
                    acts.push(Action::Run("message.popOut", json!({"key": k.to_id()})));
                } else if resp.clicked() && !current {
                    acts.push(Action::Open(k.clone()));
                }
            }
        },
    );
}

/// Whether `text` has Chinese characters (or kana, hangul) that no font installed now can draw:
/// in the browser, which brings only some fonts, the Simplified Chinese one is then fetched. The
/// first few thousand letters are looked at, each distinct character once.
pub(crate) fn han_missing(ui: &Ui, text: &str) -> bool {
    let cjk = |c: char| matches!(u32::from(c), 0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF);
    let mut seen: Vec<char> = Vec::new();
    for c in text.chars().take(4000).filter(|c| cjk(*c)) {
        if seen.len() >= 64 {
            break;
        }
        if !seen.contains(&c) {
            seen.push(c);
        }
    }
    if seen.is_empty() {
        return false;
    }
    let font = egui::FontId::proportional(14.5);
    ui.fonts_mut(|f| seen.iter().any(|c| !f.has_glyph(&font, *c)))
}
