//! The folder pane's rows: how a folder's, a section heading's look, how tightly they sit (the
//! density), and the room the first keeps for Minimize.

use mailcraft_engine::FolderDensity;

use super::*;

/// The sizes a density gives the pane's rows (issue #184): a folder's row and a mailbox's
/// heading, the folder's name and a mailbox's, and each level's indent.
#[derive(Clone, Copy)]
pub(super) struct Sizes {
    pub(super) row: f32,
    pub(super) text: f32,
    pub(super) heading: f32,
    pub(super) indent: f32,
}

/// The density the pane is drawn in, noted as it starts.
fn density_id() -> egui::Id {
    egui::Id::unique("nav-density")
}

/// Draw the pane in `density`.
pub(super) fn set_density(ui: &Ui, density: FolderDensity) {
    ui.data_mut(|d| d.insert_temp(density_id(), density));
}

/// The sizes of the density the pane is drawn in. Spacious is as the pane always was; Compact
/// fits about as many folders as Outlook for Mac in the same height.
pub(super) fn sizes(ui: &Ui) -> Sizes {
    match ui.data(|d| d.get_temp::<FolderDensity>(density_id())).unwrap_or_default() {
        FolderDensity::Spacious => Sizes { row: 30.0, text: 13.5, heading: 13.0, indent: 14.0 },
        FolderDensity::Medium => Sizes { row: 25.0, text: 13.0, heading: 12.5, indent: 11.0 },
        FolderDensity::Compact => Sizes { row: 21.0, text: 12.5, heading: 12.0, indent: 8.0 },
    }
}

/// Room kept at the end of the pane's first row for Minimize (issue #188), as Outlook keeps its
/// ‹ at the right of the Favorites heading: it had a row of its own above the folders.
pub(super) const CORNER: f32 = 26.0;

/// Where the pane's first row starts (its top), while Minimize goes at its end.
pub(super) fn corner_top() -> egui::Id {
    egui::Id::unique("nav-corner-top")
}

/// The pane's first row, as drawn: Minimize goes at its end.
pub(super) fn corner_row() -> egui::Id {
    egui::Id::unique("nav-corner-row")
}

/// The room a row at `rect` keeps for Minimize: [`CORNER`] when it's the pane's first, else none.
/// The first is noted, for Minimize to be drawn at its end.
pub(super) fn corner_inset(ui: &Ui, rect: egui::Rect) -> f32 {
    let top: Option<f32> = ui.data(|d| d.get_temp(corner_top()));
    if top.is_some_and(|y| (rect.min.y - y).abs() < 3.0) {
        ui.data_mut(|d| {
            d.remove::<f32>(corner_top());
            d.insert_temp(corner_row(), rect);
        });
        CORNER
    } else {
        0.0
    }
}

/// A section's heading that collapses it (Favorites, Search Folders), with its arrow.
pub(super) fn section_header(ui: &mut Ui, t: &Tokens, label: &str, open: bool) -> egui::Response {
    ui.add_space(2.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::click());
    corner_inset(ui, rect);
    crate::widgets::named(ui, &resp, tl!(label), true, Some(open));
    if resp.hovered() {
        ui.painter().rect_filled(rect, 4.0, t.hover_row);
    }
    let m = crate::bidi::Mirror::new(rect);
    let chev = m.rect(egui::Rect::from_min_size(egui::pos2(rect.min.x + 2.0, rect.center().y - 6.0), vec2(12.0, 12.0)));
    icons::paint(ui.painter(), chev, if open { Icon::ChevronDown } else { m.closed() }, t.faint);
    ui.painter().text(
        m.pos(egui::pos2(rect.min.x + 18.0, rect.center().y)),
        m.align(egui::Align2::LEFT_CENTER),
        tl!(label),
        theme::bold(12.0),
        t.faint,
    );
    resp
}

/// What a folder row counts: its unread mail, bold in the accent colour, or (Drafts) the items
/// it holds, in brackets so they don't read as unread, as Outlook shows Drafts (issue #52).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Count {
    Unread(u32),
    Items(u32),
}

