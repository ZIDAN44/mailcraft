//! What the formatting keys did, said to a screen reader (issue #175): Ctrl+B, Ctrl+I and Ctrl+U
//! switch a look on or off, and nothing where the keyboard is (the text) says which.

use super::*;

/// Apply what the keys asked for (`ops`), saying how bold, italic or underline is left: for
/// what's typed next with nothing selected, for the whole selection otherwise.
pub(super) fn apply_keyed(ctx: &egui::Context, ed: &mut Editor, c: &mut Compose, ops: Vec<Op>) {
    for op in ops {
        let get: Option<fn(&Style) -> bool> = match op {
            Op::Bold => Some(|s| s.bold),
            Op::Italic => Some(|s| s.italic),
            Op::Underline => Some(|s| s.underline),
            _ => None,
        };
        let which = op.clone();
        apply(ed, c, op);
        let Some(get) = get else { continue };
        let n = c.body.chars().count();
        let sel = ed.sel.start.min(n)..ed.sel.end.min(n);
        let now = if sel.is_empty() { ed.typing.as_ref().is_some_and(get) } else { rich::all(&c.body, &c.styles, sel, get) };
        let words = match (which, now) {
            (Op::Bold, true) => tl!("Bold on"),
            (Op::Bold, false) => tl!("Bold off"),
            (Op::Italic, true) => tl!("Italic on"),
            (Op::Italic, false) => tl!("Italic off"),
            (_, true) => tl!("Underline on"),
            (_, false) => tl!("Underline off"),
        };
        crate::a11y::announce(ctx, words, false);
    }
}
