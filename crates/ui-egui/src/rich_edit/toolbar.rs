//! The message editor's ribbon: formatting, colours, links, pictures, signature, date and time, word count.

use super::*;

/// A ribbon button: an icon or a letter, lit when its format is on.
pub(super) fn fmt_button(ui: &mut Ui, t: &Tokens, face: Face<'_>, on: bool, tip: &str) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::click());
    crate::widgets::named(ui, &resp, tl!(tip), true, Some(on));
    if on || resp.hovered() {
        ui.painter().rect_filled(rect, 4.0, if on { t.accent_soft } else { t.hover });
    }
    match face {
        Face::Icon(ic) => icons::paint(ui.painter(), icons::centered(rect, 16.0), ic, t.text),
        Face::Letter(s, f) => {
            let galley = ui.painter().layout_job(LayoutJob::single_section(s.to_string(), f));
            ui.painter().galley(rect.center() - galley.size() / 2.0, galley, t.text);
        }
    }
    resp.on_hover_text(tl!(tip))
}

pub(super) enum Face<'a> {
    Icon(Icon),
    Letter(&'a str, TextFormat),
}

/// A colour button with its dropdown (font colour, highlight).
pub(super) fn color_button(
    ui: &mut Ui,
    t: &Tokens,
    letter: &str,
    current: Option<[u8; 3]>,
    swatches: &[[u8; 3]],
    none: &str,
    tip: &str,
) -> Option<Option<[u8; 3]>> {
    let (rect, resp) = ui.allocate_exact_size(vec2(34.0, 28.0), Sense::click());
    crate::widgets::named(ui, &resp, tl!(tip), true, None);
    if resp.hovered() {
        ui.painter().rect_filled(rect, 4.0, t.hover);
    }
    ui.painter().text(rect.center() - vec2(4.0, 3.0), egui::Align2::CENTER_CENTER, letter, theme::regular(14.0), t.text);
    let bar = Rect::from_min_size(egui::pos2(rect.min.x + 6.0, rect.max.y - 7.0), vec2(16.0, 3.5));
    let shown = current.map_or(if letter == "A" { t.text } else { Color32::TRANSPARENT }, |c| Color32::from_rgb(c[0], c[1], c[2]));
    ui.painter().rect_filled(bar, 1.0, shown);
    icons::paint(ui.painter(), Rect::from_min_size(egui::pos2(rect.max.x - 11.0, rect.center().y - 4.0), vec2(8.0, 8.0)), Icon::ChevronDown, t.dim);
    let resp = resp.on_hover_text(tl!(tip));
    let mut picked = None;
    egui::Popup::menu(&resp).show(|ui| {
        ui.horizontal_wrapped(|ui| {
            ui.set_max_width(6.0 * 26.0);
            for c in swatches {
                let (r, s) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::click());
                // A stronger edge under the pointer.
                let edge = if s.hovered() { Stroke::new(2.0, t.text) } else { Stroke::new(1.0, t.border) };
                ui.painter().rect(r, 3.0, Color32::from_rgb(c[0], c[1], c[2]), edge, egui::StrokeKind::Inside);
                if s.clicked() {
                    picked = Some(Some(*c));
                }
            }
        });
        if ui.button(tl!(none)).clicked() {
            picked = Some(None);
        }
    });
    picked
}

