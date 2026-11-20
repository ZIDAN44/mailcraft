//! The message list: date headings, rows with avatar, sender, subject and preview, unread and
//! flag markers, hover actions, multi-select (Ctrl/Shift), drag to a folder, right-click menu.
//! Only visible rows are laid out, so long folders scroll smoothly.

use egui::{Color32, Rect, RichText, Sense, Ui, pos2, vec2};
use mailcraft_engine::model::search::Problem;
use mailcraft_engine::model::{FolderRole, Summary, addr, dates};
use mailcraft_engine::{ListFilter, Row, SearchScope, Session};
use serde_json::json;

use crate::icons::{self, Icon};
use crate::progress;
use crate::theme;
use crate::widgets;
use crate::{Action, Dialog, UiState};

mod row;

use row::{RowOf, message_row};

/// The tops of the list's rows (date headings and messages), worked out again only when the
/// engine builds its rows again ([`Session::nav_structure`]: the list changed, the day turned) or
/// the row height changes. The rows themselves are the engine's, borrowed each frame: a big
/// folder's hundreds of thousands of rows were made again every frame, then each time it changed.
pub(crate) struct ListRows {
    key: (u64, u32, u32),
    tops: Vec<f32>,
    total: f32,
}

/// What a dragged row carries: the selection goes where it's dropped (the row is selected as
/// the drag starts).
pub struct DragKeys;

const HEADER_H: f32 = 30.0;

fn row_height(s: &Session) -> f32 {
    // One line a message, whatever else is set: no avatar, no second line, no preview.
    if s.prefs.single_line {
        return 28.0;
    }
    match (s.prefs.compact, s.prefs.show_preview) {
        (true, _) => 46.0,
        (false, true) => 76.0,
        (false, false) => 58.0,
    }
}

pub fn show(s: &Session, st: &mut UiState, ui: &mut Ui, acts: &mut Vec<Action>) {
    let t = st.tokens;
    // The single-line list's head is one line too, close to its rows.
    let (top, bottom) = if s.prefs.single_line { (5, 4) } else { (10, 6) };
    egui::Frame::new().inner_margin(egui::Margin { left: 14, right: 10, top, bottom }).show(ui, |ui| {
        // Against the right in a right-to-left interface (egui runs its rows from there too).
        ui.with_layout(crate::bidi::column(), |ui| list_head(s, &t, ui, acts));
    });
    list_body(s, st, ui, acts);
}

