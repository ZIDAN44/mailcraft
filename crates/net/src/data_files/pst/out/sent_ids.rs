//! Sent items' own message ids, found in the replies to them. Outlook keeps no Message-ID for
//! the mail it sent, but a reply to a sent message that came back into the same file names it in
//! its In-Reply-To, and the reply's conversation index is the sent message's with one block more
//! (MS-OXOMSG 2.2.1.3). So as the import goes, the writer keeps each reply's parent index (as a
//! hash) with the id it answers, and the real id of every message read; the Sent Items folder is
//! read last (`plan`), and a sent item with no id of its own whose index a reply answered gets
//! that id there, unless a message read has it already.
//!
//! Nothing is guessed: only an id a reply used is given, an index replies answer with two ids
//! gives none, and an id is given once (the writer takes back one an earlier message has, in
//! the order the planner gave). What's kept is capped at [`MAX_BYTES`]: past it nothing more is,
//! and no id is given at all, since a message read past it could carry the one given. The sent
//! items' own ids, and those given, are kept past it (the folder's rows bound them), so a
//! checkpoint taken while they're read reads back as it was, and the import goes on the same.

use std::collections::{HashMap, HashSet};

use mailcraft_store::search::msgid_hash;

use super::super::item::Item;
use super::super::lanes::Built;
use super::super::resume::{SentMap, SentReply};
use super::*;

/// What the replies and ids kept may come to, counted as below.
pub(in super::super) const MAX_BYTES: usize = 16 << 20;
/// A reply kept is counted at its id's bytes and this (its key, its slot in the map); an id
/// seen, at this.
const REPLY_BYTES: usize = 48;
const SEEN_BYTES: usize = 16;
/// Ends the ids MailCraft makes up for items that have none: never a real one.
const MADE_UP: &str = "@mailcraft.invalid";

/// The replies and ids an import has read, for the sent items read last.
#[derive(Clone, Debug, Default)]
pub(in super::super) struct SentIds {
    /// A reply's parent index (its hash) and the id it answers; none when replies with that
    /// parent answer two.
    replies: HashMap<u64, Option<String>>,
    /// The real ids read before the sent items (their hashes).
    seen: HashSet<u64>,
    /// Those read and given since (the sent items' own): apart, so the lanes of an import gone
    /// on with give from what was seen when the sent items began, as the one that never
    /// stopped did, and the writer takes back what they give twice.
    later: HashSet<u64>,
    bytes: usize,
    /// Past [`MAX_BYTES`]: nothing is kept, and no id is given.
    full: bool,
    /// The sent items are being read: replies are kept no more.
    frozen: bool,
    /// This run's sent items given an id, and those of them that had it taken back.
    pub(in super::super) given: u64,
    pub(in super::super) taken_back: u64,
}

/// An id a lane gave a sent item, and the one made up for it that it has without.
pub(in super::super) struct Given {
    pub(in super::super) id: String,
    pub(in super::super) fallback: String,
}

impl SentIds {
    /// As a checkpoint kept them (hostile: an id too long or too many entries leaves it full).
    pub(in super::super) fn from_map(map: &SentMap) -> SentIds {
        let mut ids = SentIds { full: map.full, frozen: map.frozen, ..SentIds::default() };
        if ids.full || !map.sound() {
            ids.fill();
            return ids;
        }
        for r in &map.replies {
            let id = (!r.id.is_empty()).then(|| r.id.clone());
            ids.bytes = ids.bytes.saturating_add(r.id.len().saturating_add(REPLY_BYTES));
            ids.replies.insert(r.key, id);
        }
        for h in &map.seen {
            if ids.seen.insert(*h) {
                ids.bytes = ids.bytes.saturating_add(SEEN_BYTES);
            }
        }
        for h in &map.later {
            if !ids.seen.contains(h) && ids.later.insert(*h) {
                ids.bytes = ids.bytes.saturating_add(SEEN_BYTES);
            }
        }
        // Frozen, the sent items' ids were kept past it, as they are from here.
        if !ids.frozen && ids.bytes > MAX_BYTES {
            ids.fill();
        }
        ids
    }

