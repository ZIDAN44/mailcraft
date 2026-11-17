//! Send / Receive's progress, as Outlook offers it (in MailCraft's own words): a line at the
//! status bar's end while it goes, where the accounts' status is ("Downloading: 10 of 158
//! (27.6 MB of 210 MB)", "Sending: 4 of 6"), and a window of its own opened from there or from
//! the ribbon (Show progress), and by Send / Receive (F9) itself when asked to: how many tasks
//! are done, a bar for them all, a line for each POP or IMAP account's sending and receiving with
//! how far it has come (what's left on hover), why any failed, and stopping them all or one. Opened by Send /
//! Receive, it closes once all is done well; a task that failed keeps it open on its problems.
//! It can be moved, and doesn't hold the window: mail can be read meanwhile.

use mailcraft_engine::TransferRow;
use mailcraft_engine::model::job::{Transfer, TransferKind, TransferState};

use egui::{Rect, Stroke};

use crate::*;

/// The window's state.
#[derive(Clone, Debug, Default)]
pub(crate) struct SendReceiveUi {
    pub(crate) open: bool,
    /// Opened by Send / Receive itself: it closes once all is done well.
    pub(crate) by_itself: bool,
}

/// The status bar's word on Send / Receive, at its end where the accounts' status is (Outlook
/// says it there, in place of "Updating"): the first task under way, then its bar, and every
/// task under way on hover; once a task failed, "Send / Receive failed" until the window has
/// shown it. Clicked, the window opens or closes. Laid from the bar's end (its bar first).
/// Whether a task under way was said: the accounts' own status then isn't.
pub(crate) fn status_segment(ui: &mut Ui, s: &Session, t: &Tokens, st: &mut SendReceiveUi) -> bool {
    let working: Vec<&TransferRow> = s.send_receive.open().filter(|r| r.transfer.state == TransferState::Working).collect();
    let clicked = if let Some(first) = working.first() {
        let each: Vec<String> = working.iter().map(|r| tf!("{account}: {status}", account = email(s, &r.account), status = going(s, r))).collect();
        let bar = progress::bar(ui, t, first.transfer.permille(), vec2(90.0, 6.0));
        ui.add_space(6.0);
        let words = ui.label(RichText::new(going(s, first)).color(t.dim).small());
        words.union(bar).interact(Sense::click()).on_hover_text(each.join("\n")).clicked()
    } else if s.send_receive.unseen_error {
        let r = ui.link(RichText::new(tl!("Send / Receive failed")).color(t.danger).small()).clicked();
        ui.add_space(10.0);
        r
    } else {
        false
    };
    if clicked {
        st.open = !st.open;
        st.by_itself = false;
    }
    !working.is_empty()
}

/// A task under way, in the status bar's words.
fn going(s: &Session, r: &TransferRow) -> String {
    let tr = &r.transfer;
    let next = tr.done.saturating_add(1).min(tr.total);
    match tr.kind {
        TransferKind::Sending if tr.total > 0 => tf!("Sending: {n} of {total}", n = next, total = tr.total),
        TransferKind::Sending => tl!("Sending…").to_string(),
        TransferKind::Receiving if tr.total > 0 && tr.bytes_total > 0 => tf!(
            "Downloading: {n} of {total} ({done} of {all})",
            n = next,
            total = tr.total,
            done = widgets::human_bytes(tr.bytes_done),
            all = widgets::human_bytes(tr.bytes_total)
        ),
        TransferKind::Receiving if tr.total > 0 => tf!("Downloading: {n} of {total}", n = next, total = tr.total),
        TransferKind::Receiving if tr.folders_total > 0 && !tr.folder.is_empty() => tf!(
            "Updating {folder} ({n} of {total})",
            folder = folder_shown(s, &r.account, &tr.folder),
            n = tr.folders_done.saturating_add(1).min(tr.folders_total),
            total = tr.folders_total
        ),
        TransferKind::Receiving => tl!("Checking for new mail…").to_string(),
    }
}

/// A folder's name as shown (`path` itself, for one the account doesn't list).
fn folder_shown(s: &Session, account: &str, path: &str) -> String {
    s.folder(account, path).map_or_else(|| path.to_string(), i18n::folder)
}