/// The Message ribbon: font size, bold / italic / underline / strikethrough, colours, lists,
/// alignment, indent, clear formatting; then names, files, links and pictures.
/// The Message ribbon; `signature` is the sending account's (for Insert › Signature).
pub(crate) fn ribbon(ui: &mut Ui, t: &Tokens, ed: &mut Editor, c: &mut Compose, signature: &str, clock: &Clock, acts: &mut Vec<Action>) {
    c.styles.fit(&c.body);
    let n = c.body.chars().count();
    let sel = ed.sel.start.min(n)..ed.sel.end.min(n);
    let cur =
        ed.typing.clone().unwrap_or_else(|| if sel.is_empty() { typing_style(&c.body, &c.styles, sel.start) } else { c.styles.style(sel.start) });
    let para = c.styles.para(rich::lines_of(&c.body, sel.start..sel.start).0);
    let mut ops: Vec<Op> = Vec::new();
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        format_buttons(ui, t, c, &cur, para, &mut ops);
        ui.separator();
        if crate::widgets::tool_button(ui, Icon::AddressBook, "Address Book", t, !c.sending).on_hover_text(tl!("Ctrl+Shift+B")).clicked() {
            acts.push(Action::Dialog(Dialog::select_names(c, "to")));
        }
        if crate::widgets::tool_button(ui, Icon::Attach, "Attach File", t, !c.sending).clicked() {
            acts.push(Action::AttachFiles);
        }
        if crate::widgets::tool_button(ui, Icon::Link, "Link", t, !c.sending && !c.plain).on_hover_text(tl!("Ctrl+K")).clicked() {
            ed.link = Some(LinkForm::open(c, sel.clone()));
        }
        if crate::widgets::tool_button(ui, Icon::Image, "Pictures", t, !c.sending && !c.plain).clicked() {
            acts.push(Action::InsertPicture);
        }
        // The account's signature at the cursor, or making and changing it, from the message
        // (Outlook's Signature menu on the Message ribbon).
        let r = crate::widgets::tool_button(ui, Icon::Drafts, "Signature", t, !c.sending);
        egui::Popup::menu(&r).show(|ui| {
            if ui.add_enabled(!signature.trim().is_empty(), egui::Button::new(tl!("Insert signature"))).clicked() {
                acts.push(Action::InsertSignature);
                ui.close();
            }
            if ui.button(tl!("Signatures…")).clicked() {
                acts.push(Action::EditSignature { account: c.account.clone(), insert: true });
                ui.close();
            }
        });
        // Today's date or the time at the cursor, in one of a few ways (Outlook's Date & Time).
        let r = crate::widgets::tool_button(ui, Icon::Calendar, "Date & time", t, !c.sending);
        egui::Popup::menu(&r).show(|ui| {
            for text in date_choices(clock) {
                if ui.button(text.as_str()).clicked() {
                    ops.push(Op::Insert(text));
                    ui.close();
                }
            }
        });
        // How much is written: the selection's count, or the whole message's.
        if crate::widgets::tool_button(ui, Icon::ListNumber, "Word count", t, true).clicked() {
            let range = if sel.is_empty() { 0..n } else { sel.clone() };
            let text: String = c.body.chars().skip(range.start).take(range.end - range.start).filter(|ch| *ch != rich::PICTURE).collect();
            acts.push(Action::Dialog(Dialog::WordCount(count_words(&text), !sel.is_empty())));
        }
    });
    link_dialog(ui, t, ed, c);
    let changed = !ops.is_empty();
    for op in ops {
        apply(ed, c, op);
    }
    if changed {
        ed.refocus = Some(ed.sel.clone());
    }
}

/// Font size, bold / italic / underline / strikethrough, colours, lists, alignment, indent and
/// clear formatting: the formatting the Message ribbon and the signature editor share.
fn format_buttons(ui: &mut Ui, t: &Tokens, c: &Compose, cur: &Style, para: rich::Para, ops: &mut Vec<Op>) {
    ui.add_enabled_ui(!c.plain, |ui| {
        egui::ComboBox::from_id_salt(("font-size", c.id)).width(48.0).selected_text(cur.size().to_string()).show_ui(ui, |ui| {
            for z in rich::SIZES {
                if ui.selectable_label(cur.size() == z, z.to_string()).clicked() {
                    ops.push(Op::Size(z));
                }
            }
        });
        ui.add_space(4.0);
        let letter = |bold: bool, italic: bool, under: bool, strike: bool| {
            let line = Stroke::new(1.2, t.text);
            TextFormat {
                font_id: if bold { theme::bold(15.0) } else { theme::regular(15.0) },
                color: t.text,
                italics: italic,
                underline: if under { line } else { Stroke::NONE },
                strikethrough: if strike { line } else { Stroke::NONE },
                ..Default::default()
            }
        };
        for (s, face, on, tip, op) in [
            ("B", letter(true, false, false, false), cur.bold, "Bold (Ctrl+B)", Op::Bold),
            ("I", letter(false, true, false, false), cur.italic, "Italic (Ctrl+I)", Op::Italic),
            ("U", letter(false, false, true, false), cur.underline, "Underline (Ctrl+U)", Op::Underline),
            ("ab", letter(false, false, false, true), cur.strike, "Strikethrough", Op::Strike),
        ] {
            if fmt_button(ui, t, Face::Letter(s, face), on, tip).clicked() {
                ops.push(op);
            }
        }
        if let Some(h) = color_button(ui, t, "ab", cur.highlight, &HIGHLIGHTS, "No color", "Text highlight color") {
            ops.push(Op::Highlight(h));
        }
        if let Some(col) = color_button(ui, t, "A", cur.color, &COLORS, "Automatic", "Font color") {
            ops.push(Op::Color(col));
        }
        ui.separator();
        for (ic, on, tip, op) in [
            (Icon::ListBullet, para.list == List::Bullet, "Bullets (Ctrl+Shift+L)", Op::Bullets),
            (Icon::ListNumber, para.list == List::Number, "Numbering", Op::Numbers),
            (Icon::IndentLess, false, "Decrease indent (Ctrl+Shift+T)", Op::Outdent),
            (Icon::IndentMore, false, "Increase indent (Ctrl+T)", Op::Indent),
            (Icon::AlignLeft, para.align == Align::Left, "Align left (Ctrl+L)", Op::Align(Align::Left)),
            (Icon::AlignCenter, para.align == Align::Center, "Center (Ctrl+E)", Op::Align(Align::Center)),
            (Icon::AlignRight, para.align == Align::Right, "Align right (Ctrl+R)", Op::Align(Align::Right)),
            (Icon::ClearFormat, false, "Clear all formatting (Ctrl+Space)", Op::Clear),
        ] {
            if fmt_button(ui, t, Face::Icon(ic), on, tip).clicked() {
                ops.push(op);
            }
        }
    });
}

