//! The message list: date headings, rows with avatar, sender, subject and preview, unread and
//! flag markers, hover actions, multi-select (Ctrl/Shift), drag to a folder, right-click menu.
//! Only visible rows are laid out, so long folders scroll smoothly.

use egui::{Color32, Rect, RichText, Sense, Ui, pos2, vec2};
use mailcraft_engine::model::search::Problem;
use mailcraft_engine::model::{FolderRole, MessageKey, Summary, addr, dates};
use mailcraft_engine::{ListFilter, Row, SearchScope, Session};
use serde_json::json;

use crate::icons::{self, Icon};
use crate::progress;
use crate::theme;
use crate::widgets;
use crate::{Action, Dialog, UiState};

/// The tops of the list's rows (date headings and messages), worked out again only when the
/// engine builds its rows again ([`Session::nav_structure`]: the list changed, the day turned) or
/// the row height changes. The rows themselves are the engine's, borrowed each frame: a big
/// folder's hundreds of thousands of rows were made again every frame, then each time it changed.
pub(crate) struct ListRows {
    key: (u64, u32),
    tops: Vec<f32>,
    total: f32,
}

/// What a dragged row carries.
pub struct DragKeys(pub Vec<MessageKey>);

const HEADER_H: f32 = 30.0;

fn row_height(s: &Session) -> f32 {
    match (s.prefs.compact, s.prefs.show_preview) {
        (true, _) => 46.0,
        (false, true) => 76.0,
        (false, false) => 58.0,
    }
}

pub fn show(s: &Session, st: &mut UiState, ui: &mut Ui, acts: &mut Vec<Action>) {
    let t = st.tokens;
    egui::Frame::new().inner_margin(egui::Margin { left: 14, right: 10, top: 10, bottom: 6 }).show(ui, |ui| {
        // Against the right in a right-to-left interface (egui runs its rows from there too).
        ui.with_layout(crate::bidi::column(), |ui| list_head(s, &t, ui, acts));
    });
    list_body(s, st, ui, acts);
}

/// The list's name, its Focused and Other tabs, its filters and how it's arranged.
fn list_head(s: &Session, t: &theme::Tokens, ui: &mut Ui, acts: &mut Vec<Action>) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(crate::view_title(s)).font(theme::bold(18.0)).color(t.text));
    });
    ui.add_space(4.0);
    if s.focus_split() {
        ui.horizontal(|ui| {
            for (other, label) in [(false, "Focused"), (true, "Other")] {
                let on = s.focus_other == other;
                let text =
                    RichText::new(tl!(label)).color(if on { t.text } else { t.dim }).font(if on { theme::bold(14.0) } else { theme::regular(14.0) });
                let r = ui.add(egui::Button::new(text).frame_when_inactive(false));
                if on {
                    ui.painter().line_segment(
                        [pos2(r.rect.left() + 2.0, r.rect.bottom() + 1.0), pos2(r.rect.right() - 2.0, r.rect.bottom() + 1.0)],
                        egui::Stroke::new(2.0, t.accent),
                    );
                }
                if r.clicked() && !on {
                    acts.push(Action::Run("list.focusTab", json!({"value": if other { "other" } else { "focused" }})));
                }
            }
        });
        ui.add_space(2.0);
    }
    ui.horizontal(|ui| {
        // How the list is arranged, kept at the right (as Outlook's "By Date" above its list):
        // the tabs wrap in what's left, so long words (Dutch, German…) never run into it.
        let arrange = tf!("By {field} ⏷", field = tl!(s.prefs.arrange.label()));
        let right = if s.search.is_none() {
            ui.painter().layout_no_wrap(arrange.clone(), egui::TextStyle::Small.resolve(ui.style()), t.dim).size().x + 54.0
        } else {
            0.0
        };
        let left_w = (ui.available_width() - right).max(120.0);
        // The tabs, and a filter chosen from the Home tab's Filter menu when it's another. They
        // stay on one line: those that don't fit go into a "⋯" menu after the rest (the chosen
        // one always shows), so the head keeps its height however narrow the pane.
        let shown: Vec<ListFilter> = ListFilter::ALL.into_iter().chain((!ListFilter::ALL.contains(&s.filter)).then_some(s.filter)).collect();
        let font = |on: bool| if on { theme::bold(12.5) } else { theme::regular(12.5) };
        let pad = ui.spacing().button_padding.x * 2.0;
        let gap = ui.spacing().item_spacing.x;
        let width = |f: ListFilter| ui.painter().layout_no_wrap(tl!(f.label()).to_string(), font(s.filter == f), t.dim).size().x + pad;
        let scope = s.search.as_ref().filter(|_| s.open_search_folder_name().is_none());
        let scope_w = scope.map_or(0.0, |(_, current)| {
            let label = format!("{} ▾", crate::scope_label(*current));
            ui.painter().layout_no_wrap(label, egui::TextStyle::Small.resolve(ui.style()), t.dim).size().x + pad + 8.0 + gap
        });
        let (kept, hidden) = fit_tabs(&shown, s.filter, width, left_w - scope_w, gap);
        ui.allocate_ui_with_layout(vec2(left_w, 22.0), crate::bidi::row_from_start(egui::Align::Center), |ui| {
            for f in kept {
                let on = s.filter == f;
                let text = RichText::new(tl!(f.label())).color(if on { t.accent } else { t.dim }).font(font(on));
                let r = ui.add(egui::Button::new(text).frame_when_inactive(false));
                if on {
                    ui.painter().line_segment(
                        [pos2(r.rect.left() + 4.0, r.rect.bottom()), pos2(r.rect.right() - 4.0, r.rect.bottom())],
                        egui::Stroke::new(2.0, t.accent),
                    );
                }
                if r.clicked() {
                    acts.push(Action::Run("list.filter", json!({"value": f.code()})));
                }
            }
            if !hidden.is_empty() {
                let r = more_tabs(ui, t);
                egui::Popup::menu(&r).show(|ui| {
                    for f in &hidden {
                        if ui.selectable_label(false, tl!(f.label())).clicked() {
                            acts.push(Action::Run("list.filter", json!({"value": f.code()})));
                            ui.close();
                        }
                    }
                });
            }
            // A Search Folder always looks everywhere: no scope to pick.
            if let Some((q, current)) = scope {
                ui.add_space(8.0);
                ui.menu_button(RichText::new(format!("{} ▾", crate::scope_label(*current))).small(), |ui| {
                    for scope in SearchScope::ALL {
                        if ui.selectable_label(scope == *current, crate::scope_label(scope)).clicked() {
                            acts.push(Action::Run("search.run", json!({"query": q, "scope": scope.code()})));
                            ui.close();
                        }
                    }
                })
                .response
                .on_hover_text(tl!("Look in"));
            }
        });
        if s.search.is_none() {
            ui.with_layout(crate::bidi::row_from_end(egui::Align::Center), |ui| {
                let rev = s.prefs.reverse_sort;
                if ui
                    .add(egui::Button::new(RichText::new(if rev { "↓" } else { "↑" }).color(t.dim)).frame_when_inactive(false))
                    .on_hover_text(tl!("Reverse sort"))
                    .clicked()
                {
                    acts.push(Action::Run("view.reverseSort", json!({})));
                }
                let r = ui.add(egui::Button::new(RichText::new(arrange).color(t.dim).small()).frame_when_inactive(false));
                egui::Popup::menu(&r).show(|ui| arrange_menu(ui, s, acts));
            });
        }
    });
    search_status(s, t, ui);
}