    /// As a checkpoint keeps them, in order.
    pub(in super::super) fn to_map(&self) -> SentMap {
        let mut replies: Vec<SentReply> = self.replies.iter().map(|(k, id)| SentReply { key: *k, id: id.clone().unwrap_or_default() }).collect();
        replies.sort_by_key(|r| r.key);
        let mut seen: Vec<u64> = self.seen.iter().copied().collect();
        seen.sort_unstable();
        let mut later: Vec<u64> = self.later.iter().copied().collect();
        later.sort_unstable();
        SentMap { replies, seen, later, full: self.full, frozen: self.frozen }
    }

    /// Past what's kept: let go of it all, and give nothing from here.
    fn fill(&mut self) {
        self.full = true;
        self.replies = HashMap::new();
        self.seen = HashSet::new();
        self.later = HashSet::new();
        self.bytes = 0;
    }

    /// A message read: the reply it is (its parent index's hash and the id it answers), and its
    /// own id (none, or one made up, is no id).
    pub(in super::super) fn note(&mut self, reply: Option<(u64, String)>, own: &str) {
        if self.full {
            return;
        }
        if !own.is_empty() && !own.ends_with(MADE_UP) && self.see(msgid_hash(own)) {
            self.bytes = self.bytes.saturating_add(SEEN_BYTES);
        }
        // Once the sent items are read, what they answer can't be given any more (their folder
        // is the last), and their own ids are bounded by the folder's items.
        if let Some((key, id)) = reply.filter(|(k, id)| !self.frozen && *k != 0 && !id.is_empty() && !id.ends_with(MADE_UP)) {
            match self.replies.get_mut(&key) {
                None => {
                    self.bytes = self.bytes.saturating_add(id.len().saturating_add(REPLY_BYTES));
                    self.replies.insert(key, Some(id));
                }
                // Two replies to one message answering two ids: no telling which it had.
                Some(had) if had.as_deref().is_some_and(|h| h != id) => *had = None,
                Some(_) => {}
            }
        }
        if !self.frozen && self.bytes > MAX_BYTES {
            log::info!("import: past {MAX_BYTES} bytes of replies and ids; sent items keep the ids made up for them");
            self.fill();
        }
    }

    /// The id given to a sent item is its own from here: false when a message read before it
    /// (in the planner's order) has it already, and it's taken back.
    pub(in super::super) fn claim(&mut self, id: &str) -> bool {
        if self.full {
            return false;
        }
        let fresh = self.see(msgid_hash(id));
        if fresh {
            self.bytes = self.bytes.saturating_add(SEEN_BYTES);
        }
        fresh
    }

    /// An id read or given: kept with those seen before the sent items, or once they're read,
    /// apart (`later`); false when it was seen already.
    fn see(&mut self, h: u64) -> bool {
        if self.seen.contains(&h) {
            false
        } else if self.frozen {
            self.later.insert(h)
        } else {
            self.seen.insert(h)
        }
    }

    /// The sent items are read from here: the replies as they stand, for the lanes.
    pub(in super::super) fn freeze(&mut self) -> SentIds {
        self.frozen = true;
        self.clone()
    }

    /// The id the replies answer for the message whose own index has the hash `own`, unless
    /// it's ambiguous or a message read before the sent items has it (one read or given since
    /// is the writer's to take back, in the planner's order, wherever the import stopped).
    pub(in super::super) fn lookup(&self, own: u64) -> Option<&str> {
        if self.full || own == 0 {
            return None;
        }
        self.replies.get(&own)?.as_deref().filter(|id| !self.seen.contains(&msgid_hash(id)))
    }

    #[cfg(test)]
    pub(in super::super) fn is_full(&self) -> bool {
        self.full
    }
}

/// The reply an item is: its parent index's hash and the id it answers (In-Reply-To), when it
/// has both.
pub(in super::super) fn reply_of(it: &Item) -> Option<(u64, String)> {
    let parent = mailcraft_model::thread::index_keys(&it.thread_index).parent;
    (parent != 0 && !it.in_reply_to.is_empty()).then(|| (parent, it.in_reply_to.clone()))
}

/// Give a sent item that has no id of its own (none kept, and no original header, which the
/// message keeps in place of the one built with the id) the id the replies to it answer; the id
/// given.
pub(in super::super) fn give(it: &mut Item, ids: &SentIds) -> Option<String> {
    if !it.message_id.is_empty() || super::super::rebuild::usable_headers(&it.headers) {
        return None;
    }
    let own = mailcraft_model::thread::index_keys(&it.thread_index).own;
    let id = ids.lookup(own)?.to_string();
    it.message_id = id.clone();
    Some(id)
}