/// The address of an account, to name its tasks by.
fn email(s: &Session, account: &str) -> String {
    s.account(account).map(|a| a.config.email.clone()).unwrap_or_default()
}

/// A task's name, as the list shows it ("ana@example.org: receiving").
fn task_name(s: &Session, r: &TransferRow) -> String {
    match r.transfer.kind {
        TransferKind::Sending => tf!("{account}: sending", account = email(s, &r.account)),
        TransferKind::Receiving => tf!("{account}: receiving", account = email(s, &r.account)),
    }
}

/// A task's progress, in the list's words.
fn progress_words(s: &Session, r: &TransferRow) -> String {
    let tr = &r.transfer;
    match tr.state {
        TransferState::Waiting => tl!("Waiting").to_string(),
        TransferState::Done => tl!("Done").to_string(),
        TransferState::Failed => tl!("Failed").to_string(),
        TransferState::Cancelled => tl!("Stopped").to_string(),
        TransferState::Working if tr.total > 0 => tf!("{done} of {total}", done = tr.done, total = tr.total),
        TransferState::Working if !tr.folder.is_empty() => folder_shown(s, &r.account, &tr.folder),
        TransferState::Working => tl!("Under way").to_string(),
    }
}

/// What's left of a task under way: messages (and their size, when known), else folders.
fn remaining(tr: &Transfer) -> String {
    if tr.state != TransferState::Working {
        return String::new();
    }
    let left = tr.total.saturating_sub(tr.done);
    if left > 0 && tr.bytes_total > 0 {
        let size = widgets::human_bytes(tr.bytes_total.saturating_sub(tr.bytes_done));
        tf!("Messages: {n} ({size})", n = left, size = size)
    } else if left > 0 {
        tf!("Messages: {n}", n = left)
    } else if tr.folders_total > tr.folders_done {
        tf!("Folders: {n}", n = tr.folders_total - tr.folders_done)
    } else {
        String::new()
    }
}

/// The icon and colour that say where a task is.
fn mark(tr: &Transfer, t: &Tokens) -> (icons::Icon, Color32) {
    match tr.state {
        TransferState::Waiting => (icons::Icon::Clock, t.faint),
        TransferState::Working => (icons::Icon::Sync, t.accent),
        TransferState::Done => (icons::Icon::Done, t.ok),
        TransferState::Failed => (icons::Icon::Warning, t.danger),
        TransferState::Cancelled => (icons::Icon::Close, t.dim),
    }
}

