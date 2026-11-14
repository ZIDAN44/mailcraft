//! What the Exchange transports (EWS and Graph) do alike, written once. Above all a folder's
//! newest mail brought up to date (both list a folder's newest by id): what left the window goes,
//! flags change, new headers come in and the newest sources are brought in ahead, with what
//! differs between them as a [`Window`]. Then a search's finds kept, a source fetched, the ids of
//! a change and the `Bcc:` header Exchange wants. JMAP's sync is the same shape but async, in the
//! portable `mailcraft-jmap`, so not this one.

use std::collections::{HashMap, HashSet};

use mailcraft_mime as mime;
use mailcraft_model::{Flags, MessageKey, SharedSummary, Summary, sort_newest_first};
use mailcraft_store::{FolderState, Store};

use crate::jmap::{State, VALIDITY};
use crate::sync::{AheadFailures, SyncOptions, SyncReport, fill_from_source, if_kept};
use crate::{NetError, Result};

/// What a transport tells [`sync_folder_as`].
pub(crate) trait Window {
    /// A message as the server describes it (an EWS item, a Graph message).
    type Item;
    /// The newest `window` of the folder `id`: each one's id and flags, the folder's total and
    /// unread, and what the listing said of each, when it's enough to make a summary of.
    fn newest_in(&self, id: &str, window: usize) -> Result<(Vec<(String, Flags)>, (u32, u32), Vec<Self::Item>)>;
    /// Of `ids`, those still in the folder `id`, with their flags.
    fn still_in_folder(&self, id: &str, ids: &[String]) -> Result<Vec<(String, Flags)>>;
    /// The flags kept here, with what the server says of them put on (each transport says some).
    fn merged(&self, kept: &Flags, server: &Flags) -> Flags;
    /// The `missing` ones' headers; `listed`: what the listing said.
    fn headers_of(&self, missing: &[String], listed: Vec<Self::Item>) -> Result<Vec<Self::Item>>;
    fn id_of(item: &Self::Item) -> Option<&str>;
    fn summary(item: &Self::Item, key: MessageKey) -> Summary;
    /// Whether a message's source may be brought in ahead (by what's known of its size).
    fn ahead(&self, m: &Summary, opts: &SyncOptions) -> bool;
    /// A message's source; `None` when it turns out too big to keep ahead.
    fn source(&self, id: &str, opts: &SyncOptions) -> Result<Option<Vec<u8>>>;
}

/// Bring the folder's cache up to date, unless `account` was removed since it had been removed
/// `seen` times ([`if_kept`]): a removal while the sync talks to the server has it write nothing
/// of the account again.
pub(crate) fn sync_folder_as<W: Window>(
    c: &W,
    st: &mut State,
    store: &Store,
    account: &str,
    folder: &str,
    opts: &SyncOptions,
    seen: u64,
) -> Result<SyncReport> {
    let id = st.mailbox(folder)?.to_string();
    let (newest, (total, unread), listed) = c.newest_in(&id, opts.window)?;
    let mut fs = store.load_folder(account, folder)?;
    let mut report = SyncReport::default();
    if fs.validity != VALIDITY && (fs.validity != 0 || !fs.messages.is_empty()) {
        if_kept(store, account, seen, || store.clear_folder(account, folder))?;
        fs = FolderState::default();
        report.reset = true;
    }
    fs.validity = VALIDITY;
    let mut wanted: HashMap<u32, Flags> = newest.iter().map(|(id, f)| (st.number(id), f.clone())).collect();
    // Found by a search of the server past the window: kept while still in the folder.
    let extra = st.found_outside(folder, &fs.messages, &wanted);
    for (found, f) in c.still_in_folder(&id, &extra)? {
        wanted.insert(st.number(&found), f);
    }
    st.keep_found(folder, |n| wanted.contains_key(&n));
    let (kept, gone): (Vec<SharedSummary>, Vec<SharedSummary>) =
        std::mem::take(&mut fs.messages).into_iter().partition(|m| wanted.contains_key(&m.key.uid));
    if_kept(store, account, seen, || {
        gone.iter().for_each(|m| store.remove_body(&m.key));
        Ok(())
    })?;
    report.removed = gone.len();
    fs.messages = kept;
    for m in &mut fs.messages {
        if let Some(f) = wanted.get(&m.key.uid) {
            let next = c.merged(&m.flags, f);
            if next != m.flags {
                SharedSummary::make_mut(m).flags = next;
                report.changed += 1;
            }
        }
    }
    // New. (The numbers kept, in a set: looked up in the list, each of the window's messages was
    // a pass over the folder.)
    let mut have = kept_numbers(&fs);
    let missing: Vec<String> = newest.iter().filter(|(id, _)| !have.contains(&st.number(id))).map(|(id, _)| id.clone()).collect();
    for it in c.headers_of(&missing, listed)? {
        let Some(item_id) = W::id_of(&it) else { continue };
        let key = MessageKey { account: account.to_string(), folder: folder.to_string(), validity: VALIDITY, uid: st.number(item_id) };
        if !have.insert(key.uid) {
            continue;
        }
        fs.messages.push(W::summary(&it, key).into());
        report.new += 1;
    }
    sort_newest_first(&mut fs.messages);
    fs.uid_next = fs.messages.iter().map(|m| m.key.uid).max().unwrap_or(0).saturating_add(1);
    if_kept(store, account, seen, || store.save_folder(account, folder, &fs))?;
    report.fetched = prefetch(c, st, store, (account, seen), folder, &mut fs, opts)?;
    report.total = total;
    report.unread = unread;
    Ok(report)
}

