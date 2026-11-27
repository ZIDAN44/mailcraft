//! The interface's sizes, one set for every pane and dialog, as Windows' own apps have them
//! (Fluent 2): a type ramp, spacing on a 4-point grid, one height for controls, two corner radii.
//! A size used in one place only is the exception, and says why.

/// Type ramp: captions (a field's help, a date in a list, a count).
pub const CAPTION: f32 = 12.0;
/// Type ramp: the body (labels, fields, a list's lines, buttons).
pub const BODY: f32 = 14.0;
/// Type ramp: a subtitle (a dialog's title, a pane's heading).
pub const SUBTITLE: f32 = 20.0;
/// Type ramp: a page's title (File's pages).
pub const TITLE: f32 = 28.0;

/// The height of a dialog's button, text box and drop-down.
pub const CONTROL_H: f32 = 32.0;
/// The height of the panes' and menus' controls, and of a list's rows in a dialog.
pub const PANE_CONTROL_H: f32 = 26.0;
/// The least width of a dialog's button (Cancel was narrower than the button beside it).
pub const BUTTON_MIN_W: f32 = 80.0;

/// Corners: a control's (buttons, fields).
pub const CONTROL_RADIUS: u8 = 4;
/// Corners: a surface's (dialogs, menus, cards, the File page's cards).
pub const SURFACE_RADIUS: u8 = 8;

/// Spacing: between a label and its field, or two small things together.
pub const GAP_S: f32 = 8.0;
/// Spacing: between two fields of a form.
pub const GAP_M: f32 = 16.0;
/// Spacing: between groups, and a dialog's margin.
pub const GAP_L: f32 = 24.0;