/// The filter tabs that fit in `room` (each `width` wide, `gap` apart) and those that go into
/// the "⋯" menu: in order, the chosen one (`on`) always kept, room left for the menu's button.
fn fit_tabs(shown: &[ListFilter], on: ListFilter, width: impl Fn(ListFilter) -> f32, room: f32, gap: f32) -> (Vec<ListFilter>, Vec<ListFilter>) {
    let all: f32 = shown.iter().map(|f| width(*f) + gap).sum();
    if all - gap <= room {
        return (shown.to_vec(), Vec::new());
    }
    let mut left = room - MORE_W - width(on) - gap;
    let (mut kept, mut hidden) = (Vec::new(), Vec::new());
    for f in shown.iter().copied() {
        if f == on {
            kept.push(f);
        } else if hidden.is_empty() && width(f) + gap <= left {
            left -= width(f) + gap;
            kept.push(f);
        } else {
            hidden.push(f);
        }
    }
    (kept, hidden)
}

/// How wide the "⋯" button of the filters that don't fit is.
const MORE_W: f32 = 24.0;

/// The "⋯" button of the filters that don't fit, as quiet as the tabs.
fn more_tabs(ui: &mut Ui, t: &theme::Tokens) -> egui::Response {
    let (rect, r) = ui.allocate_exact_size(vec2(MORE_W, 22.0), egui::Sense::click());
    if r.hovered() {
        ui.painter().rect_filled(rect, 4.0, t.hover);
    }
    for dx in [-5.0, 0.0, 5.0] {
        ui.painter().circle_filled(rect.center() + vec2(dx, 0.0), 1.5, t.dim);
    }
    crate::widgets::named(ui, &r, tl!("Filter"), true, None);
    r.on_hover_text(tl!("Filter"))
}

