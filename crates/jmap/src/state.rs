//! What JMAP sync keeps between runs (states, ids), and messages' summaries from their properties.

use super::*;

/// What an account keeps between syncs: the numbers given to message ids, and the mailbox id
/// behind each folder path.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct State {
    #[serde(default)]
    pub(super) next: u32,
    #[serde(default)]
    pub(super) ids: HashMap<String, u32>,
    #[serde(default)]
    pub mailboxes: BTreeMap<String, String>,
    /// Messages a search of the server found in each folder (issue #4), kept there past the
    /// newest window while the server has them there.
    #[serde(default)]
    pub found: BTreeMap<String, Vec<u32>>,
    #[serde(skip)]
    pub(super) numbers: HashMap<u32, String>,
}

/// Messages a folder's search of the server brings in, at most.
pub const FOUND_MAX: usize = 500;

pub(super) const STATE_DOC: &str = "jmap.json";

impl State {
    pub fn load(store: &Store, account: &str) -> State {
        let mut s: State = store.load_account_doc(account, STATE_DOC).unwrap_or_default();
        s.numbers = s.ids.iter().map(|(k, v)| (*v, k.clone())).collect();
        s.next = s.next.max(s.numbers.keys().max().copied().unwrap_or(0));
        s
    }

    pub fn save(&self, store: &Store, account: &str) -> Result<()> {
        Ok(store.save_account_doc(account, STATE_DOC, self)?)
    }

    /// The number standing for a message id (given on first sight).
    pub fn number(&mut self, id: &str) -> u32 {
        if let Some(n) = self.ids.get(id) {
            return *n;
        }
        self.next = self.next.saturating_add(1);
        self.ids.insert(id.to_string(), self.next);
        self.numbers.insert(self.next, id.to_string());
        self.next
    }

    pub fn id(&self, n: u32) -> Option<&str> {
        self.numbers.get(&n).map(String::as_str)
    }

    /// The mailbox id behind a folder path.
    pub fn mailbox(&self, path: &str) -> Result<&str> {
        self.mailboxes.get(path).map(String::as_str).ok_or_else(|| Error::No(format!("there is no folder {} on the server", utf7::decode(path))))
    }

    /// Forget numbers no cached message uses any more.
    pub fn prune(&mut self, keep: &std::collections::HashSet<u32>) {
        self.ids.retain(|_, n| keep.contains(n));
        self.numbers.retain(|n, _| keep.contains(n));
        for v in self.found.values_mut() {
            v.retain(|n| keep.contains(n));
        }
        self.found.retain(|_, v| !v.is_empty());
    }

    /// Whether a search of the server found this message in this folder (kept past the window).
    pub fn was_found(&self, folder: &str, n: u32) -> bool {
        self.found.get(folder).is_some_and(|v| v.contains(&n))
    }

    /// What a folder's search of the server found, added to what earlier ones did (the newest
    /// finds first, [`FOUND_MAX`] at most), as IMAP keeps what it fetches.
    pub fn add_found(&mut self, folder: &str, found: &[u32]) {
        if found.is_empty() || (self.found.len() >= 1000 && !self.found.contains_key(folder)) {
            return;
        }
        let v = self.found.entry(folder.to_string()).or_default();
        let mut next: Vec<u32> = found.to_vec();
        next.extend(v.iter().filter(|n| !found.contains(n)));
        next.truncate(FOUND_MAX);
        *v = next;
    }

    /// Keep, of a folder's finds, those its server still has there.
    pub fn keep_found(&mut self, folder: &str, still: impl Fn(u32) -> bool) {
        if let Some(v) = self.found.get_mut(folder) {
            v.retain(|n| still(*n));
            if v.is_empty() {
                self.found.remove(folder);
            }
        }
    }

    /// The server ids of a folder's cached messages that are there only because a search found
    /// them (out of the newest window `wanted`): to check they're still there, and their flags.
    pub fn found_outside(&self, folder: &str, cached: &[Summary], wanted: &HashMap<u32, Flags>) -> Vec<String> {
        cached
            .iter()
            .filter(|m| !wanted.contains_key(&m.key.uid) && self.was_found(folder, m.key.uid))
            .filter_map(|m| self.id(m.key.uid).map(str::to_string))
            .collect()
    }
}

/// How a folder sync goes.
#[derive(Clone, Copy, Debug)]
pub struct SyncOptions {
    /// How many of the newest messages the first sync of a folder fetches.
    pub window: usize,
    /// How many of the newest message sources to keep downloaded.
    pub prefetch: usize,
    /// Sources larger than this are only downloaded when opened.
    pub prefetch_max_bytes: u32,
}

impl Default for SyncOptions {
    fn default() -> Self {
        SyncOptions { window: 1000, prefetch: 40, prefetch_max_bytes: 2 << 20 }
    }
}

/// What a folder sync changed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SyncReport {
    pub new: usize,
    pub changed: usize,
    pub removed: usize,
    pub total: u32,
    pub unread: u32,
    /// The cache was emptied because UIDVALIDITY changed.
    pub reset: bool,
}

impl SyncReport {
    pub fn touched(&self) -> bool {
        self.new + self.changed + self.removed > 0 || self.reset
    }
}

/// Fill in what only the full source tells (preview, attachments, size).
pub fn fill_from_source(m: &mut Summary, raw: &[u8]) {
    let full = mime::summary_from_full(raw, m.key.clone(), m.flags.clone(), m.size, m.date);
    m.preview = full.preview;
    m.has_attachments = full.has_attachments;
    // Entries built from a Graph listing carry no References or List-Id: take them now.
    let threaded = !m.in_reply_to.is_empty() || !m.references.is_empty();
    if !threaded {
        (m.in_reply_to, m.references) = (full.in_reply_to, full.references);
    }
    m.bulk = m.bulk || full.bulk;
    m.importance = if m.importance != 0 { m.importance } else { full.importance };
    if m.size == 0 {
        m.size = u32::try_from(raw.len()).unwrap_or(u32::MAX);
    }
}

/// A message's list entry from its `Email/get` details.
pub(super) fn email_summary(e: &Value, key: MessageKey) -> Summary {
    let mut header = Vec::new();
    for h in list(Some(e), "headers").iter().take(500) {
        let (Some(name), Some(value)) = (h.get("name").and_then(Value::as_str), h.get("value").and_then(Value::as_str)) else { continue };
        if name.is_empty() || name.contains([':', '\r', '\n']) {
            continue;
        }
        header.extend_from_slice(format!("{name}:{value}\r\n").as_bytes());
    }
    header.extend_from_slice(b"\r\n");
    let size = e.get("size").and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok()).unwrap_or(0);
    let received = e.get("receivedAt").and_then(Value::as_str).and_then(utc_date).unwrap_or(0);
    let mut s = mime::summary_from_header(&header, key, keywords(e.get("keywords")), size, received);
    s.preview = e.get("preview").and_then(Value::as_str).map(mime::preview).unwrap_or_default();
    s.has_attachments = e.get("hasAttachment").and_then(Value::as_bool).unwrap_or(false);
    s
}
