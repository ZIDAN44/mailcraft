//! Clean Up: the messages of a conversation that say nothing a later one doesn't, because a
//! reply quotes them in full. Only what can be shown redundant goes: a message whose text is
//! missing (not downloaded), one the user marked (unread, flagged, categorized, pinned), or one
//! whose attachments the reply doesn't carry stays.

use std::collections::HashSet;

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
/// pinned, no category: `categories` are their keywords, and any keyword that isn't a
/// [system one](crate::organize::keywords::is_system_keyword) counts as one too, as it's shown).
pub fn movable(m: &Summary, categories: &[String]) -> bool {
    m.flags.seen
        && !m.flags.flagged
        && !m.flags.deleted
        && !m.flags.has(crate::PINNED)
        && !categories.iter().any(|k| m.flags.has(k))
        && m.flags.keywords.iter().all(|k| crate::organize::keywords::is_system_keyword(k))
}

/// Which of a conversation's messages are redundant: `texts[i]` is message `i`'s text, when
/// downloaded. A message is redundant when a later reply to it (its Message-ID is in the reply's
/// `In-Reply-To` or `References`) quotes all of its text, as whole words, and, if it has
/// attachments, has some too. Later is as the list places them ([`Summary::when`]): the latest
/// message is never redundant, nor one without an id.
pub fn redundant(msgs: &[&Summary], texts: &[Option<String>], categories: &[String]) -> Vec<usize> {
    if msgs.len() > CONVERSATION_MAX {
        return Vec::new();
    }
    // Space-padded, so a match is of whole words.
    let plains: Vec<Option<String>> =
        texts.iter().map(|t| t.as_deref().filter(|t| t.len() <= TEXT_MAX).map(plain).filter(|p| !p.is_empty()).map(|p| format!(" {p} "))).collect();
    let answers = answers(msgs);
    let mut out = Vec::new();
    for (i, m) in msgs.iter().enumerate() {
        let Some(id) = may_go(m, categories) else { continue };
        let Some(Some(mine)) = plains.get(i) else { continue };
        let quoted = msgs.iter().zip(&answers).enumerate().any(|(j, (later, ids))| {
            j != i
                && may_quote(later, ids, m, &id)
                && plains.get(j).and_then(Option::as_ref).is_some_and(|theirs| theirs.len() > mine.len() && theirs.contains(mine.as_str()))
        });
        if quoted {
            out.push(i);
        }
    }
    out
}

/// Which of a conversation's messages [`redundant`] reads the text of: each that may go, when a
/// later message may quote it, and those later messages. The others' texts change nothing, so
/// they needn't be read (a conversation with none needs no text at all).
pub fn texts_wanted(msgs: &[&Summary], categories: &[String]) -> Vec<bool> {
    let mut wanted = vec![false; msgs.len()];
    if msgs.len() > CONVERSATION_MAX {
        return wanted;
    }
    let answers = answers(msgs);
    for (i, m) in msgs.iter().enumerate() {
        let Some(id) = may_go(m, categories) else { continue };
        for (j, (later, ids)) in msgs.iter().zip(&answers).enumerate() {
            if j != i && may_quote(later, ids, m, &id) {
                for k in [i, j] {
                    if let Some(w) = wanted.get_mut(k) {
                        *w = true;
                    }
                }
            }
        }
    }
    wanted
}

/// A message that may go, if a later one quotes it: [`movable`], with a Message-ID (its id,
/// normalized, as replies name it).
fn may_go(m: &Summary, categories: &[String]) -> Option<String> {
    if !movable(m, categories) {
        return None;
    }
    let id = crate::thread::norm_id(&m.message_id).to_ascii_lowercase();
    (!id.is_empty()).then_some(id)
}

/// The ids each message answers (its In-Reply-To and References), as [`may_go`] makes an id:
/// made once for a conversation, where each was made again for every message it was checked
/// against (one of 300, each naming the 256 before it, took half a second).
fn answers(msgs: &[&Summary]) -> Vec<HashSet<String>> {
    msgs.iter()
        .map(|m| std::iter::once(&m.in_reply_to).chain(&m.references).map(|r| crate::thread::norm_id(r).to_ascii_lowercase()).collect())
        .collect()
}