impl Out<'_> {
    /// A message about to be kept, in the planner's order: the reply it is and its id noted, and
    /// an id given to it that a message before it has taken back (it keeps the one made up).
    pub(in super::super) fn sent_ids(&mut self, b: &mut Built) {
        if let Some(given) = b.given.take() {
            self.sent.given = self.sent.given.saturating_add(1);
            if !self.sent.claim(&given.id) {
                self.sent.taken_back = self.sent.taken_back.saturating_add(1);
                self.take_back(b, &given.fallback);
            }
        }
        let reply = b.reply.take();
        self.sent.note(reply, &b.summary.message_id);
    }

    /// The sent items are read from here: what the lanes give their ids from.
    pub(in super::super) fn freeze_sent(&mut self) -> Arc<SentIds> {
        Arc::new(self.sent.freeze())
    }

    /// The message made again with `fallback` for its id (rare: two sent items alike).
    fn take_back(&mut self, b: &mut Built, fallback: &str) {
        match b.prepared.source() {
            Ok(raw) => {
                let Some(raw) = with_id(raw, fallback) else { return };
                b.raw_len = raw.len();
                b.summary.size = u32::try_from(raw.len()).unwrap_or(u32::MAX);
                b.summary.message_id = fallback.to_string();
                b.prepared = Store::prepare(self.store.packs(self.account), raw);
            }
            Err(e) => log::warn!("import: a sent item's id couldn't be taken back: {e}"),
        }
    }
}