/// How the search is going, under the list's head: still searching; what may be missing (mail
/// whose text the search index doesn't have yet), or, more quietly, mail too long to search
/// whole; what couldn't be read of what was typed.
fn search_status(s: &Session, t: &theme::Tokens, ui: &mut Ui) {
    if s.search.is_none() {
        return;
    }
    let progress = s.search_progress();
    let running = progress.is_some_and(|p| !p.done);
    let missing = progress.is_some_and(|p| p.missing > 0);
    // Not waiting for anything, so not "still indexing" (it showed with every message indexed).
    let cut_short = !missing && progress.is_some_and(|p| p.cut_short > 0);
    let problems = s.search_problems();
    if !running && !missing && !cut_short && problems.is_empty() {
        return;
    }
    ui.add_space(2.0);
    ui.horizontal_wrapped(|ui| {
        if running {
            ui.spinner();
            ui.label(RichText::new(tl!("Searching…")).color(t.dim).small());
        }
        if missing {
            let text = match s.indexing_work().and_then(|w| w.permille(mailcraft_engine::model::job::ITEM_WEIGHT)) {
                Some(p) => tf!("Some results may be missing: still indexing ({percent}%)", percent = progress::percent(p)),
                None => tl!("Some results may be missing: still indexing").to_string(),
            };
            ui.label(RichText::new(text).color(t.warn).small());
        }
        if cut_short {
            ui.label(RichText::new(tl!("Some long messages were searched only in part")).color(t.dim).small());
        }
        for p in problems {
            let text = match p {
                Problem::Date(value) => tf!("Couldn't read the date “{value}”", value = value),
            };
            ui.label(RichText::new(text).color(t.warn).small());
        }
    });
}

/// The list's rows, under its head.
fn list_body(s: &Session, st: &mut UiState, ui: &mut Ui, acts: &mut Vec<Action>) {
    let t = st.tokens;
    let sep = ui.available_rect_before_wrap();
    ui.painter().line_segment([sep.left_top(), sep.right_top()], egui::Stroke::new(1.0, t.border));

    let mh = row_height(s);
    let rows = s.rows();
    let key = (s.nav_structure(), mh.to_bits());
    if st.list_rows.as_ref().is_none_or(|c| c.key != key || c.tops.len() != rows.len()) {
        let mut tops = Vec::with_capacity(rows.len());
        let mut y = 0.0;
        for r in rows {
            tops.push(y);
            y += if matches!(r, Row::Header(_) | Row::Group(_)) { HEADER_H } else { mh };
        }
        st.list_rows = Some(ListRows { key, tops, total: y });
    }
    let Some(cached) = st.list_rows.take() else { return };
    let ListRows { tops, total, .. } = &cached;
    let (tops, total) = (tops.as_slice(), *total);
    if rows.is_empty() {
        ui.vertical_centered(|ui| {
            ui.add_space(60.0);
            let (r, _) = ui.allocate_exact_size(vec2(44.0, 44.0), Sense::hover());
            icons::paint(ui.painter(), r, if s.search.is_some() { Icon::Search } else { Icon::Inbox }, t.faint);
            ui.add_space(8.0);
            let msg = if s.search_progress().is_some_and(|p| !p.done) {
                tl!("Searching…")
            } else if s.search.as_ref().is_some_and(|(_, scope)| *scope == SearchScope::All) {
                tl!("No results. Try fewer words.")
            } else if s.search.is_some() {
                tl!("No results. Try fewer words, or search all folders.")
            } else if s.filter != ListFilter::All {
                tl!("Nothing matches this filter.")
            } else {
                tl!("All caught up: nothing in this folder.")
            };
            ui.label(RichText::new(msg).color(t.dim));
            ui.add_space(10.0);
            footer(s, &t, ui, acts);
        });
        st.list_rows = Some(cached);
        return;
    }

    let scroll_to = if st.scroll_to_focus {
        st.scroll_to_focus = false;
        s.focus.as_ref().and_then(|k| s.row_index_of(k))
    } else {
        None
    };
    let sent_view = matches!(s.view_role(), Some(FolderRole::Sent | FolderRole::Drafts)) || s.view == mailcraft_engine::View::Outbox;

    // After the last message: more on the server, or searching it (issue #4).
    let has_footer = s.more_on_server().is_some() || s.search.is_some();
    // A page (Page Up, Page Down): the rows shown at once, less one kept in view.
    st.list_page = ((ui.available_height() / mh.max(1.0)).floor() as u64).saturating_sub(1).max(1);
    let empty_cards = std::collections::HashMap::new();
    let cards = if s.prefs.compact {
        &empty_cards
    } else {
        let pim_key = (s.pim_revision, s.pim.values().map(|p| p.cards.len()).sum::<usize>());
        if st.cards_by_email.as_ref().is_none_or(|(k, _)| *k != pim_key) {
            st.cards_by_email = Some((pim_key, crate::reading::cards_by_email(s)));
        }
        st.cards_by_email.as_ref().map_or(&empty_cards, |(_, m)| m)
    };
    egui::ScrollArea::vertical().auto_shrink(false).id_salt("list").show_viewport(ui, |ui, viewport| {
        ui.set_height(total + if has_footer { FOOTER_H } else { 0.0 });
        let origin = ui.min_rect().min;
        let width = ui.available_width();
        if let Some(ri) = scroll_to
            && let Some(&top) = tops.get(ri)
        {
            let r = Rect::from_min_size(origin + vec2(0.0, top), vec2(width, mh));
            ui.scroll_to_rect(r, None);
        }
        // The messages a reply or forward was begun to and left in Drafts: marked [Draft], as
        // Outlook marks them (issue #52); not in Sent or Drafts, whose rows are the replies and
        // forwards themselves.
        let drafted = if sent_view { Default::default() } else { s.drafted() };
        let first = tops.partition_point(|&top| top + mh < viewport.min.y);
        for (i, row) in rows.iter().enumerate().skip(first) {
            let top = tops.get(i).copied().unwrap_or(0.0);
            if top > viewport.max.y {
                break;
            }
            match row {
                Row::Header(label) => {
                    let r = Rect::from_min_size(origin + vec2(0.0, top), vec2(width, HEADER_H));
                    let m = crate::bidi::Mirror::new(r);
                    ui.painter().text(
                        m.pos(r.left_bottom() + vec2(14.0, -8.0)),
                        m.align(egui::Align2::LEFT_BOTTOM),
                        tl!(label),
                        theme::bold(12.5),
                        t.dim,
                    );
                }
                Row::Group(name) => {
                    let r = Rect::from_min_size(origin + vec2(0.0, top), vec2(width, HEADER_H));
                    let (name, _) = crate::bidi::shown(name);
                    let m = crate::bidi::Mirror::new(r);
                    ui.painter().text(
                        m.pos(r.left_bottom() + vec2(14.0, -8.0)),
                        m.align(egui::Align2::LEFT_BOTTOM),
                        name.as_ref(),
                        theme::bold(12.5),
                        t.dim,
                    );
                }
                Row::Message { index, count, unread } => {
                    let Some(m) = s.list.get(*index) else { continue };
                    let r = Rect::from_min_size(origin + vec2(0.0, top), vec2(width, mh));
                    let drafting = drafted.marks(m);
                    message_row(s, &t, ui, r, m, *count, *unread, sent_view, drafting, cards, acts);
                }
            }
        }
        if has_footer && total < viewport.max.y {
            let r = Rect::from_min_size(origin + vec2(0.0, total), vec2(width, FOOTER_H));
            ui.scope_builder(egui::UiBuilder::new().max_rect(r.shrink2(vec2(14.0, 8.0))), |ui| {
                ui.vertical_centered(|ui| footer(s, &t, ui, acts));
            });
        }
    });
    st.list_rows = Some(cached);
}