/// Whether `later` (which answers the ids `answers`) can stand for `m` (whose id is `id`) if it
/// quotes it: later as the list places them, a reply to it, and carrying attachments if `m` does.
fn may_quote(later: &Summary, answers: &HashSet<String>, m: &Summary, id: &str) -> bool {
    (later.when(), &later.key) > (m.when(), &m.key) && (!m.has_attachments || later.has_attachments) && answers.contains(id)
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

    /// Only the texts that can change the answer are wanted, and the answer with only those is
    /// the answer with all of them.
    #[test]
    fn only_texts_that_count_are_wanted() {
        let texts = vec![
            Some("Can you send the frames?\n\nZoe".to_string()),
            Some("Sure, Thursday.\n\n> Can you send the frames?\n>\n> Zoe".to_string()),
            Some("Great.\n> Sure, Thursday.\n>\n>> Can you send the frames?\n>>\n>> Zoe".to_string()),
        ];
        let (first, reply, last) = (msg(1, 100), msg(2, 200), msg(3, 300));
        let mut unread = first.clone();
        unread.flags.seen = false;
        let mut flagged = reply.clone();
        flagged.flags.flagged = true;
        let mut stranger = last.clone();
        (stranger.in_reply_to, stranger.references) = (String::new(), Vec::new());
        let cases: [([&Summary; 3], [bool; 3]); 5] = [
            ([&first, &reply, &last], [true, true, true]),
            // Nothing that may go: no text needed.
            ([&unread, &flagged, &last], [false, false, false]),
            // Only the first may go: it and the replies to it.
            ([&first, &flagged, &last], [true, true, true]),
            // The last answers nobody: the first and its reply.
            ([&first, &flagged, &stranger], [true, true, false]),
            ([&unread, &reply, &stranger], [false, false, false]),
        ];
        for (n, (msgs, want)) in cases.iter().enumerate() {
            let wanted = texts_wanted(msgs, &[]);
            assert_eq!(wanted, want, "{n}");
            let only: Vec<Option<String>> = texts.iter().zip(&wanted).map(|(t, w)| t.clone().filter(|_| *w)).collect();
            assert_eq!(redundant(msgs, &only, &[]), redundant(msgs, &texts, &[]), "{n}");
        }
    }

    /// Later is as the list places messages, by when they came: a reply from a computer whose
    /// clock is behind (its Date before the message it answers) still quotes it, and the
    /// message the list shows last is the one kept.
    #[test]
    fn later_is_as_the_list_places_them() {
        let texts = vec![Some("Can you send the frames?".to_string()), Some("Sure.\n> Can you send the frames?".to_string())];
        let (mut first, mut reply) = (msg(1, 500), msg(2, 400));
        (first.received, reply.received) = (510, 520);
        assert_eq!(redundant(&[&first, &reply], &texts, &[]), [0]);
        assert_eq!(texts_wanted(&[&first, &reply], &[]), [true, true]);
        // Dates alone (nothing says when they came): the reply's Date is the earlier.
        (first.received, reply.received) = (0, 0);
        assert!(redundant(&[&first, &reply], &texts, &[]).is_empty());
        assert_eq!(texts_wanted(&[&first, &reply], &[]), [false, false]);
    }

    /// The longest conversation looked at, each message naming the 256 before it as threading
    /// keeps them: which texts it wants, then what's redundant (`MEASURE`, release, by hand).
    #[test]
    #[ignore = "a measurement: run in release"]
    fn measure_a_long_conversation() {
        let msgs: Vec<Summary> = (1..=CONVERSATION_MAX as u32)
            .map(|uid| {
                let mut m = msg(uid, i64::from(uid) * 100);
                m.references = (uid.saturating_sub(256).max(1)..uid).map(|n| format!("<m{n}@x.example>")).collect();
                m
            })
            .collect();
        let all: Vec<&Summary> = msgs.iter().collect();
        let texts: Vec<Option<String>> = (0..msgs.len()).map(|i| Some(format!("Note {i}, on the frames."))).collect();
        let runs = 5;
        let ms = |t: std::time::Instant| t.elapsed().as_secs_f64() * 1000.0 / f64::from(runs);
        let t = std::time::Instant::now();
        for _ in 0..runs {
            assert!(texts_wanted(&all, &[]).iter().all(|w| *w));
        }
        let wanted = ms(t);
        let t = std::time::Instant::now();
        for _ in 0..runs {
            assert!(redundant(&all, &texts, &[]).is_empty());
        }
        println!("MEASURE clean up of {} messages: texts wanted {wanted:.1} ms, redundant {:.1} ms", msgs.len(), ms(t));
    }

    /// A keyword is a category here as everywhere else (shown as one, found by `is:categorized`)
    /// unless it's a system one: another program's `$label7` was taken for a system keyword, as
    /// every `$…` was, and its message moved.
    #[test]
    fn a_keyword_shown_as_a_category_keeps_its_message() {
        let with = |k: &str| {
            let mut m = msg(1, 100);
            m.flags.keywords.push(k.into());
            movable(&m, &[])
        };
        for category in ["$label7", "$Project", "Work"] {
            assert!(!with(category), "{category}");
        }
        for system in ["$Forwarded", "$Junk", "NonJunk", "$MailFlagBit1", "$MDNSent"] {
            assert!(with(system), "{system}");
        }
    }
}
