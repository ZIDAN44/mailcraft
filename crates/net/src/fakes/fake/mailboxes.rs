//! The commands on mailboxes: `LIST`, `SELECT` / `EXAMINE`, `CREATE`, `RENAME`, `DELETE` and
//! `APPEND`.

use super::*;

impl Fake {
    pub(super) fn list(&mut self, st: &State, tag: &str) {
        for b in &st.boxes {
            self.say(&format!("* LIST (\\HasNoChildren{}{}) \"/\" \"{}\"", if b.attr.is_empty() { "" } else { " " }, b.attr, b.name));
        }
        self.ok(tag, "");
    }

    /// `SELECT` or `EXAMINE`, with QRESYNC's changes since a modseq when asked.
    pub(super) fn select(&mut self, st: &mut State, tag: &str, rest: &str, upper: &str) {
        let name = quoted_arg(rest);
        let modseq = st.modseq;
        let permanent = st.permanent_flags;
        // `(QRESYNC (validity modseq …))`
        let qresync: Option<(u32, u64)> = upper.split("(QRESYNC (").nth(1).and_then(|r| {
            let mut w = r.split([' ', ')']);
            Some((w.next()?.parse().ok()?, w.next()?.parse().ok()?))
        });
        match st.mailbox(&name) {
            Some(b) => {
                let (n, v, next) = (b.msgs.len(), b.validity, b.uid_next);
                self.say(&format!("* {n} EXISTS"));
                self.say(&format!("* OK [UIDVALIDITY {v}] ok"));
                self.say(&format!("* OK [UIDNEXT {next}] ok"));
                self.say(&format!("* OK [HIGHESTMODSEQ {modseq}] ok"));
                if let Some(p) = permanent {
                    self.say(&format!("* OK [PERMANENTFLAGS ({p})] ok"));
                }
                if let Some((_, qm)) = qresync.filter(|(qv, _)| *qv == v) {
                    let gone: Vec<String> = b.expunged.iter().filter(|(_, m)| *m > qm).map(|(u, _)| u.to_string()).collect();
                    let changed: Vec<String> = b
                        .msgs
                        .iter()
                        .enumerate()
                        .filter(|(_, m)| m.modseq > qm)
                        .map(|(i, m)| format!("* {} FETCH (UID {} FLAGS ({}) MODSEQ ({}))", i + 1, m.uid, m.flags.join(" "), m.modseq))
                        .collect();
                    if !gone.is_empty() {
                        self.say(&format!("* VANISHED (EARLIER) {}", gone.join(",")));
                    }
                    for c in changed {
                        self.say(&c);
                    }
                }
                self.selected = Some(name);
                self.ok(tag, "[READ-WRITE] ");
            }
            None => {
                self.selected = None;
                self.say(&format!("{tag} NO no such mailbox"));
            }
        }
    }

    pub(super) fn create(&mut self, st: &mut State, tag: &str, rest: &str) {
        let name = quoted_arg(rest);
        if st.mailbox(&name).is_some() {
            self.say(&format!("{tag} NO [ALREADYEXISTS] exists"));
        } else if st.create_under.as_ref().is_some_and(|p| !name.starts_with(p.as_str())) {
            self.say(&format!("{tag} NO Client tried to access nonexistent namespace."));
        } else {
            let validity = 500 + u32::try_from(st.boxes.len()).unwrap_or(0);
            st.boxes.push(Mailbox::new(name, "", validity));
            self.ok(tag, "");
        }
    }

    /// `RENAME`: the mailbox and every one inside it.
    pub(super) fn rename(&mut self, st: &mut State, tag: &str, rest: &str) {
        let mut names = rest.split('"').skip(1).step_by(2).map(str::to_string);
        let (from, to) = (names.next().unwrap_or_default(), names.next().unwrap_or_default());
        if st.mailbox(&from).is_none() || st.mailbox(&to).is_some() {
            self.say(&format!("{tag} NO cannot rename"));
        } else {
            for b in &mut st.boxes {
                if b.name == from {
                    b.name = to.clone();
                } else if let Some(rest) = b.name.strip_prefix(&format!("{from}/")) {
                    b.name = format!("{to}/{rest}");
                }
            }
            self.ok(tag, "");
        }
    }

    pub(super) fn delete(&mut self, st: &mut State, tag: &str, rest: &str) {
        let name = quoted_arg(rest);
        let before = st.boxes.len();
        st.boxes.retain(|b| b.name != name);
        if st.boxes.len() == before {
            self.say(&format!("{tag} NO [NONEXISTENT] no such mailbox"));
        } else {
            self.ok(tag, "");
        }
    }

    /// `APPEND`: the message (stamped, when the server stamps what it stores) with its UID said.
    pub(super) fn append(&mut self, st: &mut State, tag: &str, rest: &str, literal: Option<Vec<u8>>) {
        let name = quoted_arg(rest);
        let flags = paren_arg(rest);
        st.modseq += 1;
        let modseq = st.modseq;
        let raw = match st.stamp_appends {
            Some(stamp) => [stamp.as_bytes(), b"\r\n", &literal.unwrap_or_default()].concat(),
            None => literal.unwrap_or_default(),
        };
        match st.mailbox(&name) {
            Some(b) => {
                let uid = b.uid_next;
                b.uid_next += 1;
                b.msgs.push(Msg { uid, flags, raw, modseq });
                let v = b.validity;
                self.ok(tag, &format!("[APPENDUID {v} {uid}] "));
            }
            None => self.say(&format!("{tag} NO [TRYCREATE] no such mailbox")),
        }
    }
}