/// The list's name, its Focused and Other tabs, its filters and how it's arranged.
fn list_head(s: &Session, t: &theme::Tokens, ui: &mut Ui, acts: &mut Vec<Action>) {
    if s.prefs.single_line {
        list_head_line(s, t, ui, acts);
        search_status(s, t, ui);
        return;
    }
    ui.horizontal(|ui| {
        ui.label(RichText::new(crate::view_title(s)).font(theme::bold(18.0)).color(t.text));
    });
    ui.add_space(4.0);
    if s.focus_split() {
        ui.horizontal(|ui| focus_tabs(s, t, ui, 14.0, [false, true], acts));
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

/// The Inbox's Focused and Other (`others` in the order they're laid out), the one shown
/// underlined; text `size` points.
fn focus_tabs(s: &Session, t: &theme::Tokens, ui: &mut Ui, size: f32, others: [bool; 2], acts: &mut Vec<Action>) {
    for other in others {
        let label = if other { "Other" } else { "Focused" };
        let on = s.focus_other == other;
        let text = RichText::new(tl!(label)).color(if on { t.text } else { t.dim }).font(if on { theme::bold(size) } else { theme::regular(size) });
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
}

/// The single-line list's head, on one line: the list's name, then against the other side its
/// filter in a menu (named by the one chosen), where a search looks or how the list is arranged.
fn list_head_line(s: &Session, t: &theme::Tokens, ui: &mut Ui, acts: &mut Vec<Action>) {
    ui.horizontal(|ui| {
        ui.with_layout(crate::bidi::row_from_end(egui::Align::Center), |ui| {
            if s.search.is_none() {
                let rev = s.prefs.reverse_sort;
                if ui
                    .add(egui::Button::new(RichText::new(if rev { "↓" } else { "↑" }).color(t.dim)).frame_when_inactive(false))
                    .on_hover_text(tl!("Reverse sort"))
                    .clicked()
                {
                    acts.push(Action::Run("view.reverseSort", json!({})));
                }
                let arrange = tf!("By {field} ⏷", field = tl!(s.prefs.arrange.label()));
                let r = ui.add(egui::Button::new(RichText::new(arrange).color(t.dim).small()).frame_when_inactive(false));
                egui::Popup::menu(&r).show(|ui| arrange_menu(ui, s, acts));
            } else if let Some((q, current)) = s.search.as_ref().filter(|_| s.open_search_folder_name().is_none()) {
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
            // A filter chosen stands out, as its tab's underline does.
            let chosen = s.filter != ListFilter::All;
            let text = RichText::new(format!("{} ⏷", tl!(s.filter.label()))).color(if chosen { t.accent } else { t.dim }).small();
            let r = ui.add(egui::Button::new(text).frame_when_inactive(false));
            crate::widgets::named(ui, &r, tl!("Filter"), true, None);
            egui::Popup::menu(&r).show(|ui| {
                for f in ListFilter::ALL.into_iter().chain((!ListFilter::ALL.contains(&s.filter)).then_some(s.filter)) {
                    if ui.selectable_label(s.filter == f, tl!(f.label())).clicked() {
                        acts.push(Action::Run("list.filter", json!({"value": f.code()})));
                        ui.close();
                    }
                }
            });
            // The Inbox's Focused and Other, before the filter (laid out from the end: Other first).
            if s.focus_split() {
                focus_tabs(s, t, ui, 12.5, [true, false], acts);
            }
            // The name has what's left, cut short where it's too long.
            ui.with_layout(crate::bidi::row_from_start(egui::Align::Center), |ui| {
                ui.add(egui::Label::new(RichText::new(crate::view_title(s)).font(theme::bold(13.5)).color(t.text)).truncate());
            });
        });
    });
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
                Problem::Value(value) => tf!("Couldn't read “{value}”", value = value),
                Problem::TooLong => tl!("Part of a long search was left out").to_string(),
            };
            ui.label(RichText::new(text).color(t.warn).small());
        }
    });
}

/// A heading of the list as drawn: its text shown, its text as the list names it (English for a
/// fixed one), and when folded, how much mail it hides (and how much of that is unread).
struct Heading<'a> {
    shown: &'a str,
    heading: &'a str,
    folded: Option<(usize, usize)>,
}