/// `raw` with `id` for its Message-ID (the header's first, continuation lines and all); none
/// when it has no Message-ID.
fn with_id(mut raw: Vec<u8>, id: &str) -> Option<Vec<u8>> {
    let end = raw.windows(4).position(|w| w == b"\r\n\r\n")?;
    let head = raw.get(..end).unwrap_or_default();
    let mut found: Option<(usize, usize)> = None;
    let mut at = 0;
    while at < head.len() {
        let next = head.get(at..).and_then(|h| h.windows(2).position(|w| w == b"\r\n")).map_or(head.len(), |n| at.saturating_add(n));
        match found.as_mut() {
            Some(f) if head.get(at).is_some_and(|c| *c == b' ' || *c == b'\t') => f.1 = next,
            Some(_) => break,
            None if head.get(at..at.saturating_add(11)).is_some_and(|l| l.eq_ignore_ascii_case(b"message-id:")) => found = Some((at, next)),
            None => {}
        }
        at = next.saturating_add(2);
    }
    let (from, to) = found?;
    raw.splice(from..to, format!("Message-ID: <{id}>").into_bytes());
    Some(raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A conversation index: its header (GUID `g`) and `blocks` replies.
    fn index(g: u8, blocks: &[u8]) -> Vec<u8> {
        let mut i = vec![1, 0, 0, 0, 0, 0];
        i.extend([g; 16]);
        for b in blocks {
            i.extend([*b; 5]);
        }
        i
    }

    fn key(i: &[u8]) -> u64 {
        mailcraft_model::thread::index_keys(i).own
    }

    fn reply(parent: &[u8], id: &str) -> Option<(u64, String)> {
        Some((key(parent), id.to_string()))
    }

    /// A reply's parent gets the id it answers; one two replies answer differently gets none,
    /// and so does one a message read has already; past the first, an id given is taken back.
    #[test]
    fn the_map_gives_an_id_only_when_it_is_sure() {
        let (a, b, c) = (index(7, &[]), index(8, &[1]), index(9, &[]));
        let mut ids = SentIds::default();
        ids.note(reply(&a, "a@studio.example"), "r1@ana.example");
        ids.note(reply(&a, "a@studio.example"), "r2@ana.example");
        ids.note(reply(&b, "b@studio.example"), "r3@ana.example");
        ids.note(reply(&b, "b2@studio.example"), "r4@ana.example");
        ids.note(reply(&c, "r1@ana.example"), "");
        ids.note(None, "pst.a1.x.8022@mailcraft.invalid");
        let frozen = ids.freeze();
        assert_eq!(frozen.lookup(key(&a)), Some("a@studio.example"), "two replies, one id");
        assert_eq!(frozen.lookup(key(&b)), None, "two ids: no telling");
        assert_eq!(frozen.lookup(key(&c)), None, "a message read has it");
        assert_eq!(frozen.lookup(key(&index(3, &[]))), None, "nothing answers it");
        assert_eq!(frozen.lookup(0), None, "no index");
        assert!(ids.claim("a@studio.example"), "the first is given it");
        assert!(!ids.claim("a@studio.example"), "and only the first");
        // Frozen, a reply read now isn't kept, though its id is.
        ids.note(reply(&index(4, &[]), "d@studio.example"), "r5@ana.example");
        assert_eq!(ids.freeze().lookup(key(&index(4, &[]))), None);
        assert!(!ids.claim("r5@ana.example"));
    }

    /// Past what's kept, nothing is kept and nothing given; a map from a checkpoint reads back as
    /// it was written, and one that doesn't add up gives nothing.
    #[test]
    fn the_map_is_capped_and_kept() {
        let mut ids = SentIds::default();
        let a = index(7, &[]);
        ids.note(reply(&a, "a@studio.example"), "r1@ana.example");
        let back = SentIds::from_map(&ids.to_map());
        assert_eq!(back.to_map(), ids.to_map());
        assert_eq!(back.lookup(key(&a)), Some("a@studio.example"));
        let long = SentMap { replies: vec![SentReply { key: 1, id: "x".repeat(MAX_ID + 1) }], ..SentMap::default() };
        assert!(SentIds::from_map(&long).is_full());
        let id = "i".repeat(800);
        let mut n = 0u64;
        while !ids.is_full() {
            n += 1;
            ids.note(Some((n, format!("{n}{id}"))), &format!("{n}@ana.example"));
        }
        assert!(n > 15_000, "{n}");
        assert_eq!(ids.freeze().lookup(key(&a)), None, "full: nothing given");
        assert!(!ids.claim("z@studio.example"));
        assert_eq!(ids.to_map(), SentMap { full: true, frozen: true, ..SentMap::default() }, "and nothing kept");
    }

    /// Once the sent items are read, their ids are kept past the cap (the folder's rows bound
    /// them): a checkpoint taken then reads back as it was, and gives what it gave.
    #[test]
    fn ids_kept_past_the_cap_once_frozen_read_back() {
        let a = index(7, &[]);
        let mut ids = SentIds::default();
        ids.note(reply(&a, "a@studio.example"), "");
        let id = "i".repeat(800);
        for n in 1..19_500u64 {
            ids.note(Some((n.saturating_add(100), format!("{n}{id}"))), "");
        }
        assert!(!ids.is_full() && ids.bytes > MAX_BYTES - 200_000, "{}", ids.bytes);
        ids.freeze();
        for n in 0..20_000 {
            ids.note(None, &format!("s{n}@studio.example"));
        }
        assert!(ids.bytes > MAX_BYTES && !ids.is_full());
        let map = ids.to_map();
        assert!(map.sound());
        let back = SentIds::from_map(&map);
        assert!(!back.is_full());
        assert_eq!(back.lookup(key(&a)), Some("a@studio.example"));
        assert_eq!(back.to_map(), map);
        // Past what the folder's rows could add, or so many before it, it doesn't add up.
        let most = MAX_BYTES / 16;
        let many = SentMap { seen: (0..=most as u64).collect(), ..SentMap::default() };
        assert!(!many.sound());
        assert!(SentMap { frozen: true, ..many.clone() }.sound());
        assert!(SentIds::from_map(&many).is_full());
        assert!(!SentMap { later: vec![1], ..SentMap::default() }.sound(), "kept apart before the sent items");
    }

    /// Gone on with from a checkpoint taken while the sent items are read, the lanes give what
    /// they gave when the sent items began: a second sent item answered with the id the first
    /// was given (or the id a sent item has of its own) is given it too, and the writer takes it
    /// back, so its source is the one the import that never stopped kept.
    #[test]
    fn going_on_gives_what_the_sent_items_were_given_from_the_start() {
        let (a, b) = (index(7, &[]), index(8, &[]));
        let mut ids = SentIds::default();
        ids.note(reply(&a, "a@studio.example"), "r1@ana.example");
        ids.note(reply(&b, "b@studio.example"), "r2@ana.example");
        let lanes = ids.freeze();
        assert!(ids.claim("a@studio.example"), "the first sent item");
        ids.note(None, "b@studio.example");
        let back = SentIds::from_map(&ids.to_map()).freeze();
        for k in [key(&a), key(&b)] {
            assert_eq!(back.lookup(k), lanes.lookup(k), "the same ids given");
        }
        let mut back = SentIds::from_map(&ids.to_map());
        assert!(!back.claim("a@studio.example") && !back.claim("b@studio.example"), "and taken back");
        assert!(back.claim("c@studio.example"));
        assert_eq!(SentIds::from_map(&back.to_map()).to_map(), back.to_map());
    }

    fn item(index_of: Vec<u8>, own: &str, headers: &str) -> Item {
        Item { class: "IPM.Note".into(), thread_index: index_of, message_id: own.into(), headers: headers.into(), ..Item::default() }
    }

    /// Only a sent item with no id of its own is given one, by its own index.
    #[test]
    fn only_an_item_without_an_id_is_given_one() {
        let a = index(7, &[]);
        let mut ids = SentIds::default();
        let mut r = item(index(7, &[2]), "r@ana.example", "");
        r.in_reply_to = "a@studio.example".into();
        let got = reply_of(&r);
        assert_eq!(got, reply(&a, "a@studio.example"));
        ids.note(got, &r.message_id);
        assert_eq!(reply_of(&item(index(7, &[2]), "r@ana.example", "")), None, "no In-Reply-To");
        let ids = ids.freeze();
        let mut sent = item(a.clone(), "", "");
        assert_eq!(give(&mut sent, &ids).as_deref(), Some("a@studio.example"));
        assert_eq!(sent.message_id, "a@studio.example");
        assert_eq!(give(&mut item(a.clone(), "own@studio.example", ""), &ids), None, "an id of its own");
        assert_eq!(give(&mut item(a.clone(), "", "From: a@studio.example\r\nMessage-Id: <h@studio.example>\r\n"), &ids), None, "one in its header");
        // The message keeps that header, not the one built with the id given.
        assert_eq!(give(&mut item(a.clone(), "", "From: a@studio.example\r\nSubject: x\r\n"), &ids), None, "a header without one");
        let mut bad = a.clone();
        bad.push(0);
        assert_eq!(give(&mut item(bad, "", ""), &ids), None, "an index of the wrong length");
    }

    /// An id taken back goes out of the source's header (folded or not) and the summary.
    #[test]
    fn an_id_taken_back_leaves_the_message() {
        let raw = b"From: a@studio.example\r\nMessage-ID:\r\n <a@studio.example>\r\nSubject: s\r\n\r\nMessage-ID: body\r\n".to_vec();
        let out = with_id(raw, "pst.a1.x.1@mailcraft.invalid").unwrap();
        assert_eq!(out, b"From: a@studio.example\r\nMessage-ID: <pst.a1.x.1@mailcraft.invalid>\r\nSubject: s\r\n\r\nMessage-ID: body\r\n");
        let none = b"From: a@studio.example\r\n\r\nMessage-ID: body\r\n".to_vec();
        assert_eq!(with_id(none, "x@mailcraft.invalid"), None);

        let dir = std::env::temp_dir().join(format!("mailcraft-pst-sent-ids-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = Store::open_packed(dir.join("app"), dir.join("Files")).unwrap();
        let mut progress = |_| {};
        let mut out = Out::new(&store, "a1", &ImportOptions::default(), &mut progress);
        let path = out.add_folder(None, "Sent Items");
        let tally = Tally::default();
        for n in 0..2 {
            let it = Item { class: "IPM.Note".into(), subject: format!("Plan {n}"), message_id: "a@studio.example".into(), ..Item::default() };
            let fallback = format!("pst.a1.x.{n}@mailcraft.invalid");
            let super::super::lanes::Outcome::Built(mut b) = super::super::lanes::build(it, &fallback, true, false, &tally) else { panic!("built") };
            b.given = Some(Given { id: "a@studio.example".into(), fallback });
            out.sent_ids(&mut b);
            out.add_built(&path, *b, 0);
        }
        out.save_folder(&path);
        let mut st = store.load_folder("a1", &path).unwrap();
        st.messages.sort_by_key(|m| m.key.uid);
        let ids: Vec<&str> = st.messages.iter().map(|m| m.message_id.as_str()).collect();
        assert_eq!(ids, ["a@studio.example", "pst.a1.x.1@mailcraft.invalid"]);
        let second = store.read_body(&st.messages[1].key).unwrap();
        assert_eq!(
            mailcraft_mime::summary_from_full(&second, Default::default(), Default::default(), 0, 0).message_id,
            "pst.a1.x.1@mailcraft.invalid"
        );
        assert_eq!(st.messages[1].size as usize, second.len());
        drop(out);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
