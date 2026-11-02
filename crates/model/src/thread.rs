//! Conversation threading: Jamie Zawinski's algorithm (the one behind IMAP `THREAD=REFERENCES`,
//! RFC 5256), followed by Outlook-style grouping of replies that lost their references but share
//! a subject.
//!
//! 1. Every Message-ID seen (in `Message-ID`, `References` or `In-Reply-To`) gets a container.
//! 2. Each message's `References` chain (with `In-Reply-To` appended when missing) links those
//!    containers parent → child, never creating a loop; the message's own container takes the
//!    last reference as its parent.
//! 3. A message the ids left without a parent, or hanging only from ids never seen, goes under the message its conversation index
//!    (Outlook's `Thread-Index`) answers, or else, when the subjects match, joins the
//!    conversation that index names.
//! 4. Empty containers (referenced, never seen) are dropped, promoting their children.
//! 5. Roots whose base subjects match are merged when one of them is a reply or forward.

pub mod cleanup;

use std::collections::HashMap;

use crate::Summary;

/// Strip `Re:`, `Fwd:`, `AW:`, `[list]` tags and the like from a subject.
pub fn base_subject(subject: &str) -> String {
    let mut s = subject.trim();
    loop {
        let before = s;
        if s.starts_with('[')
            && let Some(end) = s.find(']')
        {
            s = s.get(end + 1..).unwrap_or_default().trim_start();
        }
        // ASCII lowering keeps byte offsets equal to `s`'s.
        let lower = s.to_ascii_lowercase();
        for p in ["re", "fw", "fwd", "aw", "sv", "vs", "wg", "rif", "tr", "antw", "odp", "ynt"] {
            if let Some(rest) = lower.strip_prefix(p) {
                // `Re:`, `RE[2]:`, `Re :`.
                let rest_t = rest.trim_start_matches(|c: char| c.is_ascii_digit() || c == '[' || c == ']').trim_start();
                if rest_t.starts_with(':') || rest_t.starts_with('：') {
                    let consumed = lower.len() - rest_t.len() + rest_t.chars().next().map_or(0, char::len_utf8);
                    s = s.get(consumed..).unwrap_or_default().trim_start();
                    break;
                }
            }
        }
        if s == before {
            break;
        }
    }
    s.trim().to_string()
}

/// Whether a subject carries a reply or forward prefix.
pub fn is_reply_subject(subject: &str) -> bool {
    base_subject(subject).len() != subject.trim().len()
}

/// Normalise a Message-ID for comparison: no angle brackets or surrounding space.
/// A Message-ID without its brackets. Whitespace and control characters, which an id can't
/// hold (RFC 5322 §3.6.4), are dropped: a hostile one's bare CR would otherwise be copied into
/// a reply's In-Reply-To and References.
pub fn norm_id(id: &str) -> String {
    let id: String = id.chars().filter(|c| !c.is_control() && !c.is_whitespace()).take(998).collect();
    id.trim_start_matches('<').trim_end_matches('>').to_string()
}

/// The most 5-byte blocks a conversation index is read with (a reply adds one).
pub const MAX_INDEX_BLOCKS: usize = 50;

/// A conversation index as threading keeps it: hashes of the whole index, of the index of the
/// message it answers (the whole less its last block) and of the conversation's (its 22-byte
/// header). 0 is none.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct IndexKeys {
    pub own: u64,
    pub parent: u64,
    pub conversation: u64,
}

/// The keys of a conversation index (MS-OXOMSG 2.2.1.3: a reserved byte, a 5-byte time and a
/// 16-byte GUID, then a 5-byte block per reply). An index of any other length, of more than
/// [`MAX_INDEX_BLOCKS`] blocks or with no GUID gives none: such an index would join mail that
/// has nothing to do with each other.
pub fn index_keys(index: &[u8]) -> IndexKeys {
    let blocks = index.len().checked_sub(22).filter(|n| n % 5 == 0).map(|n| n / 5);
    let guid = index.get(6..22).is_some_and(|g| g.iter().any(|&b| b != 0));
    let (Some(blocks), true) = (blocks, guid) else { return IndexKeys::default() };
    if blocks > MAX_INDEX_BLOCKS {
        return IndexKeys::default();
    }
    let hash = |len: usize| index.get(..len).map_or(0, |b| fnv(b).max(1));
    IndexKeys { own: hash(index.len()), parent: if blocks > 0 { hash(index.len().saturating_sub(5)) } else { 0 }, conversation: hash(22) }
}

