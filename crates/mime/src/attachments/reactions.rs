//! Reactions (RFC 9078, issue #39): an emoji answering a message, sent as a reply whose part
//! says `Content-Disposition: reaction`. Read from mail that carries one, and built to send.
//! Outlook's own reactions live only inside Exchange; these are the standard's, which any mail
//! server carries and other programs show.

use super::*;

/// The most a reaction's text is kept (an emoji sequence is a few characters).
const MAX_REACTION: usize = 32;

/// The reactions offered to send, as Outlook offers its six.
pub const REACTIONS: [&str; 6] = ["👍", "❤️", "🎉", "😆", "😮", "😢"];

/// Whether a reaction's text is emoji only (RFC 9078 §2: one or more emoji, separated by white
/// space): no letters, digits or markup, so a message can't pass words off as one.
pub fn is_reaction_text(text: &str) -> bool {
    let t = text.trim();
    let chars: Vec<char> = t.chars().collect();
    let emoji = |c: char| {
        matches!(c as u32, 0x200D | 0x20E3 | 0xFE0E | 0xFE0F | 0x2600..=0x27BF | 0x2B00..=0x2BFF | 0x1F000..=0x1FAFF | 0xE0020..=0xE007F)
            || matches!(c, '©' | '®' | '‼' | '⁉' | '™' | 'ℹ' | '↔'..='↙' | '↩' | '↪' | '⌚' | '⌛' | '⌨' | '⏏' | '⏩'..='⏺' | 'Ⓜ' | '▪' | '▫' | '▶' | '◀' | '◻'..='◾' | '〰' | '〽' | '㊗' | '㊙')
    };
    // A keycap: a digit, `#` or `*`, then the keycap's marks.
    let keycap = |i: usize| chars.get(i + 1).is_some_and(|n| matches!(*n, '\u{FE0F}' | '\u{20E3}'));
    !chars.is_empty()
        && chars.len() <= MAX_REACTION
        && chars.iter().enumerate().all(|(i, &c)| c.is_whitespace() || emoji(c) || (matches!(c, '0'..='9' | '#' | '*') && keycap(i)))
}

/// The reaction a message carries: the text of its part marked `Content-Disposition: reaction`
/// (the message itself, or one of its parts), when that is emoji only; empty otherwise.
pub(crate) fn reaction_of(m: &mail_parser::Message<'_>) -> String {
    for part in m.root().parts().take(16) {
        let marked = part.content_disposition().is_some_and(|d| d.ctype().eq_ignore_ascii_case("reaction"));
        if !marked || !matches!(part.kind(), PartKind::Text) {
            continue;
        }
        let Some(text) = part.text() else { continue };
        let text = text.trim();
        if is_reaction_text(text) {
            return text.split_whitespace().collect::<Vec<_>>().join(" ");
        }
    }
    String::new()
}

/// A reaction to `original` (its source parsed), from `from` at `date`: a reply to its sender
/// and everyone else it went to but `me` (as a reply to all, so they see it too), threaded
/// under it, its one part the emoji, marked a reaction.
pub fn reaction(original: &Parsed, emoji: &str, from: &EmailAddress, me: &[String], date: i64, message_id: &str) -> Result<Vec<u8>, String> {
    if !is_reaction_text(emoji) {
        return Err("a reaction is emoji only".into());
    }
    let mine = |a: &EmailAddress| me.iter().any(|m| m.eq_ignore_ascii_case(&a.email));
    let sender = original.reply_to.first().cloned().unwrap_or_else(|| original.from.clone());
    let mut to = vec![sender.clone()];
    let mut cc: Vec<EmailAddress> = Vec::new();
    for a in original.to.iter().chain(&original.cc) {
        if !mine(a) && !a.email.eq_ignore_ascii_case(&sender.email) && !cc.iter().any(|c| c.email.eq_ignore_ascii_case(&a.email)) {
            cc.push(a.clone());
        }
    }
    // Reacting to one's own message: to the others it went to.
    if mine(&sender) {
        to = std::mem::take(&mut cc);
    }
    if to.is_empty() {
        return Err("no one to send the reaction to".into());
    }
    let subject =
        if original.subject.to_ascii_lowercase().starts_with("re:") { original.subject.clone() } else { format!("Re: {}", original.subject) };
    let mut b = MessageBuilder::new().from(b_addr(from)).to(b_list(&to)).subject(subject).date(date.max(0));
    if !cc.is_empty() {
        b = b.cc(b_list(&cc));
    }
    if !message_id.trim().is_empty() {
        b = b.message_id(mailcraft_model::thread::norm_id(message_id));
    }
    let parent = mailcraft_model::thread::norm_id(&original.message_id);
    if !parent.is_empty() {
        b = b.in_reply_to(parent.clone());
        let mut refs: Vec<String> = original.references.iter().map(|r| mailcraft_model::thread::norm_id(r)).filter(|r| !r.is_empty()).collect();
        refs.push(parent);
        let keep = refs.len().saturating_sub(20);
        b = b.references(refs.split_off(keep));
    }
    let part = MimePart::new(ContentType::new("text/plain").attribute("charset", "utf-8"), emoji.trim().to_string())
        .header("Content-Disposition", mail_builder::headers::raw::Raw::new("reaction"));
    let mut out = Vec::new();
    b.body(part).write_to(&mut out).map_err(|e| e.to_string())?;
    Ok(out)
}
