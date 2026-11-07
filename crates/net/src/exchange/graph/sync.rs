//! Bringing a folder's cache up to date from Microsoft Graph.

use super::*;

/// Bring one folder's cache up to date.
pub fn sync_folder(c: &Client, st: &mut State, store: &Store, account: &str, folder: &str, opts: &SyncOptions) -> Result<SyncReport> {
    sync_folder_as(c, st, store, account, folder, opts, store.removals(account))
}

/// [`sync_folder`] for a worker that started when its account had been removed `seen` times: a
/// removal since, while the sync talks to the server, has it write nothing of the account again.
pub fn sync_folder_as(c: &Client, st: &mut State, store: &Store, account: &str, folder: &str, opts: &SyncOptions, seen: u64) -> Result<SyncReport> {
    let id = st.mailbox(folder)?.to_string();
    let (listed, (total, unread)) = c.newest(&id, opts.window)?;
    let newest: Vec<(String, Flags)> = listed.iter().filter_map(|m| Some((m.get("id")?.as_str()?.to_string(), flags(m)))).collect();
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
    for (found, f) in c.still_in(&id, &extra)? {
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
        // Graph has no answered flag or categories here: keep those as they are.
        if let Some(f) = wanted.get(&m.key.uid) {
            let mut next = m.flags.clone();
            next.seen = f.seen;
            next.flagged = f.flagged;
            next.draft = f.draft;
            next.set_keyword(mailcraft_model::FOLLOW_UP_COMPLETE, f.follow_up_complete());
            if next != m.flags {
                SharedSummary::make_mut(m).flags = next;
                report.changed += 1;
            }
        }
    }
    let missing: Vec<String> = newest.iter().filter(|(id, _)| fs.find(st.number(id)).is_none()).map(|(id, _)| id.clone()).collect();
    // Small gap: per-message GETs with internet headers. Big gap: reuse the listing entries.
    let details = if missing.len() <= FEW {
        c.headers(&missing)?
    } else {
        let mut sorted = missing.clone();
        sorted.sort_unstable();
        listed.iter().filter(|m| m["id"].as_str().is_some_and(|i| sorted.binary_search_by(|x| x.as_str().cmp(i)).is_ok())).cloned().collect()
    };
    for m in details {
        let Some(id) = m.get("id").and_then(Value::as_str) else { continue };
        let key = MessageKey { account: account.to_string(), folder: folder.to_string(), validity: VALIDITY, uid: st.number(id) };
        if fs.find(key.uid).is_some() {
            continue;
        }
        fs.messages.push(message_summary(&m, key).into());
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
fn prefetch(
    c: &Client,
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
        // Graph lists no sizes: mail with attachments waits until it's opened.
        .filter(|m| !m.has_attachments && !store.has_body(&m.key))
        .filter_map(|m| Some((m.key.uid, st.id(m.key.uid)?.to_string())))
        .collect();
    if wanted.is_empty() {
        return Ok(0);
    }
    let mut fetched = 0;
    let mut failures = crate::sync::AheadFailures::new();
    for (n, id) in &wanted {
        let raw = match c.download(id) {
            Ok(raw) => {
                failures.ok();
                raw
            }
            Err(e) => {
                failures.failed(&format!("{account}/{folder}: message {n}"), e)?;
                continue;
            }
        };
        if raw.len() > usize::try_from(opts.prefetch_max_bytes).unwrap_or(usize::MAX) {
            continue;
        }
        let Some(m) = fs.find_mut(*n) else { continue };
        if_kept(store, account, seen, || store.write_body(&m.key, &raw))?;
        fill_from_source(m, &raw);
        fetched += 1;
    }
    if_kept(store, account, seen, || store.save_folder(account, folder, fs))?;
    Ok(fetched)
}