/// The list's footer's height.
const FOOTER_H: f32 = 76.0;

/// What's past the list (issue #4): a folder's older mail on its server, to show more of; a
/// search's, to ask the servers for.
fn footer(s: &Session, t: &theme::Tokens, ui: &mut Ui, acts: &mut Vec<Action>) {
    if let Some((account, folder, n)) = s.more_on_server() {
        ui.label(RichText::new(tf!("{n} more on the server", n = n)).color(t.dim).small());
        if s.showing_more.as_ref().is_some_and(|(a, f)| *a == account && *f == folder) {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new(tl!("Getting them from the server…")).color(t.dim).small());
            });
        } else if ui.button(tl!("Show more")).clicked() {
            acts.push(Action::Run("folder.more", json!({"account": account, "folder": folder})));
        }
        return;
    }
    if s.search.is_none() || s.accounts.iter().all(|a| a.is_local() || a.config.pop.is_some()) {
        return;
    }
    match &s.server_search {
        Some(ss) if Some(&ss.query) == s.search.as_ref().map(|(q, _)| q) => {
            if ss.pending > 0 {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(RichText::new(tl!("Searching the server…")).color(t.dim).small());
                });
            } else {
                ui.label(RichText::new(tf!("The server found {n}", n = ss.found.len())).color(t.dim).small());
            }
            for e in ss.errors.iter().take(3) {
                ui.add(egui::Label::new(RichText::new(e).color(t.warn).small()).wrap());
            }
        }
        _ => {
            ui.label(RichText::new(tl!("Mail older than what's kept here is only on the server.")).color(t.dim).small());
            if ui.button(tl!("Search the server")).clicked() {
                acts.push(Action::Run("search.server", json!({})));
            }
        }
    }
}

/// Arrange the list by…, as a menu (the list's own and the View tab's).
pub(crate) fn arrange_menu(ui: &mut Ui, s: &Session, acts: &mut Vec<Action>) {
    for a in mailcraft_engine::Arrange::ALL {
        if ui.selectable_label(s.prefs.arrange == a, tl!(a.label())).clicked() {
            acts.push(Action::Run("view.arrange", json!({"by": a.code()})));
            ui.close();
        }
    }
    ui.separator();
    if ui.selectable_label(s.prefs.reverse_sort, tl!("Reverse sort")).clicked() {
        acts.push(Action::Run("view.reverseSort", json!({})));
        ui.close();
    }
}

