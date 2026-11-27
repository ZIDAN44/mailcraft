//! Small shared widgets: toolbar buttons, avatars; dialogs' buttons, form fields and the welcome
//! screen in parts of their own.

use egui::{Color32, CornerRadius, Response, RichText, Sense, Stroke, Ui, vec2};

use crate::icons::{self, Icon};
use crate::theme::{self, Tokens};
use crate::{Action, Dialog};

mod dialog_buttons;
mod form_fields;
mod row_actions;
mod welcome;

pub use dialog_buttons::*;
pub(crate) use form_fields::{add_row, checkbox_wrapped, field_and_button, line_edit, password_field};
pub(crate) use row_actions::row_actions;
pub use welcome::welcome;

/// The ribbon's buttons as it's drawn: those from `icons_from` on are icons alone, their labels
/// tooltips (it didn't fit the window); how wide each label is, and how far right (and left, for
/// a right-to-left ribbon) they reached.
#[derive(Clone, Debug, Default)]
pub struct RibbonFit {
    pub active: bool,
    /// The classic ribbon: big icons over their labels, every label shown (it scrolls when it
    /// doesn't fit), groups named underneath.
    pub classic: bool,
    /// Where the group being drawn started (classic: its name is centred under it).
    pub group_from: f32,
    pub icons_from: usize,
    pub count: usize,
    pub labels: Vec<f32>,
    pub right: f32,
    pub left: f32,
}

impl RibbonFit {
    /// The first button to draw without its label so that the ribbon fits `room` (laid out from
    /// its right edge when `rtl`): labels go from the end, as few as it takes.
    pub fn fit(&self, room: egui::Rect, rtl: bool) -> usize {
        let hidden: f32 = self.labels.iter().skip(self.icons_from).sum();
        let used = if rtl { room.right() - self.left } else { self.right - room.left() };
        let need = used + hidden - room.width();
        let mut from = self.labels.len();
        let mut saved = 0.0;
        while need > 0.5 && saved < need && from > 0 {
            from -= 1;
            saved += self.labels.get(from).copied().unwrap_or(0.0);
        }
        from
    }
}

pub fn ribbon_fit_id() -> egui::Id {
    egui::Id::unique("ribbon-fit")
}

/// A toolbar of a window of its own, fitted to it as the ribbon is to the main window: its last
/// buttons lose their labels, as few as it takes (they ran out past the window's edge, cut).
/// `id` keeps how many did, from one frame to the next.
pub fn fitted_toolbar(ui: &mut Ui, id: egui::Id, draw: impl FnOnce(&mut Ui)) {
    let room = ui.max_rect();
    let icons_from: usize = ui.ctx().data(|d| d.get_temp(id)).unwrap_or(usize::MAX);
    let start = RibbonFit { active: true, icons_from, right: room.left(), left: room.right(), ..Default::default() };
    // The main window's ribbon may be between frames of its own: its notes are put back.
    let before = ui.ctx().data_mut(|d| {
        let before = d.get_temp::<RibbonFit>(ribbon_fit_id());
        d.insert_temp(ribbon_fit_id(), start);
        before
    });
    draw(ui);
    let mut fit = ui.ctx().data_mut(|d| std::mem::replace(d.get_temp_mut_or_default::<RibbonFit>(ribbon_fit_id()), before.unwrap_or_default()));
    // What isn't a labelled button (the ⋯ at its end) takes room too.
    fit.right = fit.right.max(ui.min_rect().right());
    fit.left = fit.left.min(ui.min_rect().left());
    let from = fit.fit(room, crate::i18n::current().rtl());
    if from != icons_from.min(fit.count) {
        ui.ctx().data_mut(|d| d.insert_temp(id, from));
        ui.ctx().request_repaint();
    }
}

/// While the ribbon is drawn: note a button (its label's width) and whether it goes without it.
fn ribbon_button(ui: &Ui, label_width: f32) -> bool {
    ui.ctx().data_mut(|d| {
        let f = d.get_temp_mut_or_default::<RibbonFit>(ribbon_fit_id());
        if !f.active {
            return false;
        }
        let i = f.count;
        f.count += 1;
        f.labels.push(label_width + 4.0);
        !f.classic && i >= f.icons_from
    })
}

/// Whether the ribbon being drawn is the classic one (read in place: a copy would take every
/// label width noted so far with it, for every button).
fn classic(ui: &Ui) -> bool {
    ui.ctx().data_mut(|d| {
        let f = d.get_temp_mut_or_default::<RibbonFit>(ribbon_fit_id());
        f.active && f.classic
    })
}

/// A button filled in the app colour: faint when it can't be used, lighter under the pointer.
fn accent_fill(t: &Tokens, enabled: bool, hovered: bool) -> Color32 {
    if !enabled {
        t.faint
    } else if hovered {
        t.accent.gamma_multiply(0.88)
    } else {
        t.accent
    }
}

/// The classic ribbon's buttons: this tall, the group names under them.
pub const CLASSIC_BUTTON_H: f32 = 60.0;
pub const CLASSIC_CAPTION_H: f32 = 22.0;

