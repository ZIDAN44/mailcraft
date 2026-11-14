//! An account's jobs, run in turn by its runner ([`super::run`]).

use mailcraft_engine::model::job::Vacation;
use mailcraft_engine::model::{Flags, MessageKey};

use super::*;

/// Said when what's asked needs the server and the account isn't connected.
const OFFLINE: &str = "the account is offline";
/// The browser's accounts are JMAP ones, with no organisation directory (issue #9).
const NO_DIRECTORY: &str = "this account has no organisation directory";

pub(super) async fn job(sh: &Rcs, id: &str, j: Job) {
    let store = store(sh);
    match j {
        Job::SyncAll { .. } => {
            if client(sh, id).is_some() {
                sync_all(sh, id).await;
            } else {
                connect(sh, id).await;
            }
        }
        Job::SyncFolder { folder, .. } => sync_one(sh, id, &folder).await,
        Job::FetchBody { key } => {
            let r = match client(sh, id) {
                Some(c) => mailcraft_jmap::fetch_body(&c, &state(sh, id), &store, &key, store.removals(id)).await,
                None => store.read_body(&key).map(|raw| (raw, None)).ok_or_else(|| Error::No(format!("not downloaded yet, and {OFFLINE}"))),
            };
            match r {
                Ok((raw, filled)) => {
                    emit(sh, Event::Body { key, raw });
                    if let Some(summary) = filled {
                        emit(sh, Event::Filled { summary: Box::new(summary) });
                    }
                }
                Err(e) => emit(sh, Event::BodyFailed { key, error: e.to_string() }),
            }
        }
        Job::Apply { op, .. } => {
            // The cache first, so the change shows at once.
            apply_local(sh, id, &op);
            remote(sh, id, op).await;
        }
        Job::Send { id: msg, raw, from, recipients, sent_folder: sent, .. } => {
            let Some(c) = client(sh, id) else {
                return emit(sh, Event::SendFailed { account: id.to_string(), id: msg, error: OFFLINE.into() });
            };
            let sent = sent_folder(sh, id, sent);
            match mailcraft_jmap::send(&c, &state(sh, id), sent.as_deref(), &raw, &from, &recipients).await {
                Ok(()) => {
                    emit(sh, Event::Sent { account: id.to_string(), id: msg });
                    if let Some(f) = sent {
                        sync_one(sh, id, &f).await;
                    }
                }
                Err(e) => emit(sh, Event::SendFailed { account: id.to_string(), id: msg, error: e.to_string() }),
            }
        }
        // Copies of mail kept here, into a folder on the server (issue #82), marked as they were.
        Job::FileCopies { folder, mail, moved, .. } => file_copies(sh, id, folder, mail, moved).await,
        // The browser has no closing to keep a draft through: it's filed as any other.
        Job::SaveDraft { raw, folder, replaces, .. } | Job::KeepDraft { raw, folder, replaces, .. } => {
            save_draft(sh, id, raw, folder, replaces).await
        }
        Job::FindIdentities { .. } => {
            let (found, error) = match client(sh, id) {
                Some(c) => match c.identities().await {
                    Ok(found) => (found, None),
                    Err(e) => (Vec::new(), Some(e.to_string())),
                },
                None => (Vec::new(), Some(OFFLINE.to_string())),
            };
            emit(sh, Event::IdentitiesFound { account: id.to_string(), found, error })
        }
        Job::GetVacation { .. } => vacation(sh, id, None).await,
        Job::SetVacation { vacation: v, .. } => vacation(sh, id, Some(v)).await,
        // Nothing to do in the browser: no Outbox (sending needs the server), no CalDAV.
        Job::FlushOutbox { .. } | Job::SyncDav { .. } => {}
        // The browser's accounts are JMAP ones, with no organisation directory (issue #9).
        Job::SearchDirectory { query, .. } => {
            emit(sh, Event::Directory { account: id.to_string(), query, people: Vec::new(), error: Some(NO_DIRECTORY.into()), consent: false })
        }
        Job::AllowDirectory { account } => emit(sh, Event::DirectoryAllowed { account: account.id, error: Some(NO_DIRECTORY.into()) }),
        // Mail past what's kept (issue #4).
        Job::SearchServer { folders, query, search, .. } => search_server(sh, id, &folders, query, search).await,
        Job::MoreMail { folder, more, .. } => {
            let windows = {
                let mut s = sh.borrow_mut();
                let (store, base) = (s.store.clone(), s.opts.window);
                let Some(a) = s.accounts.get_mut(id) else { return };
                let kept = store.load_folder(id, &folder).map(|f| f.messages.len()).unwrap_or(0);
                let now = a.windows.get(&folder).copied().unwrap_or(0).max(base).max(kept);
                a.windows.insert(folder.clone(), now.saturating_add(more as usize).min(1_000_000));
                a.windows.clone()
            };
            if let Err(e) = store.save_account_doc(id, WINDOWS, &windows) {
                log::warn!("{id}: {WINDOWS}: {e}");
            }
            sync_one(sh, id, &folder).await;
        }
        j @ (Job::ImportIdentity { .. }
        | Job::ForgetIdentity { .. }
        | Job::ForgetOlderIdentity { .. }
        | Job::KeepUnlocked { .. }
        | Job::UseOlderIdentity { .. }
        | Job::Protect { .. }
        | Job::Decrypt { .. }
        | Job::SealDraft { .. }) => smime::job(sh, id, j),
        Job::CheckRevocation { chain } => emit(
            sh,
            Event::Revocation {
                cert: chain.into_iter().next().unwrap_or_default(),
                status: mailcraft_engine::model::Revocation::Unknown { why: BROWSER_REVOCATION.into() },
            },
        ),
        Job::Pgp { work, .. } => pgp(sh, id, work),
        Job::FindHeldKeys { .. } => emit(sh, Event::HeldKeys { keys: Vec::new(), error: Some(BROWSER_SMIME.into()) }),
        Job::UseHeldKey { .. } => emit(
            sh,
            Event::Identity { account: id.to_string(), chain: Vec::new(), older: Vec::new(), for_reading: false, error: Some(BROWSER_SMIME.into()) },
        ),
        Job::Start { .. }
        | Job::SmtpPassword { .. }
        | Job::DavPassword { .. }
        | Job::CheckUpdates
        | Job::StopSearch
        | Job::Remove { .. }
        | Job::FetchUrl { .. }
        | Job::FetchCalendar { .. }
        | Job::Discover { .. }
        | Job::SignIn { .. }
        | Job::ImportPst { .. }
        | Job::RereadDataFile { .. }
        | Job::InspectDataFile { .. }
        | Job::FileMessages { .. }
        | Job::ReadMessageFile { .. }
        | Job::StopImport { .. }
        | Job::KeepImport { .. }
        | Job::ExportPst { .. }
        | Job::DataFileHealth { .. }
        | Job::CompactDataFile { .. }
        | Job::CancelCompact { .. }
        | Job::WorkOffline { .. }
        | Job::SetSync { .. }
        | Job::Index { .. }
        | Job::StopIndex { .. } => {}
    }
}

