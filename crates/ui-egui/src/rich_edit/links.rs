//! Links in the message editor: Insert link, and addresses as they're shown and written.

use super::*;

/// Outlook's Insert Hyperlink, with what mail needs: a web page or file, or an email address.
#[derive(Clone, Debug, Default)]
pub(super) struct LinkForm {
    pub(super) email: bool,
    pub(super) text: String,
    pub(super) address: String,
    pub(super) to: String,
    pub(super) subject: String,
    /// The text the link goes on (the selection when the dialog opened).
    pub(super) sel: Range<usize>,
    /// A link was there already (Remove link is offered).
    pub(super) editing: bool,
    /// The selection spans lines: the link goes on it as it is (the text can't be retyped).
    pub(super) multi: bool,
}

impl LinkForm {
    /// The dialog for the selection: its text, and the link already on it.
    pub(super) fn open(c: &Compose, sel: Range<usize>) -> LinkForm {
        let chosen: String = c.body.chars().skip(sel.start).take(sel.end.saturating_sub(sel.start)).collect();
        let existing = c.styles.style(sel.start).link.filter(|_| !c.body.is_empty());
        let multi = chosen.contains('\n');
        let mut f = LinkForm { text: if multi { String::new() } else { chosen }, sel, editing: existing.is_some(), multi, ..Default::default() };
        match existing {
            Some(url) if url.to_ascii_lowercase().starts_with("mailto:") => {
                f.email = true;
                let rest = url.get(7..).unwrap_or_default();
                let (to, query) = rest.split_once('?').unwrap_or((rest, ""));
                f.to = percent_decode(to);
                f.subject = query.split('&').find_map(|kv| kv.strip_prefix("subject=")).map(percent_decode).unwrap_or_default();
            }
            Some(url) => f.address = url,
            None => {}
        }
        f
    }

    /// The link's address: `https://` added to a bare web address, `file:///` to a path, and
    /// a `mailto:` with its subject.
    pub(super) fn url(&self) -> Option<String> {
        if self.email {
            let to = self.to.trim();
            if to.is_empty() {
                return None;
            }
            let to = to.strip_prefix("mailto:").unwrap_or(to);
            let subject = self.subject.trim();
            return Some(if subject.is_empty() { format!("mailto:{to}") } else { format!("mailto:{to}?subject={}", percent_encode(subject)) });
        }
        let a = self.address.trim();
        if a.is_empty() || a == "https://" || a == "http://" {
            return None;
        }
        let lower = a.to_ascii_lowercase();
        let has_scheme = ["http://", "https://", "mailto:", "file:", "ftp://", "tel:"].iter().any(|p| lower.starts_with(p));
        let path = a.len() > 2 && a.as_bytes().get(1) == Some(&b':') && matches!(a.as_bytes().get(2), Some(b'\\' | b'/'));
        Some(if has_scheme {
            a.to_string()
        } else if path || a.starts_with("\\\\") {
            format!("file:///{}", a.replace('\\', "/").trim_start_matches('/'))
        } else {
            format!("https://{a}")
        })
    }
}

/// `%XX` escapes in a mailto address or subject, as text.
pub(super) fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while let Some(&c) = b.get(i) {
        let hex = |k: usize| b.get(k).and_then(|h| (*h as char).to_digit(16));
        match (c, hex(i + 1), hex(i + 2)) {
            (b'%', Some(h), Some(l)) => {
                out.push(u8::try_from(h * 16 + l).unwrap_or(b'?'));
                i += 3;
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A subject as a mailto may carry it (RFC 6068): everything but letters, digits and `-._~`
/// escaped.
pub(super) fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Insert link, as Outlook's Insert Hyperlink keeps what mail needs: what the text says, and a
/// web page (or file) or an email address with a subject. Shown over the message's own window.
pub(crate) fn link_dialog(ui: &mut Ui, t: &Tokens, ed: &mut Editor, c: &mut Compose) {
    let Some(mut f) = ed.link.take() else { return };
    let mut close = false;
    let mut done: Option<Option<String>> = None;
    let modal = egui::Modal::new(egui::Id::unique(("link-dialog", c.id))).frame(crate::dialogs::dialog_frame(t)).show(ui.ctx(), |ui| {
        ui.set_width(440.0);
        ui.label(RichText::new(tl!(if f.editing { "Edit link" } else { "Insert link" })).font(theme::bold(18.0)).color(t.text));
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new(tl!("Link to")).color(t.dim));
            ui.selectable_value(&mut f.email, false, tl!("Web page or file"));
            ui.selectable_value(&mut f.email, true, tl!("Email address"));
        });
        ui.add_space(8.0);
        let first = egui::Grid::new(("link-grid", c.id)).num_columns(2).spacing(vec2(12.0, 8.0)).show(ui, |ui| {
            let w = 300.0;
            ui.label(RichText::new(tl!("Text to display")).color(t.dim));
            if f.multi {
                ui.label(RichText::new(tl!("The selected text")).color(t.faint).italics());
            } else {
                ui.add(egui::TextEdit::singleline(&mut f.text).hint_text(tl!("The address itself")).desired_width(w));
            }
            ui.end_row();
            if f.email {
                ui.label(RichText::new(tl!("Email address")).color(t.dim));
                let r = ui.add(egui::TextEdit::singleline(&mut f.to).hint_text("name@example.com").desired_width(w));
                ui.end_row();
                ui.label(RichText::new(tl!("Subject")).color(t.dim));
                ui.add(egui::TextEdit::singleline(&mut f.subject).desired_width(w));
                ui.end_row();
                r
            } else {
                ui.label(RichText::new(tl!("Address")).color(t.dim));
                let r = ui.add(egui::TextEdit::singleline(&mut f.address).hint_text("https://").desired_width(w));
                ui.end_row();
                r
            }
        });
        if ui.memory(|m| m.focused().is_none()) {
            first.inner.request_focus();
        }
        ui.add_space(12.0);
        let url = f.url();
        let enter = ui.input(|i| i.key_pressed(Key::Enter));
        ui.horizontal(|ui| {
            if f.editing && ui.button(tl!("Remove link")).clicked() {
                done = Some(None);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add_enabled(url.is_some(), egui::Button::new(tl!("OK"))).clicked() || (enter && url.is_some()) {
                    done = Some(url.clone());
                }
                if ui.button(tl!("Cancel")).clicked() {
                    close = true;
                }
            });
        });
    });
    if modal.should_close() {
        close = true;
    }
    match done {
        Some(Some(url)) => {
            let sel = f.sel.clone();
            let n = c.body.chars().count();
            let sel = sel.start.min(n)..sel.end.min(n);
            let current: String = c.body.chars().skip(sel.start).take(sel.end - sel.start).collect();
            let shown = if f.text.trim().is_empty() {
                url.strip_prefix("mailto:").map_or(url.as_str(), |a| a.split('?').next().unwrap_or(a)).to_string()
            } else {
                f.text.clone()
            };
            if !sel.is_empty() && (shown == current || f.multi) {
                ed.sel = sel;
                apply(ed, c, Op::Link(url));
            } else {
                // New text (or the text changed): typed in place of the selection, as a link.
                let style = Style { link: Some(url), ..typing_style(&c.body, &c.styles, sel.start) };
                replace(ed, c, sel, &shown, style);
                c.plain = false;
            }
            ed.refocus = Some(ed.sel.clone());
        }
        Some(None) => {
            ed.sel = f.sel.clone();
            apply(ed, c, Op::Unlink);
            ed.refocus = Some(ed.sel.clone());
        }
        None if close => ed.refocus = Some(f.sel.clone()),
        None => ed.link = Some(f),
    }
}