/// A classic ribbon button: a big icon over its label, as wide as the label needs (a short one
/// still a comfortable target). `fill`: the filled New button.
fn big_button(ui: &mut Ui, icon: Icon, label: &str, t: &Tokens, enabled: bool, on: bool, fill: bool) -> Response {
    let font = if fill { theme::semibold(crate::sizes::CAPTION) } else { theme::regular(crate::sizes::CAPTION) };
    let text = if fill { t.on_accent } else { command_label(t, enabled) };
    let galley = ui.painter().layout_no_wrap(label.to_string(), font, text);
    let size = vec2((galley.size().x + 16.0).max(54.0), CLASSIC_BUTTON_H);
    let (rect, resp) = ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
    reached(ui, rect);
    if fill {
        ui.painter().rect_filled(rect, f32::from(crate::sizes::CONTROL_RADIUS), accent_fill(t, enabled, resp.hovered()));
    } else if on {
        ui.painter().rect(
            rect,
            f32::from(crate::sizes::CONTROL_RADIUS),
            t.accent_soft,
            Stroke::new(1.0, t.accent.gamma_multiply(0.5)),
            egui::StrokeKind::Inside,
        );
    } else {
        hover_fill(ui, &resp, rect, t, enabled);
    }
    let icon_color = if fill {
        t.on_accent
    } else if enabled {
        t.accent
    } else {
        t.faint
    };
    let ir = egui::Rect::from_center_size(egui::pos2(rect.center().x, rect.top() + 19.0), vec2(26.0, 26.0));
    icons::paint(ui.painter(), ir, icon, icon_color);
    let at = egui::pos2(rect.center().x - galley.size().x / 2.0, rect.bottom() - 6.0 - galley.size().y);
    ui.painter().galley_with_override_text_color(at, galley, text);
    resp
}

fn reached(ui: &Ui, rect: egui::Rect) {
    ui.ctx().data_mut(|d| {
        let f = d.get_temp_mut_or_default::<RibbonFit>(ribbon_fit_id());
        if f.active {
            f.right = f.right.max(rect.right());
            f.left = f.left.min(rect.left());
        }
    });
}

/// A button's background as the pointer comes and goes: the hover fades in and out (quickly),
/// a press shows at once.
fn hover_fill(ui: &Ui, resp: &Response, rect: egui::Rect, t: &Tokens, enabled: bool) {
    let k = crate::motion::toward(ui.ctx(), resp.id.with("hover"), enabled && resp.hovered(), crate::motion::FAST);
    // Filled with an edge, as Outlook's buttons are under the pointer (issue #8).
    if enabled && resp.is_pointer_button_down_on() {
        ui.painter().rect(
            rect,
            f32::from(crate::sizes::CONTROL_RADIUS),
            t.accent_soft,
            Stroke::new(1.0, t.accent.gamma_multiply(0.6)),
            egui::StrokeKind::Inside,
        );
    } else if k > 0.0 {
        ui.painter().rect(
            rect,
            f32::from(crate::sizes::CONTROL_RADIUS),
            t.hover.gamma_multiply(k),
            Stroke::new(1.0, t.border.gamma_multiply(k)),
            egui::StrokeKind::Inside,
        );
    }
}

/// What a drawn button is to a screen reader: a button (or a toggle, `on` given) named `label`
/// (translated), as egui's own buttons say; without it, AccessKit reads an unnamed button.
pub(crate) fn named(ui: &Ui, resp: &Response, label: &str, enabled: bool, on: Option<bool>) {
    let enabled = enabled && ui.is_enabled();
    resp.widget_info(|| match on {
        Some(on) => egui::WidgetInfo::selected(egui::Role::Button, enabled, on, label),
        None => egui::WidgetInfo::labeled(egui::Role::Button, enabled, label),
    });
}

/// An icon alone, its label as the tooltip (a ribbon too wide for the window).
fn icon_button(ui: &mut Ui, icon: Icon, label: &str, t: &Tokens, enabled: bool, on: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(32.0, 30.0), if enabled { Sense::click() } else { Sense::hover() });
    reached(ui, rect);
    if on {
        ui.painter().rect(
            rect,
            f32::from(crate::sizes::CONTROL_RADIUS),
            t.accent_soft,
            Stroke::new(1.0, t.accent.gamma_multiply(0.5)),
            egui::StrokeKind::Inside,
        );
    } else {
        hover_fill(ui, &resp, rect, t, enabled);
    }
    let ir = egui::Rect::from_min_size(rect.min + vec2(8.0, 7.0), vec2(16.0, 16.0));
    icons::paint(ui.painter(), ir, icon, icon_ink(t, enabled, on || resp.hovered()));
    resp.on_hover_text(label)
}

/// A command's icon: in the text's own quiet colour, full under the pointer (every icon in the
/// app colour left nothing for it to set apart), faint when it can't be used.
fn icon_ink(t: &Tokens, enabled: bool, hovered: bool) -> Color32 {
    match (enabled, hovered) {
        (false, _) => t.faint,
        (true, true) => t.text,
        (true, false) => t.dim,
    }
}