/// The newest messages' sources brought into the cache, unless `account` was removed since it
/// had been removed `seen` times ([`if_kept`]).
fn prefetch<W: Window>(
    c: &W,
    st: &State,
    store: &Store,
    (account, seen): (&str, u64),
    folder: &str,
    fs: &mut FolderState,
    opts: &SyncOptions,
) -> Result<usize> {
    let wanted: Vec<(u32, String)> = fs
        .messages
        .iter()
        .take(opts.prefetch)
        .filter(|m| c.ahead(m, opts) && !store.has_body(&m.key))
        .filter_map(|m| Some((m.key.uid, st.id(m.key.uid)?.to_string())))
        .collect();
    if wanted.is_empty() {
        return Ok(0);
    }
    let at = places(fs);
    let mut fetched = 0;
    let mut failures = AheadFailures::new();
    for (n, id) in &wanted {
        let raw = match c.source(id, opts) {
            Ok(raw) => {
                failures.ok();
                raw
            }
            Err(e) => {
                failures.failed(&format!("{account}/{folder}: message {n}"), e)?;
                continue;
            }
        };
        let Some(raw) = raw else { continue };
        let Some(m) = at.get(n).and_then(|&i| fs.messages.get_mut(i)).map(SharedSummary::make_mut) else { continue };
        if_kept(store, account, seen, || store.write_body(&m.key, &raw))?;
        fill_from_source(m, &raw);
        fetched += 1;
    }
    if_kept(store, account, seen, || store.save_folder(account, folder, fs))?;
    Ok(fetched)
}

/// The numbers a folder's list holds.
pub(crate) fn kept_numbers(fs: &FolderState) -> HashSet<u32> {
    fs.messages.iter().map(|m| m.key.uid).collect()
}

/// Each message's place in a folder's list by its number: the first under it, as
/// [`FolderState::find`] finds (looked for there, each was a pass over the folder).
pub(crate) fn places(fs: &FolderState) -> HashMap<u32, usize> {
    fs.messages.iter().enumerate().rev().map(|(i, m)| (m.key.uid, i)).collect()
}

/// What a search of the server found in `folder` brought into its list `fs`: `numbers`, all it
/// found, are kept there past the window while the folder has them, and `new`, the summaries of
/// those not kept yet, go in. Their keys, and how many came in. Nothing written once `account`
/// was removed since it had been removed `seen` times ([`if_kept`]): an error then.
pub(crate) fn found(
    st: &mut State,
    store: &Store,
    (account, seen): (&str, u64),
    folder: &str,
    mut fs: FolderState,
    numbers: &[u32],
    new: Vec<Summary>,
) -> Result<(Vec<MessageKey>, usize)> {
    st.add_found(folder, numbers);
    let came = new.len();
    if came > 0 {
        fs.messages.extend(new.into_iter().map(Into::into));
        sort_newest_first(&mut fs.messages);
        fs.uid_next = fs.messages.iter().map(|m| m.key.uid).max().unwrap_or(0).saturating_add(1);
        if_kept(store, account, seen, || store.save_folder(account, folder, &fs))?;
    }
    // (The first kept under a number, as a search of the list found it.)
    let keys: HashMap<u32, &MessageKey> = fs.messages.iter().rev().map(|m| (m.key.uid, &m.key)).collect();
    Ok((numbers.iter().filter_map(|n| keys.get(n).map(|k| (*k).clone())).collect(), came))
}

/// The server's ids of `keys` (those numbered for it), with their folders.
pub(crate) fn server_ids(st: &State, keys: &[MessageKey]) -> Vec<(String, String)> {
    keys.iter().filter(|k| k.validity == VALIDITY).filter_map(|k| Some((st.id(k.uid)?.to_string(), k.folder.clone()))).collect()
}

/// A message's source, from the cache or else downloaded and kept, with its summary as its
/// folder now has it when it was downloaded. Not kept once the account was removed since it had
/// been removed `seen` times ([`if_kept`]): an error then.
pub(crate) fn fetch_body(
    st: &State,
    store: &Store,
    key: &MessageKey,
    seen: u64,
    download: impl FnOnce(&str) -> Result<Vec<u8>>,
) -> Result<(Vec<u8>, Option<Summary>)> {
    if let Some(raw) = store.read_body(key) {
        return Ok((raw, None));
    }
    let id = st.id(key.uid).filter(|_| key.validity == VALIDITY).ok_or_else(|| NetError::No("the message is no longer on the server".into()))?;
    let raw = download(id)?;
    let filled = if_kept(store, &key.account, seen, || crate::sync::keep_source(store, key, &raw))?;
    Ok((raw, filled))
}

/// A message as it goes to Exchange, which reads the recipients from the MIME source: anyone
/// only on the envelope (Bcc) in a `Bcc:` header, which Exchange removes before delivery. `None`
/// when there's no one.
pub(crate) fn with_bcc(raw: &[u8], recipients: &[String]) -> Option<Vec<u8>> {
    let shown: Vec<String> = mime::parse(raw).map(|p| p.to.iter().chain(&p.cc).map(|a| a.email.to_ascii_lowercase()).collect()).unwrap_or_default();
    let bcc: Vec<&str> = recipients.iter().map(String::as_str).filter(|r| !shown.contains(&r.to_ascii_lowercase())).collect();
    if bcc.is_empty() {
        return None;
    }
    let mut with = format!("Bcc: {}\r\n", bcc.join(", ")).into_bytes();
    with.extend_from_slice(raw);
    Some(with)
}