/// `op` made in the account's cache (so the change shows at once), said to the window; the
/// account's folder list kept up when it changes them. Accounts on a server and those with none
/// (the sample mailbox) alike.
pub(super) fn apply_local(sh: &Rcs, id: &str, op: &Op) {
    let store = store(sh);
    match store.apply_op(id, op) {
        Ok(changed) => {
            if op.is_folder_op() {
                let folders = store.load_folders(id).unwrap_or_default();
                if let Some(a) = sh.borrow_mut().accounts.get_mut(id) {
                    a.folders = folders.clone();
                }
                emit(sh, Event::Folders { account: id.to_string(), folders });
            }
            for folder in changed {
                emit(sh, Event::FolderChanged { account: id.to_string(), folder });
            }
        }
        Err(e) => notice(sh, id, e.to_string()),
    }
}

/// Copies of mail kept here filed in `folder` on the server (issue #82), marked as they were.
async fn file_copies(sh: &Rcs, id: &str, folder: String, mail: Vec<(MessageKey, Flags)>, moved: bool) {
    let store = store(sh);
    let (mut done, mut failed) = (Vec::new(), Vec::new());
    for (key, flags) in mail {
        let Some(c) = client(sh, id) else {
            failed.push((key, OFFLINE.to_string()));
            continue;
        };
        let Some(raw) = store.read_body(&key).filter(|raw| !mailcraft_engine::mime::is_text_first(raw)) else {
            failed.push((key, "it isn't downloaded".to_string()));
            continue;
        };
        let marks = [
            (flags.seen, "$seen"),
            (flags.flagged, "$flagged"),
            (flags.answered, "$answered"),
            (flags.draft, "$draft"),
            (flags.forwarded, "$forwarded"),
        ];
        let mut keywords: Vec<&str> = marks.iter().filter(|(on, _)| *on).map(|(_, k)| *k).collect();
        keywords.extend(flags.keywords.iter().filter(|k| !k.is_empty() && k.len() <= 64).take(32).map(String::as_str));
        match mailcraft_jmap::append(&c, &state(sh, id), &folder, &raw, &keywords).await {
            Ok(_) => done.push(key),
            Err(e) => failed.push((key, e.to_string())),
        }
    }
    let any = !done.is_empty();
    emit(sh, Event::CopiesFiled { account: id.to_string(), folder: folder.clone(), done, failed, moved });
    if any {
        sync_one(sh, id, &folder).await
    }
}