/// A ribbon button's label: its English words, which the button translates and finds its key
/// tip by, or words already in the language shown (a label with a value in it, made with
/// `tf!`, has no English key of its own), with the English words its key tip goes by.
#[derive(Clone, Copy, Debug)]
pub enum Label<'a> {
    Key(&'a str),
    Shown { key: &'a str, text: &'a str },
}

impl<'a> Label<'a> {
    /// `text`, already translated (a `tf!` with a value), its key tip found by `key`.
    pub fn shown(key: &'a str, text: &'a str) -> Self {
        Label::Shown { key, text }
    }

    /// What's drawn.
    fn text(self) -> &'a str {
        match self {
            Label::Key(key) => tl!(key),
            Label::Shown { text, .. } => text,
        }
    }

    /// The English words its key tip goes by.
    fn key(self) -> &'a str {
        match self {
            Label::Key(key) | Label::Shown { key, .. } => key,
        }
    }
}

impl<'a> From<&'a str> for Label<'a> {
    fn from(key: &'a str) -> Self {
        Label::Key(key)
    }
}

/// A label made at run time (`tf!`, a value in it) is shown as it is: translating it again
/// looked the translation up as English.
impl<'a> From<&'a String> for Label<'a> {
    fn from(text: &'a String) -> Self {
        Label::Shown { key: text, text }
    }
}

/// A ribbon button: its icon, its label to the right (on the classic ribbon, a big icon over
/// its label).
pub fn tool_button<'a>(ui: &mut Ui, icon: Icon, label: impl Into<Label<'a>>, t: &Tokens, enabled: bool) -> Response {
    let label = label.into();
    let mut resp = tool_button_drawn(ui, icon, label.text(), t, enabled);
    // Its key tip, by its label in English (Alt, then its tab's letters and its own).
    crate::keytips::apply(ui, label.key(), &mut resp, enabled, t);
    resp
}

/// A ribbon button that opens a menu: [`tool_button`] with a small arrow after its label, so it
/// reads as one (Quick steps, Rules, Categorize and Snooze looked like commands).
pub fn menu_tool_button<'a>(ui: &mut Ui, icon: Icon, label: impl Into<Label<'a>>, t: &Tokens, enabled: bool) -> Response {
    let label = label.into();
    let mut resp = tool_button_with(ui, icon, label.text(), t, enabled, true);
    crate::keytips::apply(ui, label.key(), &mut resp, enabled, t);
    resp
}

/// A split button's arrow, against the button before it (it was a button of its own 8 points
/// away): as tall as it, a line between them under the pointer. Its menu is the caller's.
pub fn split_arrow(ui: &mut Ui, tip: &str, t: &Tokens) -> Response {
    if classic(ui) {
        return tiny_icon_button(ui, Icon::ChevronDown, tip, t);
    }
    ui.add_space(-ui.spacing().item_spacing.x);
    let (rect, resp) = ui.allocate_exact_size(vec2(16.0, 30.0), Sense::click());
    named(ui, &resp, tl!(tip), true, None);
    reached(ui, rect);
    if resp.hovered() {
        ui.painter().rect_filled(rect, 4.0, t.hover);
        ui.painter().vline(rect.left(), rect.shrink2(vec2(0.0, 6.0)).y_range(), Stroke::new(1.0, t.border));
    }
    icons::paint(ui.painter(), egui::Rect::from_center_size(rect.center(), vec2(10.0, 10.0)), Icon::ChevronDown, icon_ink(t, true, resp.hovered()));
    resp.on_hover_text(tl!(tip))
}

/// A ribbon button labelled `label` (in the language shown).
fn tool_button_drawn(ui: &mut Ui, icon: Icon, label: &str, t: &Tokens, enabled: bool) -> Response {
    tool_button_with(ui, icon, label, t, enabled, false)
}