/// FNV-1a: kept in the lists, so it must never change.
fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
}

#[derive(Default)]
struct Container {
    msg: Option<usize>,
    parent: Option<usize>,
    children: Vec<usize>,
}

struct Builder {
    nodes: Vec<Container>,
    by_id: HashMap<String, usize>,
}

impl Builder {
    fn node(&mut self, id: &str) -> usize {
        if let Some(&n) = self.by_id.get(id) {
            return n;
        }
        self.nodes.push(Container::default());
        let n = self.nodes.len() - 1;
        self.by_id.insert(id.to_string(), n);
        n
    }

    fn fresh(&mut self) -> usize {
        self.nodes.push(Container::default());
        self.nodes.len() - 1
    }

    /// Whether `a` is `b` or one of its ancestors.
    fn reaches(&self, a: usize, b: usize) -> bool {
        let mut cur = Some(b);
        let mut steps = 0usize;
        while let Some(c) = cur {
            if c == a {
                return true;
            }
            steps += 1;
            if steps > self.nodes.len() {
                return true;
            }
            cur = self.nodes.get(c).and_then(|n| n.parent);
        }
        false
    }

    fn unlink(&mut self, child: usize) {
        if let Some(p) = self.nodes.get(child).and_then(|n| n.parent) {
            if let Some(pn) = self.nodes.get_mut(p) {
                pn.children.retain(|&c| c != child);
            }
            if let Some(cn) = self.nodes.get_mut(child) {
                cn.parent = None;
            }
        }
    }

    fn link(&mut self, parent: usize, child: usize) {
        if parent == child || self.reaches(child, parent) {
            return;
        }
        self.unlink(child);
        if let Some(cn) = self.nodes.get_mut(child) {
            cn.parent = Some(parent);
        }
        if let Some(pn) = self.nodes.get_mut(parent) {
            pn.children.push(child);
        }
    }

    fn parent_of(&self, n: usize) -> Option<usize> {
        self.nodes.get(n).and_then(|c| c.parent)
    }

    /// What the ids left `n` hanging from when they reach no message: `n` itself when they gave
    /// it no parent, or the top of its chain when every container above it was only referenced
    /// (a reply to an original listed under another id). `None` once they reach a message.
    fn unplaced_top(&self, n: usize) -> Option<usize> {
        let mut cur = n;
        for _ in 0..=self.nodes.len() {
            match self.parent_of(cur) {
                None => return Some(cur),
                Some(p) if self.nodes.get(p).is_some_and(|c| c.msg.is_some()) => return None,
                Some(p) => cur = p,
            }
        }
        None
    }

    /// All messages under `root`, depth-first.
    fn collect(&self, root: usize, out: &mut Vec<usize>) {
        let mut stack = vec![root];
        let mut seen = vec![false; self.nodes.len()];
        while let Some(n) = stack.pop() {
            match seen.get_mut(n) {
                Some(s) if !*s => *s = true,
                _ => continue,
            }
            if let Some(c) = self.nodes.get(n) {
                if let Some(m) = c.msg {
                    out.push(m);
                }
                stack.extend(c.children.iter().rev().copied());
            }
        }
    }
}

/// One conversation: indices into the input, oldest first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Thread {
    pub messages: Vec<usize>,
    /// When its newest message came ([`Summary::when`]).
    pub latest: i64,
}

/// Place by their conversation index the messages (`own[i]` holds message `i`) the ids left
/// without a parent, or under ids no message listed carries: under the message whose index is theirs less its last block, or else
/// under the conversation's first message (the one whose index is the conversation's alone, or
/// the oldest) when its base subject is theirs. Ids stay first: they are what a reply names.
fn place_by_index(b: &mut Builder, msgs: &[&Summary], own: &[usize]) {
    let mut by_own: HashMap<u64, usize> = HashMap::new();
    let mut by_conversation: HashMap<u64, usize> = HashMap::new();
    for (i, m) in msgs.iter().enumerate() {
        if m.thread_own != 0
            && let Some(&n) = own.get(i)
        {
            by_own.entry(m.thread_own).or_insert(n);
        }
        if m.thread_conversation != 0 {
            let first = |&j: &usize| msgs.get(j).map_or((true, i64::MAX, j), |m| (m.thread_own != m.thread_conversation, m.when(), j));
            by_conversation.entry(m.thread_conversation).and_modify(|j| *j = std::cmp::min_by_key(*j, i, first)).or_insert(i);
        }
    }
    if by_own.is_empty() {
        return;
    }
    for (i, m) in msgs.iter().enumerate() {
        let Some(&n) = own.get(i) else { continue };
        let Some(top) = b.unplaced_top(n).filter(|_| m.thread_own != 0) else { continue };
        if let Some(&p) = by_own.get(&m.thread_parent).filter(|_| m.thread_parent != 0) {
            b.link(p, top);
            if b.parent_of(top).is_some() {
                continue;
            }
        }
        if let Some(&j) = by_conversation.get(&m.thread_conversation)
            && j != i
            && let (Some(first), Some(&fnode)) = (msgs.get(j), own.get(j))
            && base_subject(&first.subject).to_lowercase() == base_subject(&m.subject).to_lowercase()
        {
            b.link(fnode, top);
        }
    }
}

