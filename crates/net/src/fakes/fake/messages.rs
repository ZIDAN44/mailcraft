//! The commands on the selected mailbox's messages: `UID SEARCH`, `FETCH`, `STORE`, `MOVE`,
//! `COPY` and `EXPUNGE`, and `EXPUNGE`, `NOOP` and `LOGOUT`.

use super::*;

/// What a command on the selected mailbox did: its untagged answers, and the mail it files in
/// another mailbox (`dest`), copied from one of this UIDVALIDITY (`copied_from`, else 0).
#[derive(Default)]
struct Done {
    lines: Vec<Vec<u8>>,
    moved: Vec<Msg>,
    dest: String,
    copied_from: u32,
    /// A command the fake doesn't know: `BAD`.
    bad: bool,
}

impl Fake {
    /// `UID …`, `EXPUNGE`, `NOOP` and `LOGOUT`.
    pub(super) fn on_selected(&mut self, st: &mut State, tag: &str, rest: &str, upper: &str, literal: Option<Vec<u8>>) {
        let sel = self.selected.clone().unwrap_or_default();
        let modseq_now = st.modseq;
        let permanent = st.permanent_flags;
        // Gmail mode: an EXPUNGE in a label folder (anything but Trash/Spam) drops the label
        // and archives the message into "[Gmail]/All Mail".
        let labels = st.caps.contains("X-GM-EXT-1");
        // A move or copy into a mailbox that isn't there is refused, and nothing moves (RFC
        // 3501 §6.4.7, RFC 6851 §3.3), as APPEND's is.
        let verb = rest.split_whitespace().nth(1).map(str::to_ascii_uppercase).unwrap_or_default();
        if upper.starts_with("UID ")
            && (verb == "MOVE" || verb == "COPY")
            && st.boxes.iter().any(|b| b.name == sel)
            && st.mailbox(&quoted_arg(rest)).is_none()
        {
            self.say(&format!("{tag} NO [TRYCREATE] no such mailbox"));
            return;
        }
        let Some(b) = st.boxes.iter_mut().find(|b| b.name == sel) else {
            if upper == "NOOP" || upper == "LOGOUT" {
                self.ok(tag, "");
            } else {
                self.say(&format!("{tag} BAD no mailbox selected"));
            }
            return;
        };
        b.msgs.sort_by_key(|m| m.uid);
        let max = b.msgs.last().map_or(0, |m| m.uid);
        let words: Vec<&str> = rest.split_whitespace().collect();
        let done = if upper == "NOOP" {
            // What a real server says to a NOOP: the folder's size, now.
            Done { lines: vec![format!("* {} EXISTS", b.msgs.len()).into_bytes()], ..Done::default() }
        } else if upper == "LOGOUT" {
            Done::default()
        } else if verb == "SEARCH" {
            search(b, rest, literal.as_deref(), max)
        } else if verb == "FETCH" {
            fetch(b, &words, rest, upper, max)
        } else if verb == "STORE" {
            store(b, &words, rest, permanent, modseq_now, max);
            Done::default()
        } else if verb == "MOVE" || verb == "COPY" {
            move_or_copy(b, &verb, &words, rest, modseq_now, max)
        } else if verb == "EXPUNGE" || upper == "EXPUNGE" {
            expunge(b, &words, labels, modseq_now, max)
        } else {
            Done { bad: true, ..Done::default() }
        };
        let Done { lines, moved, dest, copied_from, bad } = done;
        let mut copyuid = String::new();
        if !moved.is_empty() || !dest.is_empty() {
            st.modseq += 1;
            let ms = st.modseq;
            if let Some(d) = st.mailbox(&dest) {
                let (mut from, mut to) = (Vec::new(), Vec::new());
                for mut m in moved {
                    from.push(m.uid.to_string());
                    m.uid = d.uid_next;
                    to.push(m.uid.to_string());
                    d.uid_next += 1;
                    m.modseq = ms;
                    d.msgs.push(m);
                }
                if copied_from != 0 && !from.is_empty() {
                    copyuid = format!("[COPYUID {} {} {}] ", d.validity, from.join(","), to.join(","));
                }
            }
        } else if upper.contains("STORE") || upper.contains("EXPUNGE") {
            st.modseq += 1;
        }
        for l in lines {
            self.out.extend(l);
            self.out.extend(b"\r\n");
        }
        if bad {
            self.say(&format!("{tag} BAD unsupported"));
        } else {
            if upper == "LOGOUT" {
                self.say("* BYE bye");
            }
            self.ok(tag, &copyuid);
        }
    }
}

/// `UID SEARCH`: the keys after it, a literal last (as MailCraft sends a UTF-8 word).
fn search(b: &Mailbox, rest: &str, literal: Option<&[u8]>, max: u32) -> Done {
    let after = rest.splitn(3, ' ').nth(2).unwrap_or("");
    let mut keys = search_tokens(after);
    if let Some(l) = literal {
        keys.push(String::from_utf8_lossy(l).into_owned());
    }
    let exists = u32::try_from(b.msgs.len()).unwrap_or(u32::MAX);
    let hits: Vec<String> = b
        .msgs
        .iter()
        .enumerate()
        .filter(|(i, m)| search_matches(m, u32::try_from(*i + 1).unwrap_or(0), &keys, max, exists))
        .map(|(_, m)| m.uid.to_string())
        .collect();
    Done { lines: vec![format!("* SEARCH {}\r\n", hits.join(" ")).trim_end().as_bytes().to_vec()], ..Done::default() }
}