/// A ribbon button labelled `label`; `menu`: with an arrow after its label.
fn tool_button_with(ui: &mut Ui, icon: Icon, label: &str, t: &Tokens, enabled: bool, menu: bool) -> Response {
    if classic(ui) {
        // Its label's width noted all the same, so the simplified ribbon fits at once when it's back.
        let width = ui.painter().layout_no_wrap(label.to_string(), theme::regular(crate::sizes::BODY), t.text).size().x;
        ribbon_button(ui, width);
        let resp = big_button(ui, icon, label, t, enabled, false, false);
        named(ui, &resp, label, enabled, None);
        return resp;
    }
    let font = theme::regular(crate::sizes::BODY);
    let galley = ui.painter().layout_no_wrap(label.to_string(), font, t.text);
    // Its arrow goes with its words (the fit counts both, or it put them back for room it hadn't).
    let arrow = if menu { 11.0 } else { 0.0 };
    if ribbon_button(ui, galley.size().x + arrow) {
        let resp = icon_button(ui, icon, label, t, enabled, false);
        named(ui, &resp, label, enabled, None);
        return resp;
    }
    let size = vec2(galley.size().x + 36.0 + arrow, 30.0);
    let (rect, resp) = ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
    named(ui, &resp, label, enabled, None);
    reached(ui, rect);
    let color = command_label(t, enabled);
    hover_fill(ui, &resp, rect, t, enabled);
    let m = crate::bidi::Mirror::new(rect);
    let ir = m.rect(egui::Rect::from_min_size(rect.min + vec2(8.0, 7.0), vec2(16.0, 16.0)));
    icons::paint(ui.painter(), ir, icon, icon_ink(t, enabled, resp.hovered()));
    let text_w = galley.size().x;
    let at = egui::pos2(m.start(rect.min.x + 30.0, text_w), rect.min.y + (30.0 - galley.size().y) / 2.0);
    ui.painter().galley_with_override_text_color(at, galley, color);
    if menu {
        let ar = m.rect(egui::Rect::from_min_size(egui::pos2(rect.min.x + 30.0 + text_w + 3.0, rect.center().y - 4.0), vec2(8.0, 8.0)));
        icons::paint(ui.painter(), ar, Icon::ChevronDown, t.dim);
    }
    resp
}

/// A ribbon button that stays lit while it's on (a search term in the box, say).
pub fn tool_toggle<'a>(ui: &mut Ui, icon: Icon, label: impl Into<Label<'a>>, t: &Tokens, on: bool) -> Response {
    let label = label.into();
    let mut resp = tool_toggle_drawn(ui, icon, label.text(), t, on);
    // Its key tip, by its label in English (Alt, then its tab's letters and its own).
    crate::keytips::apply(ui, label.key(), &mut resp, true, t);
    resp
}

/// A ribbon toggle labelled `label` (in the language shown).
fn tool_toggle_drawn(ui: &mut Ui, icon: Icon, label: &str, t: &Tokens, on: bool) -> Response {
    if classic(ui) {
        // Its label's width noted all the same, so the simplified ribbon fits at once when it's back.
        let width = ui.painter().layout_no_wrap(label.to_string(), theme::regular(crate::sizes::BODY), t.text).size().x;
        ribbon_button(ui, width);
        let resp = big_button(ui, icon, label, t, true, on, false);
        named(ui, &resp, label, true, Some(on));
        return resp;
    }
    let galley = ui.painter().layout_no_wrap(label.to_string(), theme::regular(crate::sizes::BODY), t.text);
    if ribbon_button(ui, galley.size().x) {
        let resp = icon_button(ui, icon, label, t, true, on);
        named(ui, &resp, label, true, Some(on));
        return resp;
    }
    let (rect, resp) = ui.allocate_exact_size(vec2(galley.size().x + 36.0, 30.0), Sense::click());
    named(ui, &resp, label, true, Some(on));
    reached(ui, rect);
    // Lit or not, easing between the two.
    let lit = crate::motion::toward(ui.ctx(), resp.id.with("on"), on, crate::motion::NORMAL);
    if lit > 0.0 {
        ui.painter().rect(
            rect,
            6.0,
            t.accent_soft.gamma_multiply(lit),
            Stroke::new(1.0, t.accent.gamma_multiply(0.5 * lit)),
            egui::StrokeKind::Inside,
        );
    }
    if !on {
        hover_fill(ui, &resp, rect, t, true);
    }
    let m = crate::bidi::Mirror::new(rect);
    icons::paint(ui.painter(), m.rect(egui::Rect::from_min_size(rect.min + vec2(8.0, 7.0), vec2(16.0, 16.0))), icon, t.accent);
    let at = egui::pos2(m.start(rect.min.x + 30.0, galley.size().x), rect.min.y + (30.0 - galley.size().y) / 2.0);
    ui.painter().galley_with_override_text_color(at, galley, t.text);
    resp
}

/// A borderless icon button, its label on hover (a message's actions over it, as Windows' own
/// apps draw a pane's commands): its fill only under the pointer.
pub fn icon_action(ui: &mut Ui, icon: Icon, tip: &str, t: &Tokens) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(32.0, 32.0), Sense::click());
    named(ui, &resp, tl!(tip), true, None);
    if resp.hovered() {
        ui.painter().rect_filled(rect, f32::from(crate::sizes::CONTROL_RADIUS), t.hover);
    }
    icons::paint(ui.painter(), icons::centered(rect, 18.0), icon, if resp.hovered() { t.text } else { t.dim });
    resp.on_hover_text(tl!(tip))
}

