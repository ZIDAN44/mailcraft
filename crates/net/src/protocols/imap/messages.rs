//! Messages by UID: searched, fetched, flagged, moved, expunged and appended.

use super::*;

/// The UIDs a `UID SEARCH` found, in order, each once.
pub(super) fn searched(rs: Vec<Response<'static>>) -> Vec<u32> {
    let mut out = Vec::new();
    for r in rs {
        if let Response::MailboxData(MailboxDatum::Search(v)) = r {
            out.extend(v);
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

/// The destination UIDs of a `COPYUID` code among a command's responses (at most 100,000).
pub(super) fn copied_uids(rs: &[Response<'_>]) -> Vec<u32> {
    let mut out = Vec::new();
    for r in rs {
        let outcome = match r {
            Response::Data { status: Status::Ok, outcome } | Response::Done { status: Status::Ok, outcome, .. } => outcome,
            _ => continue,
        };
        if let Some(ResponseCode::CopyUid(_, _, to)) = &outcome.code {
            for m in to {
                match m {
                    imap_proto::rfc4315::UidSetMember::Uid(u) => out.push(*u),
                    imap_proto::rfc4315::UidSetMember::UidRange(r) => out.extend(r.clone().take(100_000usize.saturating_sub(out.len()))),
                }
                if out.len() >= 100_000 {
                    return out;
                }
            }
        }
    }
    out
}

/// One message from a `UID FETCH`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Fetched {
    pub uid: u32,
    pub flags: Option<Vec<String>>,
    pub size: u32,
    pub internal_date: Option<i64>,
    pub header: Option<Vec<u8>>,
    pub body: Option<Vec<u8>>,
    pub modseq: u64,
}

impl<S: Read + Write> Client<S> {
    /// `UID SEARCH` with these arguments: strings quoted, or sent as literals in UTF-8 (with
    /// `CHARSET UTF-8`, RFC 3501 §6.4.4) when they aren't plain ASCII.
    pub fn uid_search_args(&mut self, args: &[crate::sync::SearchArg]) -> Result<Vec<u32>> {
        use crate::sync::SearchArg;
        let utf8 = args.iter().any(|a| matches!(a, SearchArg::Str(s) if quoted(s).is_none()));
        // The command's text between its literals, and the literals.
        let mut texts: Vec<String> = vec![if utf8 { "UID SEARCH CHARSET UTF-8".into() } else { "UID SEARCH".into() }];
        let mut literals: Vec<&[u8]> = Vec::new();
        for a in args {
            let (SearchArg::Atom(x) | SearchArg::Str(x)) = a;
            let text = match (a, quoted(x)) {
                (SearchArg::Atom(_), _) => format!(" {x}"),
                (SearchArg::Str(_), Some(q)) => format!(" {q}"),
                (SearchArg::Str(_), None) => {
                    if let Some(t) = texts.last_mut() {
                        t.push(' ');
                    }
                    literals.push(x.as_bytes());
                    texts.push(String::new());
                    continue;
                }
            };
            if let Some(t) = texts.last_mut() {
                t.push_str(&text);
            }
        }
        let mut parts: Vec<Part<'_>> = Vec::new();
        for (i, t) in texts.iter().enumerate() {
            if !t.is_empty() {
                parts.push(Part::Text(t));
            }
            if let Some(l) = literals.get(i) {
                parts.push(Part::Literal(l));
            }
        }
        Ok(searched(self.cmd(&parts)?))
    }

    pub fn uid_search(&mut self, criteria: &str) -> Result<Vec<u32>> {
        let cmd = format!("UID SEARCH {criteria}");
        Ok(searched(self.cmd(&[Part::Text(&cmd)])?))
    }

    /// `UID FETCH set (items)` with an optional modifier (`(CHANGEDSINCE n)`).
    pub fn uid_fetch(&mut self, set: &str, items: &str, modifier: &str) -> Result<Vec<Fetched>> {
        if set.is_empty() {
            return Ok(Vec::new());
        }
        let cmd = if modifier.is_empty() { format!("UID FETCH {set} ({items})") } else { format!("UID FETCH {set} ({items}) {modifier}") };
        let rs = self.cmd(&[Part::Text(&cmd)])?;
        let mut out: Vec<Fetched> = Vec::new();
        // Each UID's place in `out` (looked for in it, a big folder's flags were a pass over the
        // answer per message).
        let mut at: HashMap<u32, usize> = HashMap::new();
        for r in rs {
            let Response::Fetch(_, attrs) = r else { continue };
            let f = fetched(attrs);
            // Unsolicited FETCHes without a UID (flag pushes for other messages) are skipped.
            if f.uid == 0 {
                continue;
            }
            if let Some(existing) = at.get(&f.uid).and_then(|&i| out.get_mut(i)) {
                existing.flags = f.flags.or(existing.flags.take());
                existing.header = f.header.or(existing.header.take());
                existing.body = f.body.or(existing.body.take());
                existing.size = existing.size.max(f.size);
                existing.modseq = existing.modseq.max(f.modseq);
                existing.internal_date = existing.internal_date.or(f.internal_date);
            } else {
                at.insert(f.uid, out.len());
                out.push(f);
            }
        }
        Ok(out)
    }

    /// Add (`add`) or remove flags, silently.
    pub fn uid_store(&mut self, set: &str, add: bool, flags: &str) -> Result<()> {
        if set.is_empty() {
            return Ok(());
        }
        let cmd = format!("UID STORE {set} {}FLAGS.SILENT ({flags})", if add { "+" } else { "-" });
        self.cmd(&[Part::Text(&cmd)]).map(|_| ())
    }

    /// Move messages: `UID MOVE` (RFC 6851) or COPY + `\Deleted` + expunge.
    pub fn uid_move(&mut self, set: &str, dest: &str) -> Result<()> {
        self.uid_move_to(set, dest).map(drop)
    }

    /// Move messages and say their UIDs in `dest`, when the server does (UIDPLUS `COPYUID`, sent
    /// with MOVE's untagged OK or COPY's tagged one); empty when it doesn't.
    pub fn uid_move_to(&mut self, set: &str, dest: &str) -> Result<Vec<u32>> {
        if set.is_empty() {
            return Ok(Vec::new());
        }
        let dest = mailbox_arg(dest);
        if self.has("MOVE") {
            let cmd = format!("UID MOVE {set} {dest}");
            return self.cmd(&[Part::Text(&cmd)]).map(|rs| copied_uids(&rs));
        }
        let cmd = format!("UID COPY {set} {dest}");
        let uids = copied_uids(&self.cmd(&[Part::Text(&cmd)])?);
        self.uid_store(set, true, "\\Deleted")?;
        self.expunge(set)?;
        Ok(uids)
    }

    /// Expunge exactly `set` (UIDPLUS), or everything marked `\Deleted` without it.
    pub fn expunge(&mut self, set: &str) -> Result<()> {
        if self.has("UIDPLUS") {
            return self.cmd(&[Part::Text(&format!("UID EXPUNGE {set}"))]).map(|_| ());
        }
        // Plain EXPUNGE takes everything marked `\Deleted`: what another program marked and left
        // (Thunderbird's "just mark it as deleted") is unmarked for the moment, so it stays.
        let others = uid_set(&self.uid_search(&format!("DELETED NOT UID {set}"))?);
        self.uid_store(&others, false, "\\Deleted")?;
        let done = self.cmd(&[Part::Text("EXPUNGE")]).map(|_| ());
        self.uid_store(&others, true, "\\Deleted")?;
        done
    }

    /// Store a message in a folder; returns its UID when the server says (UIDPLUS `APPENDUID`).
    pub fn append(&mut self, folder: &str, flags: &str, raw: &[u8]) -> Result<Option<u32>> {
        self.append_dated(folder, flags, None, raw)
    }

    /// [`Client::append`], the message received at `date` (Unix seconds) as far as the server
    /// is to know: mail moved in from elsewhere keeps the day it came, where it was all new
    /// today to every program that sorts by that.
    pub fn append_dated(&mut self, folder: &str, flags: &str, date: Option<i64>, raw: &[u8]) -> Result<Option<u32>> {
        let date = date.and_then(internal_date).map(|d| format!("\"{d}\" ")).unwrap_or_default();
        let head = format!("APPEND {} ({flags}) {date}", mailbox_arg(folder));
        let rs = self.cmd(&[Part::Text(&head), Part::Literal(raw)])?;
        for r in rs {
            if let Response::Done { outcome, .. } = r
                && let Some(ResponseCode::AppendUid(_, uids)) = outcome.code
            {
                return Ok(uids.first().map(|m| match m {
                    imap_proto::rfc4315::UidSetMember::Uid(u) => *u,
                    imap_proto::rfc4315::UidSetMember::UidRange(r) => *r.start(),
                }));
            }
        }
        Ok(None)
    }
}

/// One FETCH response's attributes.
pub(super) fn fetched(attrs: Vec<AttributeValue<'_>>) -> Fetched {
    let mut f = Fetched::default();
    for a in attrs {
        match a {
            AttributeValue::Uid(u) => f.uid = u,
            AttributeValue::Flags(fl) => f.flags = Some(fl.iter().map(|x| x.to_string()).collect()),
            AttributeValue::Rfc822Size(n) => f.size = n,
            AttributeValue::InternalDate(d) => f.internal_date = parse_internal_date(&d),
            AttributeValue::ModSeq(m) => f.modseq = m,
            AttributeValue::BodySection { section: None, data, .. } | AttributeValue::Rfc822(data) => f.body = data.map(|d| d.into_owned()),
            AttributeValue::BodySection { section: Some(_), data, .. } | AttributeValue::Rfc822Header(data) => {
                f.header = data.map(|d| d.into_owned())
            }
            _ => {}
        }
    }
    f
}
