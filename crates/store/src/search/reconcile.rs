//! Bringing an account's index up to its mail ([`plan`]). The index isn't told of each change
//! where it's made (a sync, a move, a delete, an import): a pass compares each folder's list and
//! kept sources with the docs the index has for it. A message the index has under another
//! Message-ID or size is another message (removed, and added again); one that left a folder and
//! turned up in another, alone with its Message-ID and size, moved (its doc goes with it, nothing
//! read). A folder whose list and sources are as they were when it was last brought up
//! ([`revision`]) is passed over without reading its list.

use std::collections::HashMap;
use std::hash::Hasher;

use super::*;

/// A message to add, and what tells it from another.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    pub key: MessageKey,
    pub msgid_hash: u64,
    pub size: u32,
}

/// What brings an index up to its mail.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Plan {
    /// Messages with a source kept and no doc: to read and [add](Writer::add), newest first.
    pub add: Vec<Message>,
    /// Messages with no source kept here and no doc: [added as such](Writer::add_no_source),
    /// nothing read. A search settles them from their summaries; they aren't waiting to be
    /// indexed.
    pub no_source: Vec<Message>,
    /// Docs of messages no longer there.
    pub remove: Vec<u32>,
    /// Docs of messages that moved, and where to.
    pub retarget: Vec<(u32, MessageKey)>,
    /// Each folder looked at and its [`revision`] (0: gone): [`Writer::mark`] it once its adds
    /// are all in.
    pub marks: Vec<(String, u64)>,
    /// The messages in the folders looked at.
    pub total: usize,
    /// The writer's [epoch](Writer::epoch) when it was made: its doc ids mean nothing in another.
    pub epoch: u64,
}

impl Plan {
    /// Nothing to change (marks aside).
    pub fn is_empty(&self) -> bool {
        self.add.is_empty() && self.no_source.is_empty() && self.remove.is_empty() && self.retarget.is_empty()
    }

    /// Make the changes that read nothing: removals, moves, and the messages with no source.
    /// The adds are the caller's (each needs its source read). An error, nothing changed, when
    /// the ids were given again since the plan was made (a merge step between): plan again.
    pub fn apply(&self, w: &mut Writer) -> Result<()> {
        w.same_epoch(self.epoch)?;
        for &doc in &self.remove {
            w.remove(doc);
        }
        w.retarget_all(&self.retarget)?;
        for m in &self.no_source {
            w.add_no_source(&m.key, m.msgid_hash, m.size)?;
        }
        Ok(())
    }
}

/// What the index keeps of a Message-ID, to know a message moved.
pub fn msgid_hash(message_id: &str) -> u64 {
    fnv64(message_id)
}

/// A folder's list and kept sources, as one number that changes when either does (0 never):
/// read from its list's head and its sources' names, not the list itself. `None` when the head
/// can't be read.
pub fn revision(store: &Store, account: &str, folder: &str) -> Option<u64> {
    #[derive(serde::Deserialize, Default)]
    struct Head {
        #[serde(default)]
        validity: u32,
    }
    let path = store.folder_dir(account, folder).join("state.json");
    let head = match store.files.read(&path) {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(_) => return None,
    };
    let validity = if head.is_empty() { 0 } else { serde_json::from_slice::<Head>(&head).ok()?.validity };
    let mut uids = store.cached_bodies(account, folder, validity);
    uids.sort_unstable();
    let mut h = Fnv(0xcbf2_9ce4_8422_2325);
    h.write(&head);
    h.write_u32(validity);
    for uid in uids {
        h.write_u32(uid);
    }
    Some(h.finish().max(1))
}

/// FNV-1a over bytes.
struct Fnv(u64);

impl Hasher for Fnv {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.0 ^= u64::from(*b);
            self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
        }
    }
}