/// A quiet drop-down: its words and the app's drawn chevron after them, a fill under the pointer
/// (a filled "⏷" or a raised "▾" glyph, beside chevrons drawn elsewhere, was a third and fourth
/// kind of arrow). Its menu is the caller's.
pub fn dropdown_label(ui: &mut Ui, text: &str, t: &Tokens) -> Response {
    let galley = ui.painter().layout_no_wrap(text.to_string(), theme::regular(crate::sizes::CAPTION), t.dim);
    let (rect, resp) = ui.allocate_exact_size(vec2(galley.size().x + 26.0, 22.0), Sense::click());
    named(ui, &resp, text, true, None);
    if resp.hovered() {
        ui.painter().rect_filled(rect, f32::from(crate::sizes::CONTROL_RADIUS), t.hover);
    }
    let ink = if resp.hovered() { t.text } else { t.dim };
    let m = crate::bidi::Mirror::new(rect);
    let text_w = galley.size().x;
    ui.painter().galley_with_override_text_color(
        egui::pos2(m.start(rect.left() + 6.0, text_w), rect.center().y - galley.size().y / 2.0),
        galley,
        ink,
    );
    icons::paint(
        ui.painter(),
        m.rect(egui::Rect::from_min_size(egui::pos2(rect.left() + 10.0 + text_w, rect.center().y - 4.0), vec2(9.0, 9.0))),
        Icon::ChevronDown,
        ink,
    );
    resp
}

/// [`icon_action`]'s "⋯", for a menu of the rest.
pub fn icon_more(ui: &mut Ui, tip: &str, t: &Tokens) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(32.0, 32.0), Sense::click());
    named(ui, &resp, tl!(tip), true, None);
    if resp.hovered() {
        ui.painter().rect_filled(rect, f32::from(crate::sizes::CONTROL_RADIUS), t.hover);
    }
    for dx in [-6.0, 0.0, 6.0] {
        ui.painter().circle_filled(rect.center() + vec2(dx, 0.0), 1.6, if resp.hovered() { t.text } else { t.dim });
    }
    resp.on_hover_text(tl!(tip))
}

/// A "⋯" button in the style of [`pill_button`], for a menu of more actions.
pub fn more_pill(ui: &mut Ui, tip: &str, t: &Tokens) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(34.0, 28.0), Sense::click());
    named(ui, &resp, tl!(tip), true, None);
    let fill = if resp.hovered() { t.hover } else { t.surface };
    ui.painter().rect(rect, f32::from(crate::sizes::CONTROL_RADIUS), fill, Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
    for dx in [-6.0, 0.0, 6.0] {
        ui.painter().circle_filled(rect.center() + vec2(dx, 0.0), 1.6, t.dim);
    }
    resp.on_hover_text(tl!(tip))
}

/// The filled "New mail" button.
pub fn accent_button<'a>(ui: &mut Ui, icon: Icon, label: impl Into<Label<'a>>, t: &Tokens, enabled: bool) -> Response {
    let label = label.into();
    let mut resp = accent_button_drawn(ui, icon, label.text(), t, enabled);
    // Its key tip, by its label in English (Alt, then its tab's letters and its own).
    crate::keytips::apply(ui, label.key(), &mut resp, enabled, t);
    resp
}

/// The filled button labelled `label` (in the language shown).
fn accent_button_drawn(ui: &mut Ui, icon: Icon, label: &str, t: &Tokens, enabled: bool) -> Response {
    if classic(ui) {
        let resp = big_button(ui, icon, label, t, enabled, false, true);
        named(ui, &resp, label, enabled, None);
        return resp;
    }
    let font = theme::semibold(crate::sizes::BODY);
    let galley = ui.painter().layout_no_wrap(label.to_string(), font, t.on_accent);
    let size = vec2(galley.size().x + 40.0, 30.0);
    let (rect, resp) = ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
    named(ui, &resp, label, enabled, None);
    ui.painter().rect_filled(rect, f32::from(crate::sizes::CONTROL_RADIUS), accent_fill(t, enabled, resp.hovered()));
    let m = crate::bidi::Mirror::new(rect);
    let ir = m.rect(egui::Rect::from_min_size(rect.min + vec2(10.0, 7.0), vec2(16.0, 16.0)));
    icons::paint(ui.painter(), ir, icon, t.on_accent);
    let at = egui::pos2(m.start(rect.min.x + 32.0, galley.size().x), rect.min.y + (30.0 - galley.size().y) / 2.0);
    ui.painter().galley_with_override_text_color(at, galley, t.on_accent);
    resp
}

/// The top bar's hover: a lighter wash on the coloured bar, the pane's hover on a plain one.
pub fn bar_wash(t: &Tokens) -> Color32 {
    if t.on_bar == Color32::WHITE { Color32::from_white_alpha(40) } else { t.hover }
}

/// An icon button on the coloured title bar.
pub fn bar_icon_button(ui: &mut Ui, icon: Icon, tip: &str, t: &Tokens) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(30.0, 30.0), Sense::click());
    named(ui, &resp, tl!(tip), true, None);
    if resp.hovered() {
        ui.painter().rect_filled(rect, f32::from(crate::sizes::CONTROL_RADIUS), bar_wash(t));
    }
    icons::paint(ui.painter(), icons::centered(rect, 17.0), icon, t.on_bar);
    resp.on_hover_text(tl!(tip))
}

