//! The drafts kept on this computer, as they're kept: a short list (where each goes, the copy it
//! replaces, what it is) and each one's source in a file of its own, as the Outbox keeps its
//! messages (found in audit: the sources were in the list itself, a byte a number, and the whole
//! list read and written again for each draft kept, filed or looked for). The list an earlier
//! version kept, sources and all, is moved into this one the first time it's read.

use super::*;

/// The list of drafts kept on this computer; their sources are the store's
/// ([`Store::kept_draft_add`]).
pub(crate) const KEPT_DRAFTS: &str = "drafts-kept.json";

/// What a damaged list is set aside as: while it's there, a source the list doesn't name may be
/// one of its drafts, and is left.
const SET_ASIDE: &str = "drafts-kept.json.bad";

/// The kept drafts' list as an earlier version kept it, each draft's source in it
/// ([`PendingDraft`]): read only, to move them into this one.
pub(crate) const PENDING_DRAFTS: &str = "drafts-pending.json";

/// A draft in the list as an earlier version kept it, its source and all.
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct PendingDraft {
    pub(crate) folder: String,
    #[serde(default)]
    pub(crate) replaces: Option<MessageKey>,
    pub(crate) raw: Vec<u8>,
}

/// A draft's source, as it's known without reading it: its SHA-256 and length.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Digest {
    /// In hex.
    pub(crate) sha256: String,
    pub(crate) len: u64,
}

impl Digest {
    pub(crate) fn of(raw: &[u8]) -> Digest {
        let sha = ring::digest::digest(&ring::digest::SHA256, raw);
        let sha256 = crate::hex(sha.as_ref());
        Digest { sha256, len: raw.len() as u64 }
    }

    /// The draft's hash, which this run knows it by ([`hash_of`]): its SHA-256's first 8 bytes.
    /// None that can be read (a damaged list) is 0, which matches no source's.
    pub(crate) fn hash(&self) -> u64 {
        self.sha256.get(..16).and_then(|h| u64::from_str_radix(h, 16).ok()).unwrap_or(0)
    }
}

/// A draft kept on this computer: its source (`<id>.eml`, [`Store::kept_draft_raw`]), the folder
/// it goes in, the copy it replaces, its Message-ID, and what its source is.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct Kept {
    pub(crate) id: u64,
    pub(crate) folder: String,
    #[serde(default)]
    pub(crate) replaces: Option<MessageKey>,
    #[serde(default)]
    pub(crate) message_id: String,
    #[serde(flatten)]
    pub(crate) source: Digest,
}

impl Kept {
    /// Which draft it's a version of.
    pub(crate) fn draft(&self) -> DraftId<'_> {
        DraftId { message_id: &self.message_id, replaces: self.replaces.as_ref() }
    }
}

/// The kept drafts' list; those an earlier version kept moved into it first ([`move_old`]). With
/// the kept drafts' file held ([`pending_file`]).
pub(super) fn load(store: &Store, account: &str) -> Result<Vec<Kept>> {
    let mut list: Vec<Kept> = store.load_account_doc(account, KEPT_DRAFTS)?;
    if let Some(old) = store.load_account_doc_bytes(account, PENDING_DRAFTS)? {
        move_old(store, account, &mut list, &Digest::of(&old))?;
    }
    Ok(list)
}

/// Written when the old list's drafts were moved and it couldn't be removed: what that file is
/// ([`Digest`]), so it isn't moved again while it stays (a draft of it filed since would be
/// kept, and filed, once more at each connection).
const PENDING_MOVED: &str = "drafts-pending-moved.json";

/// The drafts an earlier version kept, each source in the list itself: each written to a file of
/// its own and added to this list, which is written, and only then the old list removed. Stopped
/// between, both are there, and a draft this list has already (the same source) isn't added
/// again; one that can't be written leaves the old list as it was.
///
/// An old list that can't be removed once moved (`old_is`, the file) doesn't stop the drafts
/// being read, kept and filed: it's noted as moved ([`PENDING_MOVED`]), and removed at a later
/// read. Only when that can't be written either is this an error.
fn move_old(store: &Store, account: &str, list: &mut Vec<Kept>, old_is: &Digest) -> Result<()> {
    let noted: Option<Digest> = store.load_account_doc(account, PENDING_MOVED)?;
    let moved = noted.as_ref() == Some(old_is);
    if !moved {
        let old: Vec<PendingDraft> = store.load_account_doc(account, PENDING_DRAFTS)?;
        let mut written = Vec::new();
        for d in old {
            let source = Digest::of(&d.raw);
            if list.iter().any(|k| k.source == source) {
                continue;
            }
            let id = next_id(list);
            let message_id = written_message_id(&d.raw);
            if let Err(e) = store.kept_draft_add(account, id, &d.raw) {
                forget_sources(store, account, &written);
                return Err(e.into());
            }
            written.push(id);
            list.push(Kept { id, folder: d.folder, replaces: d.replaces, message_id, source });
        }
        if !written.is_empty()
            && let Err(e) = save(store, account, list)
        {
            forget_sources(store, account, &written);
            return Err(e);
        }
        log::info!("{account}: {} kept drafts moved to a file each", written.len());
    }
    match store.remove_account_doc(account, PENDING_DRAFTS) {
        Ok(()) if noted.is_some() => {
            if let Err(e) = store.remove_account_doc(account, PENDING_MOVED) {
                log::warn!("{account}: {e}");
            }
        }
        Ok(()) => {}
        Err(_) if moved => {}
        Err(e) => {
            log::warn!("{account}: the kept drafts' old list was moved but couldn't be removed: {e}");
            store.save_account_doc(account, PENDING_MOVED, old_is)?;
        }
    }
    Ok(())
}