/// What a row is and takes: the one shown, one mail and folders are dropped on, one dragged.
#[derive(Clone, Copy, Default)]
pub(super) struct RowIs {
    pub(super) selected: bool,
    pub(super) drop_ok: bool,
    pub(super) draggable: bool,
}

impl RowIs {
    /// A row nothing is dropped on or dragged from, shown or not.
    pub(super) fn selected(selected: bool) -> RowIs {
        RowIs { selected, ..RowIs::default() }
    }
}

/// One row (its label as shown, translated); returns its response.
pub(super) fn row(ui: &mut Ui, t: &Tokens, icon: Icon, label: &str, count: Count, depth: usize, is: RowIs) -> egui::Response {
    let RowIs { selected, drop_ok, draggable } = is;
    let size = sizes(ui);
    let h = size.row;
    let unread = if let Count::Unread(n) = count { n } else { 0 };
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), h), if draggable { Sense::click_and_drag() } else { Sense::click() });
    // To a screen reader, a folder is one choice among the folders: its name, and its unread count.
    let spoken = match count {
        Count::Unread(n) if n > 0 => format!("{label}, {}", tf!("Unread: {n}", n = n)),
        Count::Items(n) if n > 0 => format!("{label}, {}", tf!("Items: {n}", n = n)),
        _ => label.to_string(),
    };
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::TreeItem, ui.is_enabled(), selected, &spoken));
    if selected {
        ui.ctx().data_mut(|d| d.insert_temp(egui::Id::unique(CURRENT_ROW), resp.id));
    }
    let hovering_drop = drop_ok && (resp.dnd_hover_payload::<DragKeys>().is_some() || resp.dnd_hover_payload::<DragFolder>().is_some());
    let ctx = ui.ctx().clone();
    let sel = crate::motion::toward(&ctx, resp.id.with("selected"), selected, crate::motion::NORMAL);
    let hover = crate::motion::toward(&ctx, resp.id.with("hover"), resp.hovered(), crate::motion::FAST);
    let fill = if hovering_drop { t.accent.gamma_multiply(0.3) } else { crate::motion::mix(t.hover_row.gamma_multiply(hover), t.accent_soft, sel) };
    ui.painter().rect_filled(rect, 6.0, fill);
    // Right to left: the icon on the right, the count on the left.
    let m = crate::bidi::Mirror::new(rect);
    if sel > 0.0 {
        let full = (h - 14.0).max(8.0);
        let bar = egui::Rect::from_center_size(m.pos(egui::pos2(rect.min.x + 1.5, rect.center().y)), vec2(3.0, full * sel));
        ui.painter().rect_filled(bar, 2.0, t.accent);
    }
    let x = rect.min.x + 10.0 + depth as f32 * size.indent;
    let ir = m.rect(egui::Rect::from_min_size(egui::pos2(x, rect.center().y - 8.0), vec2(16.0, 16.0)));
    icons::paint(ui.painter(), ir, icon, if selected { t.accent } else { t.dim });
    let font = if unread > 0 || selected { theme::bold(size.text) } else { theme::regular(size.text) };
    let counted = matches!(count, Count::Unread(n) | Count::Items(n) if n > 0);
    let inset = corner_inset(ui, rect);
    let text_right = rect.max.x - inset - if counted { 40.0 } else { 8.0 };
    // One line, cut to "…" in a narrow pane (it wrapped, two or three lines in a row of one), the
    // whole name on hover.
    let galley = crate::widgets::one_line(ui, label, font, t.text, text_right - x - 24.0);
    let resp = if galley.elided { resp.on_hover_text(label) } else { resp };
    let gy = rect.center().y - galley.size().y / 2.0;
    ui.painter().galley(egui::pos2(m.start(x + 24.0, galley.size().x), gy), galley, t.text);
    let shown = match count {
        Count::Unread(n) if n > 0 => Some((n.to_string(), theme::bold(12.5), t.accent)),
        Count::Items(n) if n > 0 => Some((format!("[{n}]"), theme::regular(12.5), t.dim)),
        _ => None,
    };
    if let Some((text, font, color)) = shown {
        ui.painter().text(m.pos(egui::pos2(rect.max.x - 10.0 - inset, rect.center().y)), m.align(egui::Align2::RIGHT_CENTER), text, font, color);
    }
    resp
}