/// A small icon button inside panes.
pub fn tiny_icon_button(ui: &mut Ui, icon: Icon, tip: &str, t: &Tokens) -> Response {
    // On the classic ribbon, a button's drop-down arrow is as tall as the button, beside it.
    if classic(ui) {
        let (rect, resp) = ui.allocate_exact_size(vec2(18.0, CLASSIC_BUTTON_H), Sense::click());
        named(ui, &resp, tl!(tip), true, None);
        reached(ui, rect);
        hover_fill(ui, &resp, rect, t, true);
        let ir = egui::Rect::from_center_size(egui::pos2(rect.center().x, rect.bottom() - 14.0), vec2(12.0, 12.0));
        icons::paint(ui.painter(), ir, icon, if resp.hovered() { t.text } else { t.dim });
        return resp.on_hover_text(tl!(tip));
    }
    let (rect, resp) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::click());
    named(ui, &resp, tl!(tip), true, None);
    if resp.hovered() {
        ui.painter().rect_filled(rect, 4.0, t.hover);
    }
    icons::paint(ui.painter(), icons::centered(rect, 13.0), icon, if resp.hovered() { t.text } else { t.dim });
    resp.on_hover_text(tl!(tip))
}

/// Same-height chips in columns that share the width and wrap onto more rows, as Outlook lays out
/// attachments; past `max_rows` rows they scroll. `chip(ui, i, width)` draws chip `i`.
pub fn chip_grid(ui: &mut Ui, id: egui::Id, n: usize, min_w: f32, max_w: f32, h: f32, max_rows: usize, mut chip: impl FnMut(&mut Ui, usize, f32)) {
    if n == 0 {
        return;
    }
    let gap = 6.0;
    let fits = |avail: f32| {
        let mut cols = 1usize;
        while (cols + 1) as f32 * min_w + cols as f32 * gap <= avail {
            cols += 1;
        }
        cols
    };
    let mut avail = ui.available_width().max(min_w);
    let max_rows = max_rows.max(1);
    // Leave room for the scroll bar when the chips will scroll.
    if n.div_ceil(fits(avail)) > max_rows {
        avail = (avail - egui::style::ScrollStyle::solid().bar_width - 6.0).max(min_w);
    }
    let cols = fits(avail);
    let w = ((avail - (cols - 1) as f32 * gap) / cols as f32).clamp(min_w.min(avail), max_w);
    let rows = n.div_ceil(cols);
    ui.scope(|ui| {
        // A solid scroll bar, always shown, so it's plain that there are more.
        ui.spacing_mut().scroll = egui::style::ScrollStyle::solid();
        egui::ScrollArea::vertical().id_salt(id).max_height(max_rows as f32 * (h + gap) - gap).auto_shrink([false, true]).show(ui, |ui| {
            ui.spacing_mut().item_spacing = vec2(gap, gap);
            for r in 0..rows {
                ui.horizontal(|ui| {
                    for i in r * cols..((r + 1) * cols).min(n) {
                        chip(ui, i, w);
                    }
                });
            }
        });
    });
}

/// Text cut to one line of `width`, ending in "…" when it doesn't fit.
pub fn one_line(ui: &Ui, text: &str, font: egui::FontId, color: Color32, width: f32) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_owned(), font, color);
    job.wrap = egui::text::TextWrapping { max_width: width.max(1.0), max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
    ui.painter().layout_job(job)
}

/// A labelled icon button for the reading pane's action row.
pub fn pill_button(ui: &mut Ui, icon: Icon, label: &str, t: &Tokens) -> Response {
    let label = tl!(label);
    let galley = ui.painter().layout_no_wrap(label.to_string(), theme::regular(crate::sizes::BODY), t.text);
    let size = vec2(galley.size().x + 34.0, 28.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    named(ui, &resp, label, true, None);
    let fill = if resp.hovered() { t.hover } else { t.surface };
    ui.painter().rect(rect, f32::from(crate::sizes::CONTROL_RADIUS), fill, Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
    let m = crate::bidi::Mirror::new(rect);
    icons::paint(ui.painter(), m.rect(egui::Rect::from_min_size(rect.min + vec2(9.0, 6.0), vec2(16.0, 16.0))), icon, t.accent);
    let at = egui::pos2(m.start(rect.min.x + 29.0, galley.size().x), rect.min.y + (28.0 - galley.size().y) / 2.0);
    ui.painter().galley_with_override_text_color(at, galley, t.text);
    resp
}

/// An on/off switch (a setting that applies at once). `label` (translated) is for screen readers
/// only; the row shows its own. Clicking it flips `on`; the response says it changed.
pub fn switch(ui: &mut Ui, t: &Tokens, on: &mut bool, label: &str) -> Response {
    // On or Off beside it, dim, as Windows' own switches say (36 by 20 and silent, it was
    // small and said nothing to anyone not sure which way is on).
    let said = tl!(if *on { "On" } else { "Off" });
    let word = ui.painter().layout_no_wrap(said.to_string(), theme::regular(crate::sizes::CAPTION), t.dim);
    let (whole, mut resp) = ui.allocate_exact_size(vec2(40.0 + 8.0 + word.size().x.max(20.0), 20.0), Sense::click());
    let m = crate::bidi::Mirror::new(whole);
    let rect = m.rect(egui::Rect::from_min_size(egui::pos2(whole.right() - 40.0, whole.top()), vec2(40.0, 20.0)));
    let word_at =
        m.rect(egui::Rect::from_min_size(egui::pos2(whole.right() - 48.0 - word.size().x, whole.center().y - word.size().y / 2.0), word.size()));
    ui.painter().galley(word_at.min, word, t.dim);
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let label = label.to_string();
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, ui.is_enabled(), *on, &label));
    let k = ui.ctx().animate_bool_responsive(resp.id, *on);
    let enabled = ui.is_enabled();
    let track = if *on {
        t.accent
    } else if resp.hovered() {
        t.hover
    } else {
        t.surface
    };
    let track = if enabled { track } else { track.gamma_multiply(0.5) };
    let edge = if *on { t.accent } else { t.dim };
    ui.painter().rect(rect, 10.0, track, Stroke::new(1.0, if enabled { edge } else { t.border }), egui::StrokeKind::Inside);
    let x = egui::lerp(rect.left() + 10.0..=rect.right() - 10.0, k);
    let knob = if *on { t.on_accent } else { t.dim };
    ui.painter().circle_filled(egui::pos2(x, rect.center().y), if resp.hovered() && enabled { 6.5 } else { 6.0 }, knob);
    if resp.has_focus() {
        ui.painter().rect_stroke(rect.expand(2.0), 12.0, Stroke::new(1.5, t.accent), egui::StrokeKind::Outside);
    }
    resp
}