/// A draft filed in `folder` on the server, the one it `replaces` taken away; said either way
/// (by its `Message-ID`, for the session holding the message until it's saved: #53).
async fn save_draft(sh: &Rcs, id: &str, raw: Vec<u8>, folder: String, replaces: Option<MessageKey>) {
    let message_id = mailcraft_engine::mime::written_message_id(&raw);
    let Some(c) = client(sh, id) else {
        let error = Some(format!("the draft wasn't saved on the server: {OFFLINE}"));
        return emit(sh, Event::DraftSaved { account: id.to_string(), message_id, error });
    };
    let st = state(sh, id);
    let r = async {
        mailcraft_jmap::append(&c, &st, &folder, &raw, &["$draft", "$seen"]).await?;
        if let Some(old) = replaces {
            mailcraft_jmap::apply_remote(&c, &st, &Op::Expunge { keys: vec![old] }).await?;
        }
        Ok::<(), Error>(())
    }
    .await;
    match r {
        Ok(()) => {
            emit(sh, Event::DraftSaved { account: id.to_string(), message_id, error: None });
            sync_one(sh, id, &folder).await
        }
        Err(e) => {
            emit(sh, Event::DraftSaved { account: id.to_string(), message_id, error: Some(e.to_string()) });
            failed(sh, id, e)
        }
    }
}

/// The automatic replies read, after `change` is made to them if given.
async fn vacation(sh: &Rcs, id: &str, change: Option<Vacation>) {
    let Some(c) = client(sh, id) else {
        return emit(sh, Event::Vacation { account: id.to_string(), vacation: None, error: Some(OFFLINE.into()) });
    };
    let mut failed_change = None;
    if let Some(v) = &change
        && let Err(e) = c.set_vacation(v).await
    {
        failed_change = Some(e.to_string());
    }
    let (vacation, error) = match c.vacation().await {
        Ok(v) => (Some(v), failed_change),
        Err(e) => (None, Some(failed_change.unwrap_or_else(|| e.to_string()))),
    };
    emit(sh, Event::Vacation { account: id.to_string(), vacation, error });
}
