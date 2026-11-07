//! Bringing a folder's cache up to date from the JMAP server.

use super::*;

/// Bring one folder's cache up to date.
pub async fn sync_folder<H: Http>(
    c: &Client<H>,
    st: &mut State,
    store: &Store,
    account: &str,
    folder: &str,
    opts: &SyncOptions,
) -> Result<SyncReport> {
    sync_folder_as(c, st, store, account, folder, opts, store.removals(account)).await
}

/// A change to `account`'s cache, unless the account was removed since [`Store::removals`] said
/// `seen` ([`Store::while_kept`]): an error then, which ends the sync.
fn if_kept<T>(store: &Store, account: &str, seen: u64, write: impl FnOnce() -> mailcraft_store::Result<T>) -> Result<T> {
    Ok(store.while_kept(account, seen, write).ok_or_else(|| Error::Closed("the account was removed".into()))??)
}

/// [`sync_folder`] for a worker that started when its account had been removed `seen` times: a
/// removal since, while the sync talks to the server, has it write nothing of the account again.
pub async fn sync_folder_as<H: Http>(
    c: &Client<H>,
    st: &mut State,
    store: &Store,
    account: &str,
    folder: &str,
    opts: &SyncOptions,
    seen: u64,
) -> Result<SyncReport> {
    let mailbox = st.mailbox(folder)?.to_string();
    let (newest, counts) = c.newest(&mailbox, opts.window).await?;
    let mut fs = store.load_folder(account, folder)?;
    let mut report = SyncReport::default();
    if fs.validity != VALIDITY && (fs.validity != 0 || !fs.messages.is_empty()) {
        if_kept(store, account, seen, || store.clear_folder(account, folder))?;
        fs = FolderState::default();
        report.reset = true;
    }
    fs.validity = VALIDITY;
    let wanted: HashMap<u32, Flags> = newest.iter().map(|(id, f)| (st.number(id), f.clone())).collect();
    let mut wanted = wanted;
    // Found by a search of the server past the window: kept while still in the mailbox.
    let extra = st.found_outside(folder, &fs.messages, &wanted);
    for (id, f) in c.still_in(&mailbox, &extra).await? {
        wanted.insert(st.number(&id), f);
    }
    st.keep_found(folder, |n| wanted.contains_key(&n));
    // Gone (or out of the window), and flag changes.
    let (kept, gone): (Vec<SharedSummary>, Vec<SharedSummary>) =
        std::mem::take(&mut fs.messages).into_iter().partition(|m| wanted.contains_key(&m.key.uid));
    if_kept(store, account, seen, || {
        gone.iter().for_each(|m| store.remove_body(&m.key));
        Ok(())
    })?;
    report.removed = gone.len();
    fs.messages = kept;
    for m in &mut fs.messages {
        if let Some(f) = wanted.get(&m.key.uid)
            && m.flags != *f
        {
            SharedSummary::make_mut(m).flags = f.clone();
            report.changed += 1;
        }
    }
    // New.
    let missing: Vec<String> =
        newest.iter().filter(|(id, _)| st.ids.get(id).is_some_and(|n| fs.find(*n).is_none())).map(|(id, _)| id.clone()).collect();
    for e in c.headers(&missing).await? {
        let Some(id) = e.get("id").and_then(Value::as_str) else { continue };
        let key = MessageKey { account: account.to_string(), folder: folder.to_string(), validity: VALIDITY, uid: st.number(id) };
        if fs.find(key.uid).is_some() {
            continue;
        }
        fs.messages.push(email_summary(&e, key).into());
        report.new += 1;
    }
    sort_newest_first(&mut fs.messages);
    fs.uid_next = fs.messages.iter().map(|m| m.key.uid).max().unwrap_or(0).saturating_add(1);
    if_kept(store, account, seen, || store.save_folder(account, folder, &fs))?;
    report.fetched = prefetch(c, st, store, (account, seen), folder, &mut fs, opts).await?;
    let (total, unread) = counts.unwrap_or((u32::try_from(fs.messages.len()).unwrap_or(u32::MAX), fs.unread()));
    report.total = total;
    report.unread = unread;
    Ok(report)
}

/// The newest messages' sources brought into the cache, unless `account` was removed since it
/// had been removed `seen` times ([`if_kept`]).
async fn prefetch<H: Http>(
    c: &Client<H>,
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
        .filter(|m| m.size <= opts.prefetch_max_bytes && !store.has_body(&m.key))
        .filter_map(|m| Some((m.key.uid, st.id(m.key.uid)?.to_string())))
        .collect();
    if wanted.is_empty() {
        return Ok(0);
    }
    let blobs = c.blobs(&wanted.iter().map(|(_, id)| id.clone()).collect::<Vec<_>>()).await?;
    let mut fetched = 0;
    for (n, id) in &wanted {
        let Some(blob) = blobs.get(id) else { continue };
        let raw = c.download(blob).await?;
        let Some(m) = fs.find_mut(*n) else { continue };
        if_kept(store, account, seen, || store.write_body(&m.key, &raw))?;
        fill_from_source(m, &raw);
        fetched += 1;
    }
    if_kept(store, account, seen, || store.save_folder(account, folder, fs))?;
    Ok(fetched)
}