/// One choice among a few, as joined buttons (Theme: System, Light, Dark), labels translated.
/// The picked one is raised; returns the index clicked, if any.
pub fn segmented(ui: &mut Ui, t: &Tokens, id: &str, options: &[&str], selected: usize) -> Option<usize> {
    segments(ui, t, id, options, &|i| i == selected, egui::Role::RadioButton).filter(|i| *i != selected)
}

/// Several on at once, as joined buttons (the work week's days), each on in the soft accent:
/// the one clicked, on or off (they were bare words, lit or not). Labels translated by the caller.
pub fn toggle_segments(ui: &mut Ui, t: &Tokens, id: &str, options: &[&str], on: &dyn Fn(usize) -> bool) -> Option<usize> {
    segments(ui, t, id, options, on, egui::Role::CheckBox)
}

/// Joined buttons, those `on` in the soft accent; the one clicked.
fn segments(ui: &mut Ui, t: &Tokens, id: &str, options: &[&str], is_on: &dyn Fn(usize) -> bool, role: egui::Role) -> Option<usize> {
    let font = theme::regular(crate::sizes::BODY);
    let labels: Vec<String> = options.iter().map(|o| o.to_string()).collect();
    let widths: Vec<f32> = labels.iter().map(|l| ui.painter().layout_no_wrap(l.clone(), font.clone(), t.text).size().x + 24.0).collect();
    let total: f32 = widths.iter().sum::<f32>() + 4.0;
    let (rect, _) = ui.allocate_exact_size(vec2(total, 28.0), Sense::hover());
    ui.painter().rect(rect, f32::from(crate::sizes::CONTROL_RADIUS), t.bg, Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
    let mut x = rect.left() + 2.0;
    let mut clicked = None;
    for (i, (label, w)) in labels.iter().zip(&widths).enumerate() {
        let r = egui::Rect::from_min_size(egui::pos2(x, rect.top() + 2.0), vec2(*w, 24.0));
        let resp = ui.interact(r, ui.scope_id().with(("segment", id, i)), Sense::click());
        let on = is_on(i);
        resp.widget_info(|| egui::WidgetInfo::selected(role, ui.is_enabled(), on, label));
        // The chosen one in the soft accent, its words in the accent (white on a light grey track,
        // barely lifted in the dark theme, it was hard to tell).
        if on {
            ui.painter().rect_filled(r, 4.0, t.accent_soft);
        } else if resp.hovered() {
            // A faint look of the chosen one's, never more than it: the hover's own colour was
            // lighter than the chosen segment's in the dark theme, so the one under the pointer
            // looked chosen.
            ui.painter().rect_filled(r, 4.0, t.surface.gamma_multiply(0.5));
        }
        let color = if on {
            t.accent
        } else if resp.hovered() {
            t.text
        } else {
            t.dim
        };
        ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, label, if on { theme::semibold(crate::sizes::BODY) } else { font.clone() }, color);
        if resp.clicked() {
            clicked = Some(i);
        }
        x += w;
    }
    clicked
}

pub fn group_sep(ui: &mut Ui, t: &Tokens) {
    ui.add_space(4.0);
    let h = if classic(ui) { CLASSIC_BUTTON_H + CLASSIC_CAPTION_H } else { 26.0 };
    let (r, _) = ui.allocate_exact_size(vec2(1.0, h), Sense::hover());
    ui.painter().rect_filled(r, 0.0, t.border);
    ui.add_space(4.0);
    start_group(ui);
}