/// Group messages into conversations. The result is sorted with the most recently active
/// conversation first; every input index appears exactly once.
pub fn threads<S: std::borrow::Borrow<Summary>>(msgs: &[S]) -> Vec<Thread> {
    let msgs: Vec<&Summary> = msgs.iter().map(std::borrow::Borrow::borrow).collect();
    threads_of(&msgs)
}

fn threads_of(msgs: &[&Summary]) -> Vec<Thread> {
    let mut b = Builder { nodes: Vec::with_capacity(msgs.len() * 2), by_id: HashMap::with_capacity(msgs.len() * 2) };
    let mut own = Vec::with_capacity(msgs.len());
    for (i, m) in msgs.iter().enumerate() {
        let id = norm_id(&m.message_id);
        let mut n = if id.is_empty() { b.fresh() } else { b.node(&id) };
        if b.nodes.get(n).and_then(|c| c.msg).is_some() {
            // A duplicate Message-ID (the same mail in two folders): its own container.
            n = b.fresh();
        }
        if let Some(c) = b.nodes.get_mut(n) {
            c.msg = Some(i);
        }
        own.push(n);

        let mut refs: Vec<String> = m.references.iter().map(|r| norm_id(r)).filter(|r| !r.is_empty() && *r != id).collect();
        let irt = norm_id(&m.in_reply_to);
        if !irt.is_empty() && irt != id && refs.last() != Some(&irt) {
            refs.push(irt);
        }
        // Cap absurd chains (hostile headers).
        if refs.len() > 256 {
            refs.drain(..refs.len() - 256);
        }
        let mut prev: Option<usize> = None;
        for r in &refs {
            let rn = b.node(r);
            if let Some(p) = prev
                && b.parent_of(rn).is_none()
            {
                b.link(p, rn);
            }
            prev = Some(rn);
        }
        match prev {
            Some(p) => b.link(p, n),
            None => b.unlink(n),
        }
    }
    place_by_index(&mut b, msgs, &own);

    // Roots, with empty containers resolved to their children.
    let mut roots: Vec<usize> = Vec::new();
    for (i, c) in b.nodes.iter().enumerate() {
        if c.parent.is_none() && (c.msg.is_some() || !c.children.is_empty()) {
            roots.push(i);
        }
    }

    let mut groups: Vec<Vec<usize>> = Vec::new();
    for r in roots {
        let mut v = Vec::new();
        b.collect(r, &mut v);
        if !v.is_empty() {
            groups.push(v);
        }
    }

    // Subject grouping: merge a reply-subject root into a root with the same base subject.
    let mut by_subject: HashMap<String, usize> = HashMap::new();
    let mut merged: Vec<Vec<usize>> = Vec::new();
    let first_subject = |g: &Vec<usize>| -> (String, bool) {
        let oldest = g.iter().copied().min_by_key(|&i| msgs.get(i).map_or(0, |m| m.when())).unwrap_or(0);
        let s = msgs.get(oldest).map(|m| m.subject.as_str()).unwrap_or_default();
        (base_subject(s).to_lowercase(), is_reply_subject(s))
    };
    // Original posts first, so replies join them rather than the other way round.
    groups.sort_by_key(|g| first_subject(g).1);
    for g in groups {
        let (subj, reply) = first_subject(&g);
        if !subj.is_empty()
            && reply
            && let Some(&at) = by_subject.get(&subj)
            && let Some(t) = merged.get_mut(at)
        {
            t.extend(g);
            continue;
        }
        if !subj.is_empty() {
            by_subject.entry(subj).or_insert(merged.len());
        }
        merged.push(g);
    }

    let mut out: Vec<Thread> = merged
        .into_iter()
        .map(|mut v| {
            v.sort_by_key(|&i| (msgs.get(i).map_or(0, |m| m.when()), i));
            v.dedup();
            let latest = v.iter().filter_map(|&i| msgs.get(i)).map(|m| m.when()).max().unwrap_or(0);
            Thread { messages: v, latest }
        })
        .collect();
    out.sort_by(|a, b| b.latest.cmp(&a.latest).then_with(|| a.messages.first().cmp(&b.messages.first())));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(id: &str, refs: &[&str], irt: &str, subject: &str, date: i64) -> Summary {
        Summary {
            message_id: id.into(),
            references: refs.iter().map(|s| s.to_string()).collect(),
            in_reply_to: irt.into(),
            subject: subject.into(),
            date,
            ..Default::default()
        }
    }

    #[test]
    fn subjects() {
        assert_eq!(base_subject("Re: RE: Fwd: Hello"), "Hello");
        assert_eq!(base_subject("AW: [team] Re[2]: Plan"), "Plan");
        assert_eq!(base_subject("Regarding the plan"), "Regarding the plan");
        assert_eq!(base_subject("Re : spaced"), "spaced");
        assert!(is_reply_subject("re: x"));
        assert!(!is_reply_subject("Results"));
        assert_eq!(base_subject(""), "");
        assert_eq!(base_subject("[unterminated"), "[unterminated");
    }

    #[test]
    fn threads_by_references() {
        let msgs = vec![
            m("<a@x>", &[], "", "Plan", 1),
            m("<b@x>", &["<a@x>"], "<a@x>", "Re: Plan", 2),
            m("<c@x>", &["<a@x>", "<b@x>"], "", "Re: Plan", 5),
            m("<d@x>", &[], "", "Other", 3),
        ];
        let t = threads(&msgs);
        assert_eq!(t.len(), 2);
        assert_eq!(t[0].messages, vec![0, 1, 2]);
        assert_eq!(t[0].latest, 5);
        assert_eq!(t[1].messages, vec![3]);
    }

    #[test]
    fn missing_parent_still_groups_siblings() {
        // Both reply to a message we don't have.
        let msgs = vec![m("<b@x>", &["<gone@x>"], "", "Re: Q", 2), m("<c@x>", &["<gone@x>"], "", "Re: Q", 3)];
        let t = threads(&msgs);
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].messages, vec![0, 1]);
    }

    #[test]
    fn subject_fallback_joins_replies_without_references() {
        let msgs = vec![m("<a@x>", &[], "", "Lunch?", 1), m("<b@y>", &[], "", "RE: Lunch?", 2), m("<c@z>", &[], "", "Lunch?", 3)];
        let t = threads(&msgs);
        // The reply joins the first original; the second original stays its own conversation.
        assert_eq!(t.len(), 2);
        let all: Vec<usize> = t.iter().flat_map(|t| t.messages.clone()).collect();
        assert_eq!(all.len(), 3);
        assert!(t.iter().any(|t| t.messages.contains(&0) && t.messages.contains(&1)));
    }

    #[test]
    fn loops_and_duplicates_are_safe() {
        let msgs = vec![
            m("<a@x>", &["<b@x>"], "", "x", 1),
            m("<b@x>", &["<a@x>"], "", "x", 2),
            m("<a@x>", &[], "", "dup", 3),
            m("", &[], "", "", 4),
            m("<self@x>", &["<self@x>"], "<self@x>", "s", 5),
        ];
        let t = threads(&msgs);
        let mut all: Vec<usize> = t.iter().flat_map(|t| t.messages.clone()).collect();
        all.sort();
        assert_eq!(all, vec![0, 1, 2, 3, 4]);
    }

    /// A conversation index: the header with `guid` (and a time), and `blocks` reply blocks.
    fn index(guid: u8, blocks: &[u8]) -> Vec<u8> {
        let mut v = vec![1, 0xD9, 1, 2, 3, 4];
        v.extend([guid; 16]);
        for &b in blocks {
            v.extend([b; 5]);
        }
        v
    }

    fn indexed(subject: &str, date: i64, idx: &[u8]) -> Summary {
        let mut s = m("", &[], "", subject, date);
        s.set_thread_index(idx);
        s
    }

    #[test]
    fn index_keys_take_whole_indices_with_a_guid() {
        let root = index_keys(&index(7, &[]));
        let reply = index_keys(&index(7, &[1]));
        assert!(root.own != 0 && root.parent == 0 && root.conversation == root.own);
        assert_eq!((reply.parent, reply.conversation), (root.own, root.own));
        assert_ne!(reply.own, root.own);
        // Not 22 + 5k bytes, no GUID, too many blocks, nothing: no keys.
        let mut odd = index(7, &[1]);
        odd.pop();
        for bad in [odd, index(0, &[1]), index(7, &[2; MAX_INDEX_BLOCKS + 1]), Vec::new(), vec![9; 21]] {
            assert_eq!(index_keys(&bad), IndexKeys::default(), "{}", bad.len());
        }
        assert_ne!(index_keys(&index(7, &[2; MAX_INDEX_BLOCKS])), IndexKeys::default());
    }

    #[test]
    fn the_index_places_a_reply_its_ids_dont() {
        // A sent original (no id kept) and a reply whose subject was changed.
        let msgs = vec![indexed("Plan", 1, &index(3, &[])), indexed("Other words", 5, &index(3, &[1])), indexed("Plan", 2, &index(4, &[]))];
        let t = threads(&msgs);
        assert_eq!(t.len(), 2);
        assert_eq!((t[0].messages.as_slice(), t[0].latest), (&[0, 1][..], 5));
    }

    #[test]
    fn the_index_places_a_reply_to_an_id_not_listed() {
        // The sent original is listed under a fallback id; the replies name its real one.
        let mut a = indexed("Plan", 1, &index(3, &[]));
        a.message_id = "<pst.a.1.1@mailcraft.invalid>".into();
        let mut b = indexed("Other words", 5, &index(3, &[1]));
        (b.message_id, b.in_reply_to) = ("<b@x>".into(), "<real-a@x>".into());
        let mut c = indexed("Re: Other words", 6, &index(3, &[1, 2]));
        (c.message_id, c.references) = ("<c@x>".into(), vec!["<real-a@x>".into(), "<b@x>".into()]);
        let t = threads(&[a, b, c, indexed("Else", 2, &index(4, &[]))]);
        assert_eq!(t.len(), 2);
        assert_eq!((t[0].messages.as_slice(), t[0].latest), (&[0, 1, 2][..], 6));
    }

    #[test]
    fn a_conversation_joins_only_on_its_subject() {
        // The message answered is missing: the conversation's first takes the replies whose
        // subject is its own.
        let msgs = vec![
            indexed("Plan", 1, &index(3, &[])),
            indexed("RE: Plan", 3, &index(3, &[1, 2])),
            indexed("Something else", 4, &index(3, &[1, 3])),
            indexed("Plan", 2, &index(0, &[])),
        ];
        let t = threads(&msgs);
        let with = |i: usize| t.iter().position(|t| t.messages.contains(&i));
        assert_eq!(with(0), with(1));
        assert_ne!(with(0), with(2));
        assert_ne!(with(0), with(3), "no GUID, no conversation");
    }

    #[test]
    fn ids_come_before_the_index() {
        // The reply names its parent by id; its index points elsewhere.
        let mut a = indexed("A", 1, &index(5, &[]));
        a.message_id = "<a@x>".into();
        let b = indexed("B", 2, &index(6, &[]));
        let mut c = indexed("Re: A", 3, &index(6, &[1]));
        c.in_reply_to = "<a@x>".into();
        let t = threads(&[a, b, c]);
        let with = |i: usize| t.iter().position(|t| t.messages.contains(&i));
        assert_eq!(with(0), with(2));
        assert_ne!(with(1), with(2));
    }

    #[test]
    fn received_orders_conversations() {
        let mut late = m("<b@x>", &["<a@x>"], "", "Re: Q", 2);
        late.received = 9;
        let msgs = vec![m("<a@x>", &[], "", "Q", 1), late, m("<c@x>", &[], "", "Other", 5)];
        let t = threads(&msgs);
        assert_eq!((t[0].messages.as_slice(), t[0].latest), (&[0, 1][..], 9));
    }

    #[test]
    fn long_chains_are_capped() {
        let refs: Vec<String> = (0..10_000).map(|i| format!("<r{i}@x>")).collect();
        let refs_ref: Vec<&str> = refs.iter().map(String::as_str).collect();
        let msgs = vec![m("<z@x>", &refs_ref, "", "deep", 1)];
        assert_eq!(threads(&msgs).len(), 1);
    }
}