/// What brings `account`'s index (held by `w`) up to its mail, in `folders` (none: every folder
/// it has, and every folder the index names). Nothing is changed or read but lists and the names
/// of the sources kept.
pub fn plan(store: &Store, w: &Writer, account: &str, folders: &[String]) -> Result<Plan> {
    let known: BTreeSet<String> = store.load_folders(account)?.into_iter().map(|f| f.path).collect();
    let look: BTreeSet<String> =
        if folders.is_empty() { known.iter().cloned().chain(w.folders().iter().cloned()).collect() } else { folders.iter().cloned().collect() };
    let mut plan = Plan { epoch: w.epoch(), ..Plan::default() };
    // Docs going, and messages to add, with their dates (newest added first).
    let mut gone: Vec<Indexed> = Vec::new();
    let mut adds: Vec<(i64, Message, bool)> = Vec::new();
    let mut indexed = w.docs_by_folder();
    for folder in look {
        if !known.contains(&folder) {
            gone.extend(indexed.remove(&folder).unwrap_or_default().into_values());
            if w.marked(&folder).is_some() {
                plan.marks.push((folder, 0));
            }
            continue;
        }
        let rev = revision(store, account, &folder);
        if rev.is_some() && w.marked(&folder) == rev {
            continue;
        }
        let st = store.load_folder(account, &folder)?;
        let sources: BTreeSet<u32> = store.cached_bodies(account, &folder, st.validity).into_iter().collect();
        let mut docs = indexed.remove(&folder).unwrap_or_default();
        for s in &st.messages {
            plan.total = plan.total.saturating_add(1);
            let has = sources.contains(&s.key.uid);
            let hash = msgid_hash(&s.message_id);
            match docs.remove(&s.key.uid) {
                // The same message: a source let go since leaves its words in.
                Some(i) if i.validity == st.validity && i.msgid_hash == hash && i.size == s.size && (i.source || !has) => continue,
                // Another message, or its source kept since.
                Some(i) => gone.push(i),
                None => {}
            }
            adds.push((s.date, Message { key: s.key.clone(), msgid_hash: hash, size: s.size }, has));
        }
        gone.extend(docs.into_values());
        if let Some(rev) = rev {
            plan.marks.push((folder, rev));
        }
    }
    // Moves: a doc going and a message coming with the same Message-ID and size, each the only
    // one so. A doc with no source kept goes (nothing to keep of it).
    let mut going: HashMap<(u64, u32), Vec<usize>> = HashMap::new();
    for (n, i) in gone.iter().enumerate() {
        if i.source {
            going.entry((i.msgid_hash, i.size)).or_default().push(n);
        }
    }
    let mut coming: HashMap<(u64, u32), Vec<usize>> = HashMap::new();
    for (n, (_, m, _)) in adds.iter().enumerate() {
        coming.entry((m.msgid_hash, m.size)).or_default().push(n);
    }
    let mut moved_docs = BTreeSet::new();
    let mut moved_msgs = BTreeSet::new();
    for (k, docs) in &going {
        if let ([d], Some([m])) = (docs.as_slice(), coming.get(k).map(Vec::as_slice))
            && let (Some(i), Some((_, msg, _))) = (gone.get(*d), adds.get(*m))
        {
            plan.retarget.push((i.doc, msg.key.clone()));
            moved_docs.insert(*d);
            moved_msgs.insert(*m);
        }
    }
    plan.retarget.sort_by_key(|(d, _)| *d);
    plan.remove = gone.iter().enumerate().filter(|(n, _)| !moved_docs.contains(n)).map(|(_, i)| i.doc).collect();
    plan.remove.sort_unstable();
    let mut adds: Vec<(i64, Message, bool)> = adds.into_iter().enumerate().filter(|(n, _)| !moved_msgs.contains(n)).map(|(_, a)| a).collect();
    adds.sort_by_key(|a| std::cmp::Reverse(a.0));
    for (_, m, has) in adds {
        if has { plan.add.push(m) } else { plan.no_source.push(m) }
    }
    Ok(plan)
}