/// A ribbon group ends: on the classic ribbon its name (translated) is written under it; then
/// the line before the next one, unless it's the `last`.
pub fn group(ui: &mut Ui, t: &Tokens, name: &str, last: bool) {
    if classic(ui) {
        let from = ui.ctx().data_mut(|d| d.get_temp_mut_or_default::<RibbonFit>(ribbon_fit_id()).group_from);
        let to = cursor_x(ui);
        let y = ui.max_rect().top() + CLASSIC_BUTTON_H + CLASSIC_CAPTION_H / 2.0 + 4.0;
        let mid = if from > 0.0 && from != to { (from + to) / 2.0 } else { to };
        ui.painter().text(egui::pos2(mid, y), egui::Align2::CENTER_CENTER, tl!(name), theme::regular(crate::sizes::CAPTION), group_caption(t));
    }
    if !last {
        group_sep(ui, t);
    }
}

/// A row in a sideways scroll area: from the left, or in a right-to-left interface from the
/// right of a strip as wide as the row took (laid out against the scroll area's right edge, what
/// didn't fit ran off its left, where it can't be scrolled to).
pub fn scrolled_row(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    if !crate::i18n::current().rtl() {
        ui.horizontal(add);
        return;
    }
    let id = ui.scope_id().with("rtl-row-width");
    let width = ui.ctx().data(|d| d.get_temp::<f32>(id)).unwrap_or(0.0).max(ui.available_width());
    let height = ui.spacing().interact_size.y;
    ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
        ui.allocate_ui_with_layout(vec2(width, height), egui::Layout::right_to_left(egui::Align::Center), |ui| {
            add(ui);
            let used = ui.min_rect().width();
            let before = ui.ctx().data_mut(|d| std::mem::replace(d.get_temp_mut_or_default::<f32>(id), used));
            // Laid out again at its new width.
            if (before - used).abs() > 0.5 {
                ui.ctx().request_repaint();
            }
        });
    });
}

/// Where the next thing goes in a row: its left edge, or its right one in a right-to-left row.
fn cursor_x(ui: &Ui) -> f32 {
    if ui.layout().main_dir() == egui::Direction::RightToLeft { ui.cursor().right() } else { ui.cursor().left() }
}

/// Where the next group starts (the classic ribbon centres its name under it).
pub fn start_group(ui: &Ui) {
    let x = cursor_x(ui);
    ui.ctx().data_mut(|d| {
        let f = d.get_temp_mut_or_default::<RibbonFit>(ribbon_fit_id());
        if f.active {
            f.group_from = x;
        }
    });
}

/// What a pane shows with nothing in it: a big faint icon over a line saying why (`text`, as
/// shown).
pub fn empty_state(ui: &mut Ui, t: &Tokens, icon: Icon, text: &str) {
    ui.centered_and_justified(|ui| {
        ui.vertical_centered(|ui| {
            ui.add_space(ui.available_height() * 0.35);
            let (r, _) = ui.allocate_exact_size(vec2(56.0, 56.0), Sense::hover());
            icons::paint(ui.painter(), r, icon, t.border);
            ui.add_space(8.0);
            ui.label(RichText::new(text).color(t.faint));
        });
    });
}

/// A round avatar with initials.
pub fn avatar(ui: &Ui, center: egui::Pos2, radius: f32, initials: &str, color: Color32) {
    ui.painter().circle_filled(center, radius, color);
    ui.painter().text(center, egui::Align2::CENTER_CENTER, initials, theme::semibold(radius * 0.8), Color32::WHITE);
}

pub fn human_size(bytes: usize) -> String {
    human_bytes(u64::try_from(bytes).unwrap_or(u64::MAX))
}

/// A size as people read it: bytes, KB, MB, or GB (a data file's tens of gigabytes; `usize`
/// can't hold them in the browser).
pub fn human_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    let b = bytes as f64;
    if b < KB {
        format!("{bytes} B")
    } else if b < KB * KB {
        format!("{:.0} KB", b / KB)
    } else if b < KB * KB * KB {
        format!("{:.1} MB", b / KB / KB)
    } else {
        format!("{:.1} GB", b / KB / KB / KB)
    }
}

pub fn shorten(s: &str, max: usize) -> String {
    if s.chars().count() <= max { s.to_string() } else { format!("{}…", s.chars().take(max.saturating_sub(1)).collect::<String>()) }
}

/// The colour of a ribbon command's label: the text's, or, when it can't be used now, a quiet
/// grey that still reads (issue #45).
pub(crate) fn command_label(t: &Tokens, enabled: bool) -> Color32 {
    if enabled { t.text } else { theme::quiet_text(t) }
}

/// The colour of a classic ribbon group's caption (Respond, Tags…): quiet, and read (issue #45).
pub(crate) fn group_caption(t: &Tokens) -> Color32 {
    theme::quiet_text(t)
}
