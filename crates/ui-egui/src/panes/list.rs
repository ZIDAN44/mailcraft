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

mod context;
mod row;
#[cfg(test)]
mod tests;

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TouchSwipeAction {
    Archive,
    MarkRead,
    MarkUnread,
}

fn touch_swipe_action(delta: egui::Vec2, was_read: bool) -> Option<TouchSwipeAction> {
    if delta.x.abs() < 88.0 || delta.x.abs() <= delta.y.abs() * 1.35 {
        return None;
    }
    Some(if delta.x < 0.0 {
        TouchSwipeAction::Archive
    } else if was_read {
        TouchSwipeAction::MarkUnread
    } else {
        TouchSwipeAction::MarkRead
    })
}

fn row_height(s: &Session, touch: bool) -> f32 {
    match (s.prefs.compact, s.prefs.show_preview) {
        (true, _) if touch => 56.0,
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
    let touch = ui.ctx().input(|input| input.has_touch_screen());
    ui.horizontal(|ui| {
        ui.label(RichText::new(crate::view_title(s)).font(theme::bold(18.0)).color(t.text));
    });
    ui.add_space(4.0);
    if s.focus_split() {
        ui.with_layout(crate::bidi::row_from_start(egui::Align::Center), |ui| {
            let gap = ui.spacing().item_spacing.x;
            let width = ((ui.available_width() - gap) / 2.0).max(44.0);
            for (other, label) in [(false, "Focused"), (true, "Other")] {
                let on = s.focus_other == other;
                let text =
                    RichText::new(tl!(label)).color(if on { t.text } else { t.dim }).font(if on { theme::bold(14.0) } else { theme::regular(14.0) });
                let button = egui::Button::new(text).frame_when_inactive(false);
                let r = if touch { ui.add_sized(vec2(width, 44.0), button) } else { ui.add(button) };
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
        let arrange_text_width = ui.painter().layout_no_wrap(arrange.clone(), egui::TextStyle::Small.resolve(ui.style()), t.dim).size().x;
        let touch_sort_width = (arrange_text_width + ui.spacing().button_padding.x * 2.0).max(44.0);
        let gap = ui.spacing().item_spacing.x;
        let right = if s.search.is_none() { if touch { touch_sort_width + 44.0 + gap * 2.0 } else { arrange_text_width + 54.0 } } else { 0.0 };
        let left_w = (ui.available_width() - right).max(120.0);
        let filter_height = if touch { 44.0 } else { 22.0 };
        let more_width = if touch { 44.0 } else { MORE_W };
        // The tabs, and a filter chosen from the Home tab's Filter menu when it's another. They
        // stay on one line: those that don't fit go into a "⋯" menu after the rest (the chosen
        // one always shows), so the head keeps its height however narrow the pane.
        let shown: Vec<ListFilter> = ListFilter::ALL.into_iter().chain((!ListFilter::ALL.contains(&s.filter)).then_some(s.filter)).collect();
        let font = |on: bool| if on { theme::bold(12.5) } else { theme::regular(12.5) };
        let pad = ui.spacing().button_padding.x * 2.0;
        let widths: Vec<(ListFilter, f32)> = shown
            .iter()
            .copied()
            .map(|f| {
                let measured = ui.painter().layout_no_wrap(tl!(f.label()).to_string(), font(s.filter == f), t.dim).size().x + pad;
                (f, measured.max(if touch { 44.0 } else { 0.0 }))
            })
            .collect();
        let width = |f: ListFilter| widths.iter().find_map(|(filter, width)| (*filter == f).then_some(*width)).unwrap_or(44.0);
        let scope = s.search.as_ref().filter(|_| s.open_search_folder_name().is_none());
        let scope_button_width = scope.map_or(0.0, |(_, current)| {
            let label = format!("{} ▾", crate::scope_label(*current));
            (ui.painter().layout_no_wrap(label, egui::TextStyle::Small.resolve(ui.style()), t.dim).size().x + pad).max(if touch { 44.0 } else { 0.0 })
        });
        let scope_w = if scope.is_some() { scope_button_width + 8.0 + gap } else { 0.0 };
        let (kept, hidden) = fit_tabs_sized(&shown, s.filter, width, left_w - scope_w, gap, more_width);
        ui.allocate_ui_with_layout(vec2(left_w, filter_height), crate::bidi::row_from_start(egui::Align::Center), |ui| {
            for f in kept {
                let on = s.filter == f;
                let text = RichText::new(tl!(f.label())).color(if on { t.accent } else { t.dim }).font(font(on));
                let button = egui::Button::new(text).frame_when_inactive(false);
                let r = ui.add_sized(vec2(width(f), filter_height), button);
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
                let r = more_tabs(ui, t, more_width, filter_height);
                crate::bidi::popup_menu(&r).show(|ui| {
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
                let label = format!("{} ▾", crate::scope_label(*current));
                let button = egui::Button::new(RichText::new(&label).small()).frame_when_inactive(false);
                let response = if touch { ui.add_sized(vec2(scope_button_width, filter_height), button) } else { ui.add(button) };
                widgets::named(ui, &response, &format!("{}: {}", tl!("Look in"), crate::scope_label(*current)), true, None);
                crate::bidi::popup_menu(&response).show(|ui| {
                    for scope in SearchScope::ALL {
                        if ui.selectable_label(scope == *current, crate::scope_label(scope)).clicked() {
                            acts.push(Action::Run("search.run", json!({"query": q, "scope": scope.code()})));
                            ui.close();
                        }
                    }
                });
                response.on_hover_text(tl!("Look in"));
            }
        });
        if s.search.is_none() {
            ui.with_layout(crate::bidi::row_from_end(egui::Align::Center), |ui| {
                let rev = s.prefs.reverse_sort;
                let reverse = egui::Button::new(RichText::new(if rev { "↓" } else { "↑" }).color(t.dim)).frame_when_inactive(false);
                let reverse = if touch { ui.add_sized(vec2(44.0, 44.0), reverse) } else { ui.add(reverse) };
                widgets::named(ui, &reverse, tl!("Reverse sort"), true, Some(rev));
                if reverse.on_hover_text(tl!("Reverse sort")).clicked() {
                    acts.push(Action::Run("view.reverseSort", json!({})));
                }
                let arrange_button = egui::Button::new(RichText::new(arrange).color(t.dim).small()).frame_when_inactive(false);
                let r = if touch { ui.add_sized(vec2(touch_sort_width, 44.0), arrange_button) } else { ui.add(arrange_button) };
                crate::bidi::popup_menu(&r).show(|ui| arrange_menu(ui, s, acts));
            });
        }
    });
    search_status(s, t, ui);
}

/// The filter tabs that fit in `room` (each `width` wide, `gap` apart) and those that go into
/// the "⋯" menu: in order, the chosen one (`on`) always kept, room left for the menu's button.
#[cfg(test)]
fn fit_tabs(shown: &[ListFilter], on: ListFilter, width: impl Fn(ListFilter) -> f32, room: f32, gap: f32) -> (Vec<ListFilter>, Vec<ListFilter>) {
    fit_tabs_sized(shown, on, width, room, gap, MORE_W)
}

fn fit_tabs_sized(
    shown: &[ListFilter],
    on: ListFilter,
    width: impl Fn(ListFilter) -> f32,
    room: f32,
    gap: f32,
    more_width: f32,
) -> (Vec<ListFilter>, Vec<ListFilter>) {
    let all: f32 = shown.iter().map(|f| width(*f) + gap).sum();
    if all - gap <= room {
        return (shown.to_vec(), Vec::new());
    }
    let mut left = room - more_width - width(on) - gap;
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
fn more_tabs(ui: &mut Ui, t: &theme::Tokens, width: f32, height: f32) -> egui::Response {
    let (rect, r) = ui.allocate_exact_size(vec2(width, height), egui::Sense::click());
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

    let touch = ui.ctx().input(|input| input.has_touch_screen());
    let mh = row_height(s, touch);
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
    let scroll = egui::ScrollArea::vertical().auto_shrink(false).id_salt("list").show_viewport(ui, |ui, viewport| {
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
                    row::show(s, &t, ui, r, m, *count, *unread, sent_view, drafting, acts);
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
    if touch && st.phone_mode {
        const PULL_TO_REFRESH: f32 = -72.0;
        let id = scroll.id.with("pull-to-refresh");
        let overscroll = scroll.state.overscroll().y;
        let can_sync = !s.prefs.work_offline
            && s.accounts.iter().any(|account| !account.is_local() && !account.is_data_file())
            && s.registry().get("mail.sync").is_some_and(|command| (command.enabled)(s).is_none());
        if overscroll < -18.0 && can_sync {
            let (label, _) = crate::bidi::shown(tl!("Send / Receive all folders"));
            let galley = ui.painter().layout_no_wrap(label.to_string(), theme::regular(12.5), t.dim);
            let group_width = galley.size().x + 30.0;
            let left = scroll.inner_rect.center().x - group_width / 2.0;
            let y = scroll.inner_rect.top() + (-overscroll).min(72.0) / 2.0;
            let mirror = crate::bidi::Mirror::new(Rect::from_min_size(pos2(left, y - 24.0), vec2(group_width, 48.0)));
            let icon_rect = mirror.rect(Rect::from_center_size(pos2(left + 10.0, y), vec2(16.0, 16.0)));
            icons::paint(ui.painter(), icon_rect, Icon::Sync, t.dim);
            ui.painter().galley(pos2(mirror.start(left + 22.0, galley.size().x), y - galley.size().y / 2.0), galley, t.dim);
        }
        let (pointer_down, pointer_released) = ui.ctx().input(|input| (input.pointer.any_down(), input.pointer.any_released()));
        let mut armed = ui.ctx().data(|data| data.get_temp::<bool>(id).unwrap_or(false));
        if pointer_down {
            if overscroll <= PULL_TO_REFRESH {
                armed = true;
            } else if overscroll > -12.0 {
                armed = false;
            }
        } else if pointer_released {
            let pulled_far_enough = overscroll <= PULL_TO_REFRESH;
            if (armed || pulled_far_enough) && can_sync {
                acts.push(Action::Run("mail.sync", json!({})));
            }
            armed = false;
        } else if overscroll > -12.0 {
            armed = false;
        }
        ui.ctx().data_mut(|data| data.insert_temp(id, armed));
    }
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