/// Renders a single message or thread row in the mail list pane.
#[allow(clippy::too_many_arguments)]
fn message_row(
    s: &Session,
    t: &theme::Tokens,
    ui: &mut Ui,
    r: Rect,
    m: &Summary,
    count: usize,
    unread_in_thread: usize,
    sent_view: bool,
    drafting: bool,
    cards: &std::collections::HashMap<String, std::sync::Arc<mailcraft_engine::model::vcard::Card>>,
    acts: &mut Vec<Action>,
) {
    let id = ui.make_persistent_id(("row", &m.key));
    let resp = ui.interact(r, id, Sense::click_and_drag());
    // The pointer anywhere on the row, its own buttons included (egui hovers only the button
    // under the pointer, and the buttons must stay while they're pointed at).
    let hovered = ui.rect_contains_pointer(r) && !ui.ctx().is_being_dragged(id) || resp.hovered();
    let selected = s.selection.contains(&m.key);
    let unread = if count > 1 { unread_in_thread > 0 } else { !m.flags.seen };
    // Who the sender is (issue #29): their address beside their name, a warning, External.
    let marks = if sent_view { mailcraft_engine::SenderMarks::default() } else { s.sender_marks(m) };
    // To a screen reader a row is one item of the list: who, the subject, when, and what its
    // markers show (unread, flagged, attachments).
    resp.widget_info(|| {
        let mut who = if sent_view { m.to.first().map(|a| a.display().to_string()).unwrap_or_default() } else { m.from.display().to_string() };
        if marks.address {
            who = format!("{who} <{}>", m.from.email.trim());
        }
        let subject = if m.subject.is_empty() { tl!("(no subject)") } else { m.subject.as_str() };
        let mut spoken = vec![who, subject.to_string(), crate::i18n::date(&dates::list_time(m.when(), &s.clock))];
        if drafting {
            spoken.insert(0, tl!("[Draft]").to_string());
        }
        for (on, word) in [
            (marks.warning, "Check who sent it"),
            (marks.external, "External"),
            (unread, "Unread"),
            (m.flags.flagged, "Flagged"),
            (m.flags.follow_up_complete(), "Completed"),
            (m.has_attachments, "Has attachments"),
        ] {
            if on {
                spoken.push(tl!(word).to_string());
            }
        }
        egui::WidgetInfo::selected(egui::Role::ListItem, true, selected, spoken.join(", "))
    });
    let p = ui.painter_at(r);
    let sel = crate::motion::toward(ui.ctx(), id.with("selected"), selected, crate::motion::NORMAL);
    let hover = crate::motion::toward(ui.ctx(), id.with("hover"), hovered, crate::motion::FAST);
    let fill = crate::motion::mix(t.hover_row.gamma_multiply(hover), t.accent_soft, sel);
    p.rect_filled(r.shrink2(vec2(4.0, 1.0)), 6.0, fill);
    // Laid out left to right, mirrored for a right-to-left interface: the avatar on the right,
    // the time and markers on the left.
    let mr = crate::bidi::Mirror::new(r);
    if unread {
        p.rect_filled(mr.rect(Rect::from_min_size(r.min + vec2(4.0, 8.0), vec2(3.0, r.height() - 16.0))), 2.0, t.accent);
    }
    let rule = [mr.pos(r.left_bottom() + vec2(14.0, 0.0)), mr.pos(r.right_bottom() - vec2(8.0, 0.0))];
    p.line_segment(rule, egui::Stroke::new(1.0, t.border.gamma_multiply(0.6)));

    let compact = s.prefs.compact;
    let mut x = r.min.x + 14.0;
    let who_addr = if sent_view { m.to.first().cloned().unwrap_or_default() } else { m.from.clone() };
    if !compact {
        let c = mr.pos(egui::pos2(x + 17.0, r.min.y + 26.0));
        let card = cards.get(&who_addr.email.trim().to_ascii_lowercase()).map(AsRef::as_ref);
        widgets::card_avatar(ui, c, 17.0, &who_addr.initials(), theme::avatar_color(&who_addr.email, t.dark), card);
        x += 44.0;
    }
    let right = r.max.x - 14.0;
    let line1 = r.min.y + if compact { 6.0 } else { 9.0 };
    // Sender (or recipients in Sent) and time.
    let time = crate::i18n::date(&dates::list_time(m.when(), &s.clock));
    let time_g = p.layout_no_wrap(time, if unread { theme::bold(12.0) } else { theme::regular(12.0) }, if unread { t.accent } else { t.dim });
    let mut tw = time_g.size().x;
    p.galley(egui::pos2(mr.start(right - tw, tw), line1 + 1.0), time_g, t.dim);
    // A search over several folders: each result says where it is, before its time.
    let several =
        s.search.as_ref().is_some_and(|(_, scope)| *scope != SearchScope::Current) || matches!(s.view, mailcraft_engine::View::Unified { .. });
    if let Some(f) = s.folder(&m.key.account, &m.key.folder).filter(|_| several) {
        let name: String = crate::i18n::folder(f).chars().take(24).collect();
        let fg = p.layout_no_wrap(name, theme::regular(11.5), t.faint);
        let fw = fg.size().x;
        p.galley(egui::pos2(mr.start(right - tw - 10.0 - fw, fw), line1 + 2.0), fg, t.faint);
        tw += fw + 10.0;
    }
    // From outside the organisation: a small tag before the time.
    if marks.external {
        let eg = p.layout_no_wrap(tl!("External").to_string(), theme::regular(11.0), t.dim);
        let ew = eg.size().x + 10.0;
        let er = mr.rect(Rect::from_min_size(egui::pos2(right - tw - 10.0 - ew, line1), vec2(ew, 17.0)));
        p.rect_stroke(er, 4.0, egui::Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
        p.galley(er.center() - eg.size() / 2.0, eg, t.dim);
        tw += ew + 10.0;
    }
    let mut who_x = x;
    // Something about the sender doesn't add up: a warning sign before the name (the reading
    // pane says what).
    if marks.warning {
        let ir = mr.rect(Rect::from_min_size(egui::pos2(who_x, line1 + 1.0), vec2(15.0, 15.0)));
        icons::paint(&p, ir, Icon::Warning, t.danger);
        who_x += 19.0;
    }
    // A reply begun and left in Drafts: [Draft] before the name, in red, as Outlook writes it.
    if drafting {
        let dg = p.layout_no_wrap(tl!("[Draft]").to_string(), if unread { theme::bold(14.0) } else { theme::regular(14.0) }, t.danger);
        let dw = dg.size().x;
        p.galley(egui::pos2(mr.start(who_x, dw), line1), dg, t.danger);
        who_x += dw + 6.0;
    }
    let who = if sent_view { tf!("To: {names}", names = addr::display_list(&m.to)) } else { m.from.display().to_string() };
    let who = if count > 1 && !marks.address { format!("{who}  ({count})") } else { who };
    let (who, who_rtl) = crate::bidi::shown(&who);
    let who_right = right - tw - 10.0;
    let who_font = if unread { theme::bold(14.0) } else { theme::regular(14.0) };
    // A line between `from` and `to`, at its start: right-to-left text starts at the right, as
    // everything does in a right-to-left interface. What doesn't fit is cut from its end.
    let place = |rtl: bool, from: f32, to: f32, w: f32| mr.start(if rtl && !mr.on() { to - w } else { from }, w);
    if marks.address {
        // The name, then the address it really came from, dimmer. The address is what tells, so
        // it's laid out first (cut to its domain if it must be: `<…@example.com>`), but the name
        // keeps up to nine twentieths of the room, so a narrow row still says who.
        let room = (who_right - who_x).max(1.0);
        let email = m.from.email.trim();
        let more = if count > 1 { format!("  ({count})") } else { String::new() };
        let font = theme::regular(12.5);
        let name_w = p.layout_no_wrap(who.to_string(), who_font.clone(), t.text).size().x;
        let cap = (room - name_w.min(room * 0.45) - 6.0).clamp(1.0, room * 0.7);
        let full = format!("<{email}>{more}");
        let fits = p.layout_no_wrap(full.clone(), font.clone(), t.dim).size().x <= cap;
        let address = match email.rsplit_once('@') {
            Some((_, domain)) if !fits => format!("<…@{domain}>{more}"),
            _ => full,
        };
        let ag = widgets::one_line(ui, &address, font, t.dim, cap);
        let aw = ag.size().x;
        let who_g = widgets::one_line(ui, &who, who_font, t.text, (room - aw - 6.0).max(1.0));
        let nw = who_g.size().x;
        p.galley(egui::pos2(mr.start(who_x, nw), line1), who_g, t.text);
        p.galley(egui::pos2(mr.start(who_x + nw + 6.0, aw), line1 + 1.5), ag, t.dim);
    } else {
        let who_g = widgets::one_line(ui, &who, who_font, t.text, who_right - who_x);
        p.galley(egui::pos2(place(who_rtl, who_x, who_right, who_g.size().x), line1), who_g, t.text);
    }

    // Subject with markers.
    let line2 = line1 + 19.0;
    let mut icons_w = 0.0;
    let marker = |ic: Icon, color: Color32, w: &mut f32| {
        *w += 18.0;
        let ir = mr.rect(Rect::from_min_size(egui::pos2(right - *w + 3.0, line2 + 1.0), vec2(14.0, 14.0)));
        icons::paint(&p, ir, ic, color);
    };
    if !hovered {
        if m.flags.pinned() {
            marker(Icon::Pin, t.accent, &mut icons_w);
        }
        if m.flags.flagged {
            marker(Icon::Flag, t.flag, &mut icons_w);
        } else if m.flags.follow_up_complete() {
            marker(Icon::Done, t.dim, &mut icons_w);
        }
        if m.has_attachments {
            marker(Icon::Attach, t.dim, &mut icons_w);
        }
        // S/MIME, as the header says (checked once it's opened).
        match m.smime {
            mailcraft_engine::model::Smime::Encrypted => marker(Icon::Lock, t.dim, &mut icons_w),
            mailcraft_engine::model::Smime::Signed => marker(Icon::Seal, t.dim, &mut icons_w),
            mailcraft_engine::model::Smime::None => {}
        }
        // Importance the sender set: a red ! for high, a ↓ for low.
        if m.importance != 0 {
            icons_w += 12.0;
            let (mark, color) = if m.importance > 0 { ("!", t.danger) } else { ("↓", t.accent) };
            p.text(mr.pos(egui::pos2(right - icons_w + 6.0, line2 + 8.0)), egui::Align2::CENTER_CENTER, mark, theme::bold(13.0), color);
        }
        if m.flags.answered {
            marker(Icon::Reply, t.faint, &mut icons_w);
        } else if m.flags.forwarded {
            marker(Icon::Forward, t.faint, &mut icons_w);
        }
    }
    // Categories: a coloured dot each, before the markers.
    if !hovered {
        for c in s.categories_of(m).into_iter().rev().take(4) {
            icons_w += 13.0;
            p.circle_filled(mr.pos(egui::pos2(right - icons_w + 7.0, line2 + 8.0)), 5.0, theme::category_color(c.color, t.dark));
        }
    }
    let subject = if m.subject.is_empty() { tl!("(no subject)").to_string() } else { m.subject.clone() };
    let subj_color = if unread { t.accent } else { t.text };
    let (subject, subj_rtl) = crate::bidi::shown(&subject);
    let subj_right = right - icons_w.max(if hovered { 70.0 } else { 0.0 }) - 4.0;
    let subj_g = widgets::one_line(ui, &subject, if unread { theme::bold(13.0) } else { theme::regular(13.0) }, subj_color, subj_right - x);
    p.galley(egui::pos2(place(subj_rtl, x, subj_right, subj_g.size().x), line2), subj_g, subj_color);
    if s.prefs.show_preview && !compact && !m.preview.is_empty() {
        let (preview, pv_rtl) = crate::bidi::shown(&m.preview);
        let pv = widgets::one_line(ui, &preview, theme::regular(12.5), t.dim, right - x);
        p.galley(egui::pos2(place(pv_rtl, x, right, pv.size().x), line2 + 19.0), pv, t.dim);
    }

    // Hover actions.
    let mut clicked_action = false;
    if hovered {
        let mut bx = right - 22.0;
        let actions: [(Icon, &str, &'static str, serde_json::Value); 3] = [
            (Icon::Delete, tl!("Delete"), "mail.delete", json!({})),
            (Icon::Flag, tl!(if m.flags.flagged { "Unflag" } else { "Flag" }), "mail.flag", json!({})),
            (
                if m.flags.seen { Icon::Unread } else { Icon::Read },
                tl!(if m.flags.seen { "Mark unread" } else { "Mark read" }),
                "mail.toggleRead",
                json!({}),
            ),
        ];
        for (ic, tip, cmd, params) in actions {
            let br = mr.rect(Rect::from_min_size(egui::pos2(bx, line2 - 3.0), vec2(22.0, 22.0)));
            let bresp = ui.interact(br, id.with(cmd), Sense::click()).on_hover_text(tip);
            crate::widgets::named(ui, &bresp, tip, true, None);
            if bresp.hovered() {
                ui.painter().rect_filled(br, 4.0, t.border);
            }
            let color = if cmd == "mail.flag" && m.flags.flagged { t.flag } else { t.dim };
            icons::paint(ui.painter(), icons::centered(br, 14.0), ic, color);
            if bresp.clicked() {
                clicked_action = true;
                if !selected {
                    acts.push(Action::Run("list.select", json!({"key": m.key.to_id()})));
                }
                acts.push(Action::Run(cmd, params));
            }
            bx -= 24.0;
        }
    }

    if resp.clicked() && !clicked_action {
        let mods = ui.input(|i| i.modifiers);
        // Shift: from the anchor to here (with Ctrl, added to what's selected); Ctrl: this one in
        // or out (issue #26).
        if mods.shift {
            acts.push(Action::Run("list.selectRange", json!({"key": m.key.to_id(), "add": mods.command || mods.ctrl})));
        } else if mods.command || mods.ctrl {
            acts.push(Action::Run("list.select", json!({"key": m.key.to_id(), "add": true})));
        } else {
            acts.push(Action::Open(m.key.clone()));
        }
    }
    if resp.double_clicked() {
        // Drafts carry on being written; other mail opens in a window of its own.
        if s.view_role() == Some(FolderRole::Drafts) || m.flags.draft {
            acts.push(Action::Run("draft.edit", json!({"key": m.key.to_id()})));
        } else {
            acts.push(Action::Run("message.popOut", json!({"key": m.key.to_id()})));
        }
    }
    if resp.drag_started() {
        if !selected {
            acts.push(Action::Run("list.select", json!({"key": m.key.to_id()})));
        }
        let keys = if selected { s.selection.clone() } else { vec![m.key.clone()] };
        resp.dnd_set_drag_payload(DragKeys(keys));
    }
    if resp.dragged() {
        let n = if selected { s.selection.len() } else { 1 };
        egui::Tooltip::always_open(ui.ctx().clone(), ui.layer_id(), id.with("drag"), egui::PopupAnchor::Pointer).show(|ui| {
            ui.label(if n == 1 { tl!("Move 1 message").to_string() } else { tf!("Move {n} messages", n = n) });
        });
    }
    let key = m.key.clone();
    let flags = m.flags.clone();
    let (subject, sender) = (m.subject.clone(), m.from.email.clone());
    resp.context_menu(|ui| {
        if !s.selection.contains(&key) {
            acts.push(Action::Run("list.select", json!({"key": key.to_id()})));
        }
        let item = |ui: &mut Ui, acts: &mut Vec<Action>, label: &str, act: Action| {
            if ui.button(label).clicked() {
                acts.push(act);
                ui.close();
            }
        };
        item(ui, acts, tl!("Open in new window"), Action::Run("message.popOut", json!({"key": key.to_id()})));
        ui.separator();
        // One of several selected: answering and printing are for one message (they failed, or
        // asked where to save a PDF and then said nothing is open).
        let several = selected && s.selection.len() > 1;
        for (label, act) in [
            (tl!("Reply"), Action::Run("mail.reply", json!({}))),
            (tl!("Reply all"), Action::Run("mail.replyAll", json!({}))),
            (tl!("Forward"), Action::Run("mail.forward", json!({}))),
            (tl!("Quick print"), Action::Print),
        ] {
            if ui.add_enabled(!several, egui::Button::new(label)).clicked() {
                acts.push(act);
                ui.close();
            }
        }
        ui.separator();
        item(
            ui,
            acts,
            tl!(if flags.seen { "Mark as unread" } else { "Mark as read" }),
            Action::Run(if flags.seen { "mail.markUnread" } else { "mail.markRead" }, json!({})),
        );
        ui.menu_button(tl!("Follow up"), |ui| crate::organize::follow_up_menu(ui, s, t, acts));
        ui.separator();
        item(ui, acts, tl!("Move to…"), Action::Dialog(Dialog::move_to()));
        item(ui, acts, tl!("Archive"), Action::Run("mail.archive", json!({})));
        item(ui, acts, tl!("Report junk"), Action::Run("mail.junk", json!({})));
        ui.menu_button(tl!("Junk"), |ui| crate::junk_menu(ui, s, acts));
        item(ui, acts, tl!("Ignore"), Action::Run("mail.ignore", json!({})));
        item(ui, acts, tl!("Delete"), Action::Run("mail.delete", json!({})));
        ui.separator();
        // Find related, as Outlook's: the conversation, or everything from the sender.
        ui.menu_button(tl!("Find related"), |ui| {
            let topic = mailcraft_engine::model::thread::base_subject(&subject);
            let searches = [
                ("Messages in this conversation", format!("subject:\"{}\"", topic.replace('"', ""))),
                ("Messages from sender", format!("from:{sender}")),
            ];
            for (label, query) in searches {
                if ui.button(tl!(label)).clicked() {
                    acts.push(Action::Search(query));
                    ui.close();
                }
            }
        });
        crate::organize::context_items(ui, s, t, acts, flags.pinned());
        ui.separator();
        item(ui, acts, tl!("View message source"), Action::Run("message.source", json!({"key": key.to_id()})));
    });
    // Reply actions need the message open; not one of several selected, which the menu acts on
    // all of (opening it selected it alone: Delete took only that one).
    let one_of_several = selected && s.selection.len() > 1;
    if resp.secondary_clicked() && !one_of_several && !s.reading.as_ref().is_some_and(|r| r.key == m.key) {
        acts.push(Action::Open(m.key.clone()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The owner's report: in a narrow list "Has files", then "Flagged", wrapped onto a second
    /// line. Those that don't fit go into the "⋯" menu instead, in order; the chosen one stays.
    #[test]
    fn filter_tabs_that_dont_fit_go_into_a_menu() {
        let all = ListFilter::ALL.to_vec();
        let width = |_: ListFilter| 60.0;
        assert_eq!(fit_tabs(&all, ListFilter::All, width, 1000.0, 4.0), (all.clone(), Vec::new()));
        let (kept, hidden) = fit_tabs(&all, ListFilter::All, width, 160.0, 4.0);
        assert_eq!(kept.len() + hidden.len(), all.len());
        assert!(kept.first() == Some(&ListFilter::All) && !hidden.is_empty());
        let used: f32 = kept.iter().map(|_| 64.0).sum::<f32>() + MORE_W;
        assert!(used <= 160.0 + 4.0, "{kept:?}");
        // The chosen one, last in order, stays out of the menu; the others keep their order.
        let last = *all.last().unwrap();
        let (kept, hidden) = fit_tabs(&all, last, width, 160.0, 4.0);
        assert!(kept.contains(&last) && !hidden.contains(&last), "{kept:?} {hidden:?}");
        let order: Vec<ListFilter> = all.iter().copied().filter(|f| hidden.contains(f)).collect();
        assert_eq!(hidden, order);
        // No room at all: the chosen one and the menu.
        let (kept, hidden) = fit_tabs(&all, ListFilter::All, width, 10.0, 4.0);
        assert_eq!((kept, hidden.len()), (vec![ListFilter::All], all.len() - 1));
    }
}
