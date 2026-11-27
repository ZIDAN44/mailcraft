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

/// A dialog's width, one of three (there were fourteen, four for the same two-line question):
/// a question or one field.
pub const DIALOG_S: f32 = 360.0;
/// A dialog's width: a form.
pub const DIALOG_M: f32 = 440.0;
/// A dialog's width: a list or an editor.
pub const DIALOG_L: f32 = 560.0;
/// The widest a dialog that shows something to read is (a message attached, a person's card).
pub const VIEWER_MAX: f32 = 720.0;

#[cfg(test)]
mod tests {
    /// Audit S5: no text size is written as a number in the window's code, only the ramp's names
    /// above (a list row alone used six sizes, down to 11 points; 22 in all).
    #[test]
    fn text_sizes_come_from_the_ramp() {
        fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
                let p = e.path();
                let name = p.file_name().and_then(|n| n.to_str()).unwrap_or_default();
                if p.is_dir() && name != "tests" {
                    walk(&p, out);
                } else if name.ends_with(".rs") && name != "tests.rs" && name != "sizes.rs" {
                    out.push(p);
                }
            }
        }
        let mut files = Vec::new();
        walk(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut files);
        assert!(files.len() > 100, "the window's code found: {}", files.len());
        let starts_number = |rest: &str| rest.chars().next().is_some_and(|c| c.is_ascii_digit());
        let mut written = Vec::new();
        for f in files {
            let text = std::fs::read_to_string(&f).unwrap_or_default();
            for (i, line) in text.lines().enumerate() {
                let font = ["theme::regular(", "theme::semibold("].iter().any(|call| line.split(call).skip(1).any(starts_number));
                let rich = line.contains("RichText") && line.split(".size(").skip(1).any(starts_number);
                if font || rich {
                    written.push(format!("{}:{}: {}", f.display(), i + 1, line.trim()));
                }
            }
        }
        assert!(written.is_empty(), "text sizes written as numbers:\n{}", written.join("\n"));
    }
}