/// An icon of the list's.
fn mark_icon(ui: &mut Ui, icon: icons::Icon, color: Color32) {
    let (r, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
    icons::paint(ui.painter(), icons::centered(r, 14.0), icon, color);
}

/// How far all the tasks have come together, in thousandths: those ended whole, those under way
/// as far as they've come.
fn overall(rows: &[TransferRow]) -> u16 {
    if rows.is_empty() {
        return 0;
    }
    let sum: u64 = rows
        .iter()
        .map(|r| match r.transfer.state {
            TransferState::Waiting => 0,
            TransferState::Working => u64::from(r.transfer.permille().unwrap_or(0)),
            _ => 1000,
        })
        .sum();
    u16::try_from(sum / rows.len() as u64).unwrap_or(1000)
}

impl MailApp {
    /// The window, when it's open: from the status bar or the ribbon, or by Send / Receive itself
    /// when the user asked for that (it then closes once all is done well).
    pub(crate) fn send_receive_window(&mut self, ctx: &egui::Context) {
        let mut st = std::mem::take(&mut self.ui.send_receive);
        // Opened by Send / Receive; one the user opened stays open, whatever it comes to.
        if std::mem::take(&mut self.session.send_receive.show_asked) && !st.open {
            (st.open, st.by_itself) = (true, true);
        }
        let rows = &self.session.send_receive.rows;
        if st.open && st.by_itself && !rows.iter().any(|r| r.transfer.state.is_open()) {
            // Done well, it closes; failed, it's kept open on why.
            st.by_itself = false;
            st.open = rows.iter().any(|r| r.transfer.state == TransferState::Failed);
        }
        if !st.open {
            self.ui.send_receive = st;
            return;
        }
        self.session.transfers_seen();
        let t = self.ui.tokens;
        let mut acts = Vec::new();
        let screen = ctx.content_rect();
        let frame = egui::Frame::new()
            .fill(t.surface)
            .corner_radius(10)
            .inner_margin(Margin::symmetric(14, 12))
            .stroke(Stroke::new(1.0, t.border))
            .shadow(egui::epaint::Shadow { offset: [0, 6], blur: 24, spread: 0, color: Color32::from_black_alpha(55) });
        // Moved where the user likes (by any of it but its controls), first in the corner the
        // status bar's line is in, above it: its line stays in sight.
        let (corner, pivot) = if i18n::current().rtl() {
            (egui::pos2(screen.left() + 24.0, screen.bottom() - 44.0), egui::Align2::LEFT_BOTTOM)
        } else {
            (egui::pos2(screen.right() - 24.0, screen.bottom() - 44.0), egui::Align2::RIGHT_BOTTOM)
        };
        let closed = egui::Window::new(tl!("Send / Receive progress"))
            .id(egui::Id::unique("send-receive-window"))
            .title_bar(false)
            .frame(frame)
            .collapsible(false)
            .resizable(false)
            .movable(true)
            .pivot(pivot)
            .default_pos(corner)
            .show(ctx, |ui: &mut Ui| draw(ui, &self.session, &t, &mut acts))
            .and_then(|r| r.inner)
            .unwrap_or(false);
        st.open &= !closed;
        self.ui.send_receive = st;
        for a in acts {
            self.act(a);
        }
    }
}

/// The window's width inside its margins.
const WIDTH: f32 = 440.0;

/// A row of the window, its things centred on one line: a centred layout given the window's
/// height took all of it (the window ran off the screen with one task in sight).
fn row<R>(ui: &mut Ui, height: f32, add: impl FnOnce(&mut Ui) -> R) -> R {
    let w = ui.available_width();
    ui.allocate_ui_with_layout(vec2(w, height), bidi::row_from_start(Align::Center), add).inner
}

/// A small icon button, a hover behind it, named for a screen reader and on hover.
fn icon_button(ui: &mut Ui, t: &Tokens, icon: icons::Icon, name: &str) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, name));
    if resp.hovered() {
        ui.painter().rect_filled(rect, 5, t.hover);
    }
    icons::paint(ui.painter(), icons::centered(rect, 12.0), icon, if resp.hovered() { t.text } else { t.dim });
    resp.on_hover_text(name)
}

/// The window's insides, compact: its title, how far it has come, a line per task, why any
/// failed, and stopping it; whether it was closed.
fn draw(ui: &mut Ui, s: &Session, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    ui.set_width(WIDTH);
    let rows = &s.send_receive.rows;
    let (done, all) = s.send_receive.completed();
    let mut closed = false;
    row(ui, 24.0, |ui| {
        // Not text to select: the window is moved by it.
        ui.add(egui::Label::new(RichText::new(tl!("Send / Receive progress")).font(theme::bold(14.0)).color(t.text)).selectable(false));
        ui.with_layout(bidi::row_from_end(Align::Center), |ui| {
            closed = icon_button(ui, t, icons::Icon::Close, tl!("Close")).clicked();
        });
    });
    let summary =
        if all == 0 { tl!("Nothing has been sent or received yet").to_string() } else { tf!("{done} of {all} done", done = done, all = all) };
    ui.add(egui::Label::new(RichText::new(summary).small().color(t.dim)).selectable(false).truncate());
    ui.add_space(6.0);
    progress::bar(ui, t, Some(overall(rows)), vec2(WIDTH, 4.0));
    ui.add_space(8.0);
    egui::ScrollArea::vertical().max_height(ROW * 8.0).auto_shrink([false, true]).show(ui, |ui| {
        for r in rows {
            task_row(ui, s, t, r, acts);
        }
    });
    problems(ui, s, t);
    ui.add_space(6.0);
    ui.painter().hline(ui.available_rect_before_wrap().x_range(), ui.cursor().top(), Stroke::new(1.0, t.border));
    ui.add_space(6.0);
    row(ui, 26.0, |ui| {
        let mut open = s.prefs.open_send_receive;
        let check = egui::Checkbox::new(&mut open, RichText::new(tl!("Open this window when Send / Receive starts")).small());
        if ui.add(check).changed() {
            acts.push(Action::Run("mail.openSyncProgress", json!({"on": open})));
        }
        ui.with_layout(bidi::row_from_end(Align::Center), |ui| {
            let going = rows.iter().any(|r| r.transfer.state.is_open());
            if ui.add_enabled(going, egui::Button::new(RichText::new(tl!("Cancel all")).small())).clicked() {
                acts.push(Action::Run("mail.cancelSync", json!({})));
            }
        });
    });
    closed
}