/// A heading of the list (a date, a sender…), as Outlook's groups (issue #189): an arrow at its
/// start, down while its mail shows; a click folds it away, the arrow then pointing along the
/// text and the heading saying how much it hides; clicked again, it shows. Its menu folds or
/// shows every heading.
fn group_heading(t: &crate::theme::Tokens, ui: &mut Ui, r: Rect, h: Heading<'_>, acts: &mut Vec<Action>) {
    let Heading { shown, heading, folded } = h;
    let resp = ui.interact(r, ui.scope_id().with(("list-heading", heading)), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(r, 0.0, t.hover_row);
    }
    let m = crate::bidi::Mirror::new(r);
    let arrow = Rect::from_center_size(m.pos(r.left_bottom() + vec2(19.0, -14.0)), vec2(10.0, 10.0));
    icons::paint(ui.painter(), arrow, if folded.is_some() { m.closed() } else { Icon::ChevronDown }, t.dim);
    let text = ui.painter().text(m.pos(r.left_bottom() + vec2(30.0, -8.0)), m.align(egui::Align2::LEFT_BOTTOM), shown, theme::bold(12.5), t.dim);
    let name = match folded {
        Some((count, unread)) => {
            // How much it hides, in the accent when some of it is unread.
            let at = if m.on() { text.min.x - 6.0 } else { text.max.x + 6.0 };
            let colour = if unread > 0 { t.accent } else { t.faint };
            ui.painter().text(pos2(at, text.max.y), m.align(egui::Align2::LEFT_BOTTOM), format!("({count})"), theme::regular(12.0), colour);
            tf!("{heading} ({count}), folded", heading = shown, count = count)
        }
        None => shown.to_string(),
    };
    crate::widgets::named(ui, &resp, &name, true, None);
    let fold = |acts: &mut Vec<Action>, folded: bool| acts.push(Action::Run("list.foldGroup", json!({"heading": heading, "folded": folded})));
    resp.context_menu(|ui| {
        let (label, to) = if folded.is_some() { ("Expand this group", false) } else { ("Collapse this group", true) };
        if ui.button(tl!(label)).clicked() {
            fold(acts, to);
            ui.close();
        }
        ui.separator();
        if ui.button(tl!("Collapse all groups")).clicked() {
            acts.push(Action::Run("list.collapseAll", json!({})));
            ui.close();
        }
        if ui.button(tl!("Expand all groups")).clicked() {
            acts.push(Action::Run("list.expandAll", json!({})));
            ui.close();
        }
    });
    let tip = if folded.is_some() { tl!("Show this group's mail") } else { tl!("Hide this group's mail") };
    if resp.on_hover_text(tip).clicked() {
        fold(acts, folded.is_none());
    }
}