pub(super) fn save(store: &Store, account: &str, list: &[Kept]) -> Result<()> {
    store.save_account_doc(account, KEPT_DRAFTS, list).map_err(NetError::from)
}

/// An id for a draft's source no draft in `list` has: never one a draft had a moment ago
/// either (from the clock, in milliseconds), so a draft read for a connection and replaced since
/// isn't taken for the one that replaced it.
fn next_id(list: &[Kept]) -> u64 {
    let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
    let after = list.iter().map(|k| k.id).max().map_or(0, |m| m.saturating_add(1));
    after.max(ms)
}

/// Keep a draft's source (`source`, what it is), and its entry for the list (not written yet);
/// `taken`, the list as it was, whose ids it doesn't take (those leaving it too: their sources go
/// after it's written).
pub(super) fn add(
    store: &Store,
    account: &str,
    taken: &[Kept],
    folder: &str,
    replaces: Option<MessageKey>,
    raw: &[u8],
    source: Digest,
) -> Result<Kept> {
    let id = next_id(taken);
    store.kept_draft_add(account, id, raw)?;
    Ok(Kept { id, folder: folder.to_string(), replaces, message_id: written_message_id(raw), source })
}

/// The sources of drafts the list doesn't name any more, once it's written: removed. One that
/// can't be is said in the log, and left.
pub(super) fn remove_sources<'a>(store: &Store, account: &str, gone: impl IntoIterator<Item = &'a Kept>) {
    for k in gone {
        if let Err(e) = store.kept_draft_remove(account, k.id) {
            log::warn!("{account}: a kept draft's source wasn't removed: {e}");
        }
    }
}

/// Sources written for a list that couldn't be written: removed, as best they can be.
pub(super) fn forget_sources(store: &Store, account: &str, ids: &[u64]) {
    for id in ids {
        let _ = store.kept_draft_remove(account, *id);
    }
}

/// A kept draft read for a connection, if the list still has it (not filed since, nor replaced by
/// a newer version): its source. One whose source is gone is taken off the list (there's nothing
/// to file); one whose source can't be read now stays, for the next connection. Nothing when the
/// account was removed since its worker started (`removals`, [`Store::while_kept`]).
pub(super) fn still_kept(store: &Store, account: &str, removals: u64, d: &Kept) -> Option<Vec<u8>> {
    store.while_kept(account, removals, || read_kept(store, account, d)).flatten()
}

fn read_kept(store: &Store, account: &str, d: &Kept) -> Option<Vec<u8>> {
    let _one_at_a_time = pending_file();
    let list = load(store, account).ok()?;
    if !list.iter().any(|k| k.id == d.id && k.source == d.source) {
        return None;
    }
    match store.kept_draft_raw(account, d.id) {
        Ok(Some(raw)) => {
            if Digest::of(&raw) != d.source {
                log::warn!("{account}: a kept draft's source isn't as it was kept; it's filed as it is");
            }
            Some(raw)
        }
        Ok(None) => {
            log::warn!("{account}: a kept draft's source is gone; it's kept no more");
            let left: Vec<Kept> = list.into_iter().filter(|k| k.id != d.id).collect();
            if let Err(e) = save(store, account, &left) {
                log::warn!("{account}: {e}");
            }
            None
        }
        Err(e) => {
            log::warn!("{account}: a kept draft's source couldn't be read: {e}");
            None
        }
    }
}

/// Sources the list doesn't name (MailCraft stopped between writing one and the list, or between
/// writing the list and removing one): removed. Not while a damaged list is set aside, whose
/// drafts they may be.
pub(crate) fn sweep(store: &Store, account: &str, list: &[Kept]) {
    if !matches!(store.load_account_doc_bytes(account, SET_ASIDE), Ok(None)) {
        return;
    }
    for id in store.kept_draft_ids(account) {
        if !list.iter().any(|k| k.id == id) {
            log::info!("{account}: a kept draft's source no draft names is removed");
            let _ = store.kept_draft_remove(account, id);
        }
    }
}

/// The kept drafts' sources, in the list's order.
#[cfg(test)]
pub(crate) fn kept_raws(store: &Store, account: &str) -> Result<Vec<Vec<u8>>> {
    let _one_at_a_time = pending_file();
    let list = load(store, account)?;
    list.iter().map(|k| Ok(store.kept_draft_raw(account, k.id)?.unwrap_or_default())).collect()
}