/// A task's line's height, and its columns: the mark, which way, and how far (the account takes
/// the rest), and stopping it.
const ROW: f32 = 26.0;
const MARK: f32 = 22.0;
const KIND: f32 = 72.0;
const STATUS: f32 = 128.0;
const STOP: f32 = 24.0;

/// A rectangle `w` wide at `x` from the row's start (mirrored in a right-to-left interface).
fn column(row: Rect, x: f32, w: f32) -> Rect {
    bidi::Mirror::new(row).rect(Rect::from_min_size(egui::pos2(row.left() + x, row.top()), vec2(w, row.height())))
}

/// `add`ed in `col`, centred on the row's line.
fn in_column<R>(ui: &mut Ui, col: Rect, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.scope_builder(egui::UiBuilder::new().max_rect(col).layout(bidi::row_from_start(Align::Center)), add).inner
}

/// One task: its mark, account, which way, how far (a small bar while it goes, what's left on
/// hover), and, while it's waiting or under way, a button that stops it alone.
fn task_row(ui: &mut Ui, s: &Session, t: &Tokens, r: &TransferRow, acts: &mut Vec<Action>) {
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(w, ROW), Sense::hover());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 5, t.hover_row);
    }
    let tr = &r.transfer;
    let (icon, color) = mark(tr, t);
    icons::paint(ui.painter(), icons::centered(column(rect, 0.0, MARK), 13.0), icon, color);
    let account_w = (w - MARK - KIND - STATUS - STOP).max(60.0);
    let label = |text: RichText| egui::Label::new(text).selectable(false).truncate();
    in_column(ui, column(rect, MARK, account_w), |ui| ui.add(label(RichText::new(email(s, &r.account)).color(t.text))));
    let way = match tr.kind {
        TransferKind::Sending => tl!("Sending"),
        TransferKind::Receiving => tl!("Receiving"),
    };
    in_column(ui, column(rect, MARK + account_w + 6.0, KIND - 6.0), |ui| ui.add(label(RichText::new(way).small().color(t.dim))));
    in_column(ui, column(rect, MARK + account_w + KIND, STATUS), |ui| {
        if tr.state == TransferState::Working {
            progress::bar(ui, t, tr.permille(), vec2(34.0, 4.0));
            ui.add_space(6.0);
        }
        let color = match tr.state {
            TransferState::Failed => t.danger,
            TransferState::Waiting | TransferState::Cancelled => t.dim,
            _ => t.text,
        };
        ui.add(label(RichText::new(progress_words(s, r)).small().color(color)));
    });
    if tr.state.is_open() {
        in_column(ui, column(rect, w - STOP, STOP), |ui| {
            if icon_button(ui, t, icons::Icon::Close, tl!("Stop")).clicked() {
                acts.push(Action::Run("mail.cancelSync", json!({"account": r.account})));
            }
        });
    }
    // What's left, and why it failed, on hover.
    let left = remaining(tr);
    let hover = match &tr.error {
        Some(e) => tf!("{task} failed: {error}", task = task_name(s, r), error = tl!(e)),
        None if !left.is_empty() => format!("{}\n{left}", task_name(s, r)),
        None => task_name(s, r),
    };
    resp.on_hover_text(hover);
}

/// Why each task that failed did, under the tasks: only when one has.
fn problems(ui: &mut Ui, s: &Session, t: &Tokens) {
    for r in s.send_receive.errors() {
        let why = r.transfer.error.as_deref().map(|e| tl!(e).to_string()).unwrap_or_default();
        ui.add_space(4.0);
        egui::Frame::new().fill(t.bg).corner_radius(6).inner_margin(Margin::symmetric(8, 6)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.with_layout(bidi::row_from_start(Align::Min), |ui| {
                mark_icon(ui, icons::Icon::Warning, t.danger);
                ui.add(
                    egui::Label::new(RichText::new(tf!("{task} failed: {error}", task = task_name(s, r), error = why)).small().color(t.text)).wrap(),
                );
            });
        });
    }
}