/// The list's rows, under its head.
fn list_body(s: &Session, st: &mut UiState, ui: &mut Ui, acts: &mut Vec<Action>) {
    let t = st.tokens;
    let sep = ui.available_rect_before_wrap();
    ui.painter().line_segment([sep.left_top(), sep.right_top()], egui::Stroke::new(1.0, t.border));

    let mh = row_height(s);
    let rows = s.rows();
    // Not shown in groups: the headings take no room and aren't drawn.
    let header_h = if s.prefs.groups { HEADER_H } else { 0.0 };
    let key = (s.nav_structure(), mh.to_bits(), header_h.to_bits());
    if st.list_rows.as_ref().is_none_or(|c| c.key != key || c.tops.len() != rows.len()) {
        let mut tops = Vec::with_capacity(rows.len());
        let mut y = 0.0;
        for r in rows {
            tops.push(y);
            y += match r {
                Row::Header(_) | Row::Group(_) => header_h,
                // A folded heading's mail (issue #189): counted on the heading, taking no room.
                Row::Folded { .. } => 0.0,
                Row::Message { .. } => mh,
            };
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
        // A search over several folders, or the unified Inbox: each row says where it is.
        let several_folders =
            s.search.as_ref().is_some_and(|(_, scope)| *scope != SearchScope::Current) || matches!(s.view, mailcraft_engine::View::Unified { .. });
        // Tab stops on one row: the list's focus, else the first message.
        let tab_stop = s.focus.as_ref().filter(|k| s.row_index_of(k).is_some()).or_else(|| {
            // The first message shown (the list's first may be under a folded heading).
            rows.iter().find_map(|r| match r {
                Row::Message { index, .. } => s.list.get(*index).map(|m| &m.key),
                _ => None,
            })
        });
        // The row with the keyboard, and whether it's still here.
        let keyboard = st.row_keyboard.filter(|id| ui.memory(|m| m.focused()) == Some(*id));
        st.list_stop = None;
        let mut keyboard_drawn = false;
        let first = tops.partition_point(|&top| top + mh < viewport.min.y);
        for (i, row) in rows.iter().enumerate().skip(first) {
            let top = tops.get(i).copied().unwrap_or(0.0);
            if top > viewport.max.y {
                break;
            }
            match row {
                Row::Header(_) | Row::Group(_) | Row::Folded { .. } if !s.prefs.groups => {}
                Row::Folded { .. } => {}
                Row::Header(_) | Row::Group(_) => {
                    let (shown, heading) = match row {
                        Row::Header(label) => (tl!(label).to_string(), label.to_string()),
                        Row::Group(name) => (name.clone(), name.clone()),
                        _ => continue,
                    };
                    let folded = match rows.get(i + 1) {
                        Some(Row::Folded { count, unread }) => Some((*count, *unread)),
                        _ => None,
                    };
                    let r = Rect::from_min_size(origin + vec2(0.0, top), vec2(width, HEADER_H));
                    group_heading(&t, ui, r, Heading { shown: &shown, heading: &heading, folded }, acts);
                }
                Row::Message { index, count, unread } => {
                    let Some(m) = s.list.get(*index) else { continue };
                    let r = Rect::from_min_size(origin + vec2(0.0, top), vec2(width, mh));
                    let drafting = drafted.marks(m);
                    let of = RowOf {
                        m,
                        r,
                        count: *count,
                        unread_in_thread: *unread,
                        sent_view,
                        drafting,
                        several_folders,
                        tab_stop: tab_stop == Some(&m.key),
                    };
                    let resp = message_row(s, &t, ui, of, acts);
                    if tab_stop == Some(&m.key) {
                        st.list_stop = Some(resp.id);
                    }
                    row_keyboard(s, st, ui, &m.key, &resp);
                    keyboard_drawn |= keyboard == Some(resp.id);
                }
            }
        }
        // The row with the keyboard gone (its message deleted, moved, archived): the message the
        // list's focus went on to takes it, read as the next is in Outlook, not the window.
        if keyboard.is_some() && !keyboard_drawn {
            st.focus_row_at = Some(ui.input(|i| i.time));
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

/// How long after the list's focus moved by the keyboard its row may still take the keyboard
/// (it's drawn once it's scrolled into view).
const FOCUS_ROW_SECS: f64 = 1.0;

/// The keyboard on a message row (issue #175). The row the list's focus moved to by the keyboard
/// takes it, as a clicked row does, so a screen reader reads the message reached: the keyboard
/// stayed on the window, and nothing was read. The row keeps its arrows, which move the list's
/// focus (egui's own would also take the keyboard to the row below: two rows a press).
fn row_keyboard(s: &Session, st: &mut UiState, ui: &Ui, key: &mailcraft_engine::model::MessageKey, resp: &egui::Response) {
    let now = ui.input(|i| i.time);
    let moved = st.focus_row_at.is_some_and(|at| now - at < FOCUS_ROW_SECS) && s.focus.as_ref() == Some(key);
    if moved || resp.clicked() {
        resp.request_focus();
        // Not egui's own move for the same press as well: at the frame's end it takes the
        // keyboard on from the row to the control below (the row keeps its arrows from the next
        // frame on).
        ui.memory_mut(|m| m.move_focus(egui::FocusDirection::None));
        st.focus_row_at = None;
    }
    if resp.has_focus() {
        let arrows = egui::EventFilter { vertical_arrows: true, horizontal_arrows: true, ..Default::default() };
        ui.memory_mut(|m| m.set_focus_lock_filter(resp.id, arrows));
        st.row_keyboard = Some(resp.id);
    }
}

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
    if ui.selectable_label(s.prefs.groups, tl!("Show in groups")).clicked() {
        acts.push(Action::Run("view.groups", json!({})));
        ui.close();
    }
    // Every heading folded away, or shown (issue #189): a folder's list in groups.
    let foldable = s.prefs.groups && s.search.is_none();
    for (label, cmd) in [("Collapse all groups", "list.collapseAll"), ("Expand all groups", "list.expandAll")] {
        if ui.add_enabled(foldable, egui::Button::new(tl!(label)).frame(false)).clicked() {
            acts.push(Action::Run(cmd, json!({})));
            ui.close();
        }
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