/// The signature editor's bar (Outlook's signature editor has one): the formatting, a link, and
/// pictures (issue #11: signatures were thought to be plain text, their formatting there only to
/// paste or by its shortcuts).
pub(crate) fn signature_bar(ui: &mut Ui, t: &Tokens, ed: &mut Editor, c: &mut Compose, acts: &mut Vec<Action>) {
    c.styles.fit(&c.body);
    let n = c.body.chars().count();
    let sel = ed.sel.start.min(n)..ed.sel.end.min(n);
    let cur =
        ed.typing.clone().unwrap_or_else(|| if sel.is_empty() { typing_style(&c.body, &c.styles, sel.start) } else { c.styles.style(sel.start) });
    let para = c.styles.para(rich::lines_of(&c.body, sel.start..sel.start).0);
    let mut ops: Vec<Op> = Vec::new();
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        format_buttons(ui, t, c, &cur, para, &mut ops);
        ui.separator();
        if crate::widgets::tool_button(ui, Icon::Link, "Link", t, true).on_hover_text(tl!("Ctrl+K")).clicked() {
            ed.link = Some(LinkForm::open(c, sel.clone()));
        }
        if crate::widgets::tool_button(ui, Icon::Image, "Pictures", t, true).clicked() {
            acts.push(Action::SignaturePicture { paste: false });
        }
    });
    let changed = !ops.is_empty();
    for op in ops {
        apply(ed, c, op);
    }
    if changed {
        ed.refocus = Some(ed.sel.clone());
    }
}

/// The ways today's date and the time can go in: written out, as numbers, with the time, the
/// time alone.
pub(crate) fn date_choices(clock: &Clock) -> Vec<String> {
    let now = mailcraft_engine::model::dates::civil(clock.now, clock);
    let day = mailcraft_engine::model::dates::today(clock);
    let iso = format!("{:04}-{:02}-{:02}", now.year, now.month, now.day);
    let time = format!("{:02}:{:02}", now.hour, now.minute);
    vec![crate::calendar::long_date(day), iso.clone(), format!("{iso} {time}"), time]
}

/// What Word count shows: words, characters with and without spaces, paragraphs and lines.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub words: usize,
    pub chars: usize,
    pub chars_no_spaces: usize,
    pub paragraphs: usize,
    pub lines: usize,
}

/// Words in a run without spaces, as word processors count them: `2026-10-22` and `don't` are
/// one; in Chinese, Japanese and Korean each character is one (and the rest around them one each).
pub(super) fn token_words(token: &str) -> usize {
    let wide =
        |c: char| matches!(c as u32, 0x1100..=0x11FF | 0x2E80..=0x9FFF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF | 0xFF66..=0xFF9F | 0x20000..=0x2FA1F);
    let mut n = 0;
    let mut in_run = false;
    for c in token.chars() {
        if wide(c) {
            n += 1;
            in_run = false;
        } else if c.is_alphanumeric() && !in_run {
            n += 1;
            in_run = true;
        }
    }
    // Punctuation alone (a dash between words) is no word.
    n
}

pub(crate) fn count_words(text: &str) -> Counts {
    Counts {
        words: text.split_whitespace().map(token_words).sum(),
        chars: text.chars().filter(|c| *c != '\n' && *c != '\r').count(),
        chars_no_spaces: text.chars().filter(|c| !c.is_whitespace()).count(),
        paragraphs: text.split('\n').filter(|l| !l.trim().is_empty()).count(),
        lines: if text.is_empty() { 0 } else { text.split('\n').count() },
    }
}
