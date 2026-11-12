//! A reply or forward of a received message: its subject, recipients and threading, and the
//! original quoted under the text the way Outlook lays it out.

use mailcraft_model::EmailAddress;
use mailcraft_model::dates::{Clock, full_time};
use serde::{Deserialize, Serialize};

use crate::{Draft, Parsed, layout, rich};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReplyMode {
    Reply,
    ReplyAll,
    Forward,
}

fn prefixed(prefix: &str, subject: &str) -> String {
    let base = mailcraft_model::thread::without_prefixes(subject);
    let already = subject.trim().to_ascii_lowercase().starts_with(&prefix.to_ascii_lowercase());
    if already { subject.trim().to_string() } else { format!("{prefix} {base}") }
}

/// The original under a reply or forward, the way Outlook lays it out: a rule, then who sent it
/// to whom and when (labels in bold), then (`body`) the original with its own formatting.
fn quoted_original(p: &Parsed, clock: &Clock, body: bool) -> (String, rich::Styles) {
    let mut lines: Vec<(&str, String)> = vec![("From:", p.from.to_string()), ("Sent:", full_time(p.date, clock))];
    if !p.to.is_empty() {
        lines.push(("To:", mailcraft_model::addr::format_list(&p.to)));
    }
    if !p.cc.is_empty() {
        lines.push(("Cc:", mailcraft_model::addr::format_list(&p.cc)));
    }
    lines.push(("Subject:", p.subject.clone()));
    // The rule: an empty paragraph drawn as a line.
    let mut text = String::new();
    let mut styles = rich::Styles::plain("");
    if let Some(first) = styles.paras.first_mut() {
        first.rule = true;
    }
    let bold = rich::Style { bold: true, ..Default::default() };
    for (label, value) in lines {
        let line = format!("\n{label} {value}");
        let mut st = rich::Styles::plain(&line);
        st.restyle(&line, 1..1 + label.chars().count(), |_| bold.clone());
        rich::append(&mut text, &mut styles, &line, &st);
    }
    if body {
        let (orig, ost) = quoted_body(p);
        rich::append(
            &mut text,
            &mut styles,
            "

",
            &rich::Styles::plain(
                "

",
            ),
        );
        rich::append(&mut text, &mut styles, &orig, &ost);
    }
    (text, styles)
}

/// A message's body as text and styles to edit, with its own formatting. Mail laid out in tables
/// goes as its cells one after another: its text drawn as a table (`rich`) would come out as lines
/// of `│` and `├──` to edit.
pub fn quoted_body(p: &Parsed) -> (String, rich::Styles) {
    if !p.layout.is_empty() {
        layout::flatten(&p.layout)
    } else if p.had_html && !p.rich.is_empty() {
        rich::from_rich(&p.rich)
    } else {
        (p.text.clone(), rich::Styles::plain(&p.text))
    }
}

/// A reply or forward of `p`, as plain text. `me` are the user's own addresses, left out of
/// Reply All.
pub fn reply(p: &Parsed, mode: ReplyMode, me: &[String], clock: &Clock, signature: &str) -> Draft {
    let (mut d, styles) = reply_doc(p, mode, me, clock, signature);
    d.body = rich::to_text(&d.body, &styles);
    d
}

/// A reply or forward of `p` to edit: the draft (its body the editable text) and the text's
/// styles.
pub fn reply_doc(p: &Parsed, mode: ReplyMode, me: &[String], clock: &Clock, signature: &str) -> (Draft, rich::Styles) {
    reply_doc_with(p, mode, me, clock, signature, true)
}

/// [`reply_doc`] without the original's body: who sent it to whom and when end the text, the
/// original going after it as it came (its own HTML, shown as the reading pane shows it).
pub fn reply_head_doc(p: &Parsed, mode: ReplyMode, me: &[String], clock: &Clock, signature: &str) -> (Draft, rich::Styles) {
    reply_doc_with(p, mode, me, clock, signature, false)
}

fn reply_doc_with(p: &Parsed, mode: ReplyMode, me: &[String], clock: &Clock, signature: &str, with_body: bool) -> (Draft, rich::Styles) {
    let mine = |a: &EmailAddress| me.iter().any(|m| m.eq_ignore_ascii_case(a.email.trim()));
    let sig = if signature.trim().is_empty() { String::new() } else { format!("\n\n-- \n{}", signature.trim_end()) };
    // The cursor's line, the signature, a blank line, then the original under its rule.
    let mut body = sig;
    let mut styles = rich::Styles::plain(&body);
    rich::append(&mut body, &mut styles, "\n\n", &rich::Styles::plain("\n\n"));
    let (orig, ost) = quoted_original(p, clock, with_body);
    rich::append(&mut body, &mut styles, &orig, &ost);
    let mut d = Draft::default();
    match mode {
        ReplyMode::Forward => {
            d.subject = prefixed("Fwd:", &p.subject);
        }
        ReplyMode::Reply | ReplyMode::ReplyAll => {
            d.subject = prefixed("Re:", &p.subject);
            let primary = if p.reply_to.is_empty() { vec![p.from.clone()] } else { p.reply_to.clone() };
            // Replying to your own sent message goes back to its recipients.
            d.to = if primary.iter().all(&mine) && !p.to.is_empty() { p.to.clone() } else { primary };
            if mode == ReplyMode::ReplyAll {
                let mut cc: Vec<EmailAddress> = Vec::new();
                for a in p.to.iter().chain(&p.cc) {
                    let dup = d.to.iter().chain(&cc).any(|x| x.email.eq_ignore_ascii_case(&a.email));
                    if !mine(a) && !dup {
                        cc.push(a.clone());
                    }
                }
                d.cc = cc;
            }
            if !p.message_id.is_empty() {
                d.in_reply_to = p.message_id.clone();
                d.references = p.references.clone();
                // A parent with no References: what it answers leads them (RFC 5322 §3.6.4).
                if d.references.is_empty() && !p.in_reply_to.is_empty() {
                    d.references.push(p.in_reply_to.clone());
                }
                d.references.push(p.message_id.clone());
                // RFC 5322 §3.6.4 allows trimming; keep the first and the last 20.
                if d.references.len() > 21 {
                    let tail = d.references.split_off(d.references.len() - 20);
                    d.references.truncate(1);
                    d.references.extend(tail);
                }
            }
        }
    }
    d.body = body;
    (d, styles)
}
