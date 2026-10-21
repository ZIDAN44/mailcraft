//! Clean Up: the messages of a conversation that say nothing a later one doesn't, because a
//! reply quotes them in full. Only what can be shown redundant goes: a message whose text is
//! missing (not downloaded), one the user marked (unread, flagged, categorized, pinned), or one
//! whose attachments the reply doesn't carry stays.

use crate::Summary;

/// How much of a text is compared (a longer one is kept: not worth the time, nor the risk).
const TEXT_MAX: usize = 200_000;
/// The longest conversation looked at (each message is compared with each later one).
pub const CONVERSATION_MAX: usize = 300;

/// A text as compared: quote marks gone, case and spacing evened out.
pub fn plain(text: &str) -> String {
    let mut out = String::with_capacity(text.len().min(TEXT_MAX));
    for line in text.lines() {
        let line = line.trim_start_matches(|c: char| c == '>' || c.is_whitespace());
        for w in line.split_whitespace() {
            if !out.is_empty() {
                out.push(' ');
            }
            out.extend(w.chars().flat_map(char::to_lowercase));
        }
    }
    out
}

/// A message that a clean-up may move: nothing the user marked on it (read, not flagged nor
/// pinned, no category: `categories` are their keywords, and any keyword that isn't a system
/// one, `$…`, counts as one too).
pub fn movable(m: &Summary, categories: &[String]) -> bool {
    m.flags.seen
        && !m.flags.flagged
        && !m.flags.deleted
        && !m.flags.has(crate::PINNED)
        && !categories.iter().any(|k| m.flags.has(k))
        && !m.flags.keywords.iter().any(|k| !k.starts_with('$'))
}

/// Which of a conversation's messages are redundant: `texts[i]` is message `i`'s text, when
/// downloaded. A message is redundant when a later reply to it (its Message-ID is in the reply's
/// `In-Reply-To` or `References`) quotes all of its text, as whole words, and, if it has
/// attachments, has some too. The latest message is never redundant, nor one without an id.
pub fn redundant(msgs: &[&Summary], texts: &[Option<String>], categories: &[String]) -> Vec<usize> {
    if msgs.len() > CONVERSATION_MAX {
        return Vec::new();
    }
    // Space-padded, so a match is of whole words.
    let plains: Vec<Option<String>> =
        texts.iter().map(|t| t.as_deref().filter(|t| t.len() <= TEXT_MAX).map(plain).filter(|p| !p.is_empty()).map(|p| format!(" {p} "))).collect();
    let mut out = Vec::new();
    for (i, m) in msgs.iter().enumerate() {
        if !movable(m, categories) {
            continue;
        }
        let Some(Some(mine)) = plains.get(i) else { continue };
        let id = crate::thread::norm_id(&m.message_id).to_ascii_lowercase();
        if id.is_empty() {
            continue;
        }
        let quoted = msgs.iter().enumerate().any(|(j, later)| {
            j != i
                && (later.date, &later.key) > (m.date, &m.key)
                && (!m.has_attachments || later.has_attachments)
                && std::iter::once(&later.in_reply_to).chain(&later.references).any(|r| crate::thread::norm_id(r).eq_ignore_ascii_case(&id))
                && plains.get(j).and_then(Option::as_ref).is_some_and(|theirs| theirs.len() > mine.len() && theirs.contains(mine.as_str()))
        });
        if quoted {
            out.push(i);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Flags, MessageKey};

    fn msg(uid: u32, date: i64) -> Summary {
        Summary {
            key: MessageKey { account: "a".into(), folder: "INBOX".into(), validity: 1, uid },
            message_id: format!("m{uid}@x.example"),
            // Each answers the one before, naming the whole chain.
            in_reply_to: if uid > 1 { format!("<m{}@x.example>", uid - 1) } else { String::new() },
            references: (1..uid).map(|n| format!("<m{n}@x.example>")).collect(),
            date,
            flags: Flags { seen: true, ..Default::default() },
            ..Default::default()
        }
    }

    #[test]
    fn only_messages_quoted_in_full_later_are_redundant() {
        let first = msg(1, 100);
        let reply = msg(2, 200);
        let last = msg(3, 300);
        let texts = vec![
            Some("Can you send the frames?\n\nZoe".to_string()),
            Some("Sure, Thursday.\n\nOn Mon Zoe wrote:\n> Can you send the frames?\n>\n> Zoe".to_string()),
            Some("Great.\n> Sure, Thursday.\n>\n> On Mon Zoe wrote:\n>> Can you  SEND the frames?\n>>\n>> Zoe".to_string()),
        ];
        let all = [&first, &reply, &last];
        assert_eq!(redundant(&all, &texts, &[]), [0, 1]);
        // Order given doesn't matter: by date.
        assert_eq!(redundant(&[&last, &first, &reply], &[texts[2].clone(), texts[0].clone(), texts[1].clone()], &[]), [1, 2]);
        // A message only partly quoted, or not downloaded, stays.
        let partial = vec![Some("Can you send the frames? And the key.".to_string()), texts[1].clone(), None];
        assert!(redundant(&all, &partial, &[]).is_empty());
        // Marked: unread, flagged, pinned, a category; attachments the reply lacks.
        for change in 0..5 {
            let mut m = first.clone();
            match change {
                0 => m.flags.seen = false,
                1 => m.flags.flagged = true,
                2 => m.flags.keywords.push(crate::PINNED.into()),
                3 => m.flags.keywords.push("$label2".into()),
                _ => m.has_attachments = true,
            }
            assert_eq!(redundant(&[&m, &reply, &last], &texts, &["$label2".to_string()]), [1], "{change}");
        }
        // An empty text, or the same text sent twice: nothing to say either is the one to drop
        // (only a longer one quotes another).
        let same = vec![Some("Thanks!".to_string()), Some("Thanks!".to_string()), Some("  ".to_string())];
        assert!(redundant(&all, &same, &[]).is_empty());
        // Words, not letters: "Ok" isn't quoted by "book"; and only a reply in its chain counts.
        let short = vec![Some("Ok".to_string()), Some("I'll book it".to_string()), None];
        assert!(redundant(&all, &short, &[]).is_empty());
        let mut stranger = reply.clone();
        (stranger.in_reply_to, stranger.references) = (String::new(), Vec::new());
        assert!(redundant(&[&first, &stranger], &texts[..2], &[]).is_empty());
        let mut anonymous = first.clone();
        anonymous.message_id.clear();
        assert!(redundant(&[&anonymous, &reply], &texts[..2], &[]).is_empty());
        // A huge text isn't compared.
        let huge = vec![Some("x ".repeat(TEXT_MAX)), Some(format!("y {}", "x ".repeat(TEXT_MAX))), None];
        assert!(redundant(&all, &huge, &[]).is_empty());
    }
}