/// `UID FETCH`: what's asked of each message in the set (changed since a modseq, if asked).
fn fetch(b: &Mailbox, words: &[&str], rest: &str, upper: &str, max: u32) -> Done {
    let set = words.get(2).copied().unwrap_or("");
    let since: u64 = rest.split("CHANGEDSINCE ").nth(1).and_then(|s| s.trim_end_matches(')').trim().parse().ok()).unwrap_or(0);
    let mut lines = Vec::new();
    let (bodystructure, sections) = (upper.contains("BODYSTRUCTURE"), structure::asked_sections(upper));
    for (i, m) in b.msgs.iter().enumerate() {
        if !in_set(m.uid, set, max) || m.modseq <= since {
            continue;
        }
        let mut item = format!("* {} FETCH (UID {} FLAGS ({}) MODSEQ ({})", i + 1, m.uid, m.flags.join(" "), m.modseq).into_bytes();
        if upper.contains("RFC822.SIZE") {
            item.extend(format!(" RFC822.SIZE {}", m.raw.len()).as_bytes());
        }
        if upper.contains("INTERNALDATE") {
            item.extend(b" INTERNALDATE \"14-Oct-2026 09:30:00 +0000\"");
        }
        if upper.contains("BODY.PEEK[HEADER]") {
            let end = m.raw.windows(4).position(|w| w == b"\r\n\r\n").map_or(m.raw.len(), |p| p + 4);
            item.extend(format!(" BODY[HEADER] {{{end}}}\r\n").as_bytes());
            item.extend(&m.raw[..end]);
        }
        // Its parts, split once, when something asked needs them.
        let parts = (bodystructure || !sections.is_empty()).then(|| structure::Part::of(&m.raw));
        if let Some(p) = parts.as_ref().filter(|_| bodystructure) {
            item.extend(format!(" BODYSTRUCTURE {}", p.structure()).as_bytes());
        }
        for (spec, path, mime) in &sections {
            if let Some(data) = parts.as_ref().and_then(|p| p.section(path, *mime)) {
                item.extend(format!(" BODY[{spec}] {{{}}}\r\n", data.len()).as_bytes());
                item.extend(&data);
            }
        }
        if upper.contains("BODY.PEEK[]") {
            item.extend(format!(" BODY[] {{{}}}\r\n", m.raw.len()).as_bytes());
            item.extend(&m.raw);
        }
        item.push(b')');
        lines.push(item);
    }
    Done { lines, ..Done::default() }
}

/// `UID STORE`: flags put on or taken off; a keyword the mailbox can't keep is forgotten.
fn store(b: &mut Mailbox, words: &[&str], rest: &str, permanent: Option<&str>, modseq_now: u64, max: u32) {
    let set = words.get(2).copied().unwrap_or("");
    let add = words.get(3).is_some_and(|w| w.starts_with('+'));
    let mut flags = paren_arg(rest);
    if let Some(p) = permanent.filter(|p| !p.split(' ').any(|k| k == r"\*")) {
        flags.retain(|f| f.starts_with('\\') || p.split(' ').any(|k| k.eq_ignore_ascii_case(f)));
    }
    for m in b.msgs.iter_mut().filter(|m| in_set(m.uid, set, max)) {
        for f in &flags {
            m.flags.retain(|x| !x.eq_ignore_ascii_case(f));
            if add {
                m.flags.push(f.clone());
            }
        }
        m.modseq = modseq_now + 1;
    }
}

/// `UID MOVE` (out of this mailbox) or `UID COPY` (a copy of each): filed by the caller.
fn move_or_copy(b: &mut Mailbox, verb: &str, words: &[&str], rest: &str, modseq_now: u64, max: u32) -> Done {
    let set = words.get(2).copied().unwrap_or("");
    let moved = if verb == "MOVE" {
        let (go, stay): (Vec<Msg>, Vec<Msg>) = std::mem::take(&mut b.msgs).into_iter().partition(|m| in_set(m.uid, set, max));
        b.msgs = stay;
        b.expunged.extend(go.iter().map(|m| (m.uid, modseq_now + 1)));
        go
    } else {
        b.msgs.iter().filter(|m| in_set(m.uid, set, max)).map(|m| Msg { uid: m.uid, flags: m.flags.clone(), raw: m.raw.clone(), modseq: 0 }).collect()
    };
    Done { moved, dest: quoted_arg(rest), copied_from: b.validity, ..Done::default() }
}

/// `UID EXPUNGE` (the set) or `EXPUNGE` (all): what's marked `\Deleted` goes; in Gmail mode, from
/// a label folder into All Mail.
fn expunge(b: &mut Mailbox, words: &[&str], labels: bool, modseq_now: u64, max: u32) -> Done {
    let set = words.get(2).copied().unwrap_or("1:*");
    let gone: Vec<u32> = b.msgs.iter().filter(|m| in_set(m.uid, set, max) && m.flags.iter().any(|f| f == "\\Deleted")).map(|m| m.uid).collect();
    let mut done = Done::default();
    if labels && !matches!(b.attr, "\\Trash" | "\\Junk") {
        done.moved =
            b.msgs.iter().filter(|m| gone.contains(&m.uid)).map(|m| Msg { uid: m.uid, flags: Vec::new(), raw: m.raw.clone(), modseq: 0 }).collect();
        done.dest = "[Gmail]/All Mail".into();
    }
    b.msgs.retain(|m| !gone.contains(&m.uid));
    b.expunged.extend(gone.iter().map(|u| (*u, modseq_now + 1)));
    done
}
