//! Exporting an account's mail to an Outlook data file (`.pst`).
//!
//! The file is written with the MIT-licensed `outlook-pst-rw` crate (MS-PST, Unicode format),
//! from MailCraft's fork: each folder kept on this computer becomes a folder of the same name,
//! and the account's Sent, Trash, Drafts and Junk folders go to the file's own Sent Items,
//! Deleted Items, Drafts and Junk Email ([`place`]). Each message whose body is here goes in with
//! its addresses (Bcc too), text and HTML bodies, attachments and header block, and with what the
//! store keeps of it: read or not, a draft, flagged (and when it's due), answered or forwarded,
//! its categories, importance and sensitivity ([`item`]). Messages only known by their headers
//! (never opened, on an account that keeps headers only) are counted as skipped. The account's
//! contacts, contact groups, notes and tasks go in as Outlook's own items, in the file's Contacts,
//! Notes and Tasks folders ([`pim`]). Events aren't mapped onto Outlook's items yet, so the
//! account's calendar goes beside the file as `… Calendar.ics`, which every calendar reads.
//!
//! A data file that's already there is added to, as Outlook's export does (issue #18): its mail
//! stays, the account's goes into folders of the same names, and messages already in it (by
//! `Message-ID`) aren't written twice. That is a file in the Unicode format Outlook 2003 and later
//! make, encoded as Outlook encodes them (the writer is MailCraft's fork, which adds to those) or
//! not; Outlook 97–2002's ANSI format can't be added to. The file is written as a copy that
//! replaces it at the end, so a failed export leaves it as it was.

use std::collections::{BTreeMap, HashSet};
use std::io;
use std::path::Path;
use std::rc::Rc;

use mailcraft_mime as mime;
use mailcraft_model::{Folder, FolderRole, MessageKey, utf7};
use mailcraft_store::Store;
use outlook_pst::messaging::message::{Message as _, UnicodeMessage};
use outlook_pst::messaging::store::{Store as PstStore, UnicodeStore};
use outlook_pst::ndb::node_id::NodeId;
use outlook_pst::{
    UnicodePstAttachment, UnicodePstBatchMessage, UnicodePstFile, UnicodePstMessage, UnicodePstProperty, UnicodePstPropertyKey, UnicodePstRecipient,
    UnicodePstRecipientType,
};

mod contact;
mod item;
mod pattern;
mod pim;
mod place;
mod task;
#[cfg(test)]
mod tests;

use item::Owned;

/// Messages written per commit to the file, at most: the writer rewrites the folder's table of
/// contents with each, so a big folder written a hundred at a time took longer with every batch.
const BATCH: usize = 2000;
/// …and their bytes (attachments and all), at most.
const BATCH_BYTES: usize = 48 << 20;
/// Attachments larger than this are left out.
const MAX_ATTACHMENT: usize = 100 * 1024 * 1024;
/// How deep folders in a data file already there are looked through.
const MAX_DEPTH: usize = 64;

/// What an export did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Exported {
    pub messages: usize,
    /// Messages whose body isn't on this computer, or that couldn't be read.
    pub skipped: usize,
    /// Messages a data file already there had (by `Message-ID`), not written again.
    pub already: usize,
    /// Events written beside the data file (its calendar).
    pub events: usize,
    /// The account's tasks, notes and people (contact groups among them) written into it…
    pub tasks: usize,
    pub notes: usize,
    pub contacts: usize,
    /// …and those a file added to had already, as they are now (an export marks each it writes).
    pub pim_already: usize,
}

/// `wVer` of Outlook 97–2002's ANSI data files (MS-PST 2.2.2.6: 14 or 15; Unicode files say 23
/// or more).
const ANSI_VERSIONS: [u16; 2] = [14, 15];

/// The `Message-ID`s in an existing data file's folders (a file the writer can add to: the Unicode
/// format, encoded or not, not Outlook 97–2002's ANSI one nor Outlook's offline copy of a server
/// account).
fn ids_in(path: &Path) -> Result<HashSet<String>, String> {
    let mut head = [0u8; 12];
    std::fs::File::open(path).and_then(|mut f| io::Read::read_exact(&mut f, &mut head)).map_err(|e| e.to_string())?;
    if !head.starts_with(b"!BDN") || head.get(8..10) != Some(b"SM") {
        return Err("it isn't an Outlook data file: choose another name to make a new one".into());
    }
    if ANSI_VERSIONS.contains(&u16::from_le_bytes([head[10], head[11]])) {
        return Err("this data file is in Outlook 97–2002's format, which mail can't be added to yet: choose another name to make a new one".into());
    }
    let read = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> io::Result<HashSet<String>> {
        let pst = outlook_pst::UnicodePstFile::open(path)?;
        pst.set_max_value_size(super::pst::MAX_VALUE);
        let store = UnicodeStore::read(Rc::new(pst))?;
        let mut ids = HashSet::new();
        let any: Rc<dyn PstStore> = store.clone();
        let top = any.open_folder(&any.properties().ipm_sub_tree_entry_id()?)?;
        let mut stack = vec![(top, 0usize)];
        while let Some((folder, depth)) = stack.pop() {
            if let Some(contents) = folder.contents_table() {
                for row in contents.rows_matrix() {
                    let Ok(entry) = any.properties().make_entry_id(NodeId::from(u32::from(row.id()))) else { continue };
                    let Ok(m) = UnicodeMessage::read(store.clone(), &entry, Some(&[0x1035])) else { continue };
                    if let Some(id) = m.properties().get(0x1035).and_then(crate::pst::text_of) {
                        ids.insert(mailcraft_model::thread::norm_id(&id));
                    }
                }
            }
            if depth < MAX_DEPTH
                && let Some(table) = folder.hierarchy_table()
            {
                for row in table.rows_matrix() {
                    if let Ok(sub) = any.properties().make_entry_id(NodeId::from(u32::from(row.id()))).and_then(|e| any.open_folder(&e)) {
                        stack.push((sub, depth + 1));
                    }
                }
            }
        }
        Ok(ids)
    }));
    match read {
        Ok(Ok(ids)) => Ok(ids),
        Ok(Err(e)) if matches!(e.raw_os_error(), Some(32 | 33)) => {
            Err("Outlook (or another program) has this data file open: close Outlook, including from the notification area, and try again".into())
        }
        Ok(Err(_)) | Err(_) => Err("this data file can't be read, so mail can't be added to it: choose another name to make a new one".into()),
    }
}

/// Write `account`'s mail to a data file at `path`, named `name` when it's new; a file already
/// there is added to. `follow_up`: when flagged messages are due (local days, by
/// [`mailcraft_model::Summary::follow_up_id`]). Calls `progress` with the number of messages
/// written so far.
pub fn export(
    store: &Store,
    account: &str,
    name: &str,
    path: &Path,
    follow_up: &BTreeMap<String, i64>,
    progress: &mut dyn FnMut(usize),
) -> Result<Exported, String> {
    let folders = store.load_folders(account).map_err(|e| e.to_string())?;
    let mut done = Exported::default();
    let temp = path.with_extension("pst.part");
    let _ = std::fs::remove_file(&temp);
    // Added to: a copy of it, with the messages it has already left out.
    let existing = path.is_file();
    let had = if existing { ids_in(path)? } else { HashSet::new() };
    if existing {
        std::fs::copy(path, &temp).map_err(|e| e.to_string())?;
    }
    let mut created = existing;
    // The special folders a file added to records, else the names a new one gets.
    let targets = place::Targets::of(existing.then_some(temp.as_path()));
    let pim: mailcraft_model::pim::Pim = store.load_account_doc(account, mailcraft_model::pim::PIM_DOC).unwrap_or_default();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| i64::try_from(d.as_secs()).unwrap_or(0)).unwrap_or(0);
    // Where the mail went, so the contacts, notes and tasks of a new file don't go there.
    let mut taken: Vec<Vec<String>> = Vec::new();
    let mut inside = pim::Written::default();
    let written = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<(), String> {
        for f in folders.iter().filter(|f| f.selectable) {
            let Ok(st) = store.load_folder(account, &f.path) else { continue };
            let folder = place::place(f, &folders, &targets);
            taken.push(folder.clone());
            let folder: Vec<&str> = folder.iter().map(String::as_str).collect();
            let mut batch = Vec::new();
            let mut bytes = 0usize;
            for s in &st.messages {
                let key = MessageKey { account: account.to_string(), folder: f.path.clone(), validity: s.key.validity, uid: s.key.uid };
                let state = item::state(s, f.role, follow_up);
                match store.read_body(&key).and_then(|raw| item::owned(&raw, s.received, state)) {
                    Some(m) if !m.message_id.is_empty() && had.contains(&mailcraft_model::thread::norm_id(&m.message_id)) => done.already += 1,
                    Some(m) => {
                        bytes += m.body.len() + m.html.as_ref().map_or(0, String::len) + m.files.iter().map(|f| f.3.len()).sum::<usize>();
                        batch.push(m);
                    }
                    None => done.skipped += 1,
                }
                if batch.len() >= BATCH || bytes >= BATCH_BYTES {
                    let n = batch.len();
                    write(&temp, &folder, &std::mem::take(&mut batch), &mut created)?;
                    bytes = 0;
                    done.messages += n;
                    progress(done.messages);
                }
            }
            if !batch.is_empty() {
                write(&temp, &folder, &batch, &mut created)?;
                done.messages += batch.len();
                progress(done.messages);
            }
        }
        record_special(&temp, &folders, &targets, created);
        inside = pim::write(&temp, &pim, &taken, existing, &mut created, now)?;
        Ok(())
    }));
    let result = match written {
        Ok(Ok(())) if !created => Err("there is no mail on this computer to export".to_string()),
        // A file added to keeps its own name.
        Ok(Ok(())) if existing => Ok(()),
        Ok(Ok(())) => UnicodePstFile::set_store_display_name(&temp, name).map(drop).map_err(|e| e.to_string()),
        Ok(Err(e)) => Err(e),
        Err(_) => Err("the data file could not be written".into()),
    };
    match result {
        Ok(()) => {
            // `rename` replaces a file already there (MoveFileExW with MOVEFILE_REPLACE_EXISTING on
            // Windows), so the old file stays until the new one takes its place; if it can't, the
            // mail is still in the `.pst.part` file, and the error says so.
            if let Err(e) = std::fs::rename(&temp, path) {
                return Err(format!("the data file couldn't be replaced ({e}): the mail is in {}", temp.display()));
            }
            let events = calendar_beside(&pim, path, now).map_err(|e| e.to_string())?;
            Ok(Exported { events, tasks: inside.tasks, notes: inside.notes, contacts: inside.contacts, pim_already: inside.already, ..done })
        }
        Err(e) => {
            let _ = std::fs::remove_file(&temp);
            Err(e)
        }
    }
}

/// The account's calendar beside the data file (its tasks are inside it): the events written.
fn calendar_beside(pim: &mailcraft_model::pim::Pim, path: &Path, now: i64) -> mailcraft_store::Result<usize> {
    let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "Mail".into());
    let dir = path.parent().unwrap_or(Path::new("."));
    let events: Vec<mailcraft_model::ical::CalEvent> = pim.events.iter().map(|e| e.event.clone()).collect();
    if !events.is_empty() {
        let ics = mailcraft_model::ical::write_calendar(&events, &[], now);
        mailcraft_store::write_atomic(&dir.join(format!("{stem} Calendar.ics")), ics.as_bytes())?;
    }
    Ok(events.len())
}

/// Record the folders of the account's special roles as the file's own special folders, unless
/// the file records one already (a file added to keeps its own); made when they have no mail.
/// The mail is in by then: a file that can't take the record keeps it as written, and the log
/// says why.
fn record_special(path: &Path, folders: &[Folder], targets: &place::Targets, created: bool) {
    if !created {
        return;
    }
    for (role, special) in place::SPECIAL {
        if !folders.iter().any(|f| f.selectable && f.role == role) {
            continue;
        }
        match UnicodePstFile::special_folder(path, special) {
            Ok(Some(_)) => continue,
            Ok(None) => {}
            Err(e) => {
                log::info!("the data file's {special:?} can't be read: {e}");
                continue;
            }
        }
        let names = targets.get(special);
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        if let Err(e) = UnicodePstFile::set_special_folder(path, special, &names) {
            log::info!("the data file's {special:?} can't be recorded: {e}");
        }
    }
}

fn write(path: &Path, folder: &[&str], batch: &[Owned], created: &mut bool) -> Result<(), String> {
    let recipients: Vec<Vec<UnicodePstRecipient<'_>>> = batch
        .iter()
        .map(|m| m.recipients.iter().map(|(name, email, kind)| UnicodePstRecipient { name, email, recipient_type: *kind }).collect())
        .collect();
    let files: Vec<Vec<UnicodePstAttachment<'_>>> = batch
        .iter()
        .map(|m| {
            m.files
                .iter()
                .map(|(filename, mime_type, cid, data)| UnicodePstAttachment {
                    filename,
                    mime_type,
                    content_id: cid.as_deref(),
                    data,
                    ..Default::default()
                })
                .collect()
        })
        .collect();
    let categories: Vec<Vec<&str>> = batch.iter().map(|m| m.state.categories.iter().map(String::as_str).collect()).collect();
    let properties: Vec<_> = batch.iter().zip(&categories).map(|(m, c)| item::properties(m, c)).collect();
    let messages: Vec<UnicodePstBatchMessage<'_>> = batch
        .iter()
        .zip(&recipients)
        .zip(&files)
        .zip(&properties)
        .map(|(((m, r), f), p)| UnicodePstBatchMessage {
            message: UnicodePstMessage {
                subject: &m.subject,
                sender_name: &m.from.0,
                sender_email: &m.from.1,
                recipients: r,
                body: &m.body,
                html_body: m.html.as_deref(),
                message_id: &m.message_id,
                delivery_time: m.received,
                message_class: "IPM.Note",
                properties: p,
            },
            attachments: f,
        })
        .collect();
    let (first, rest) = match (*created, messages.split_first()) {
        (_, None) => return Ok(()),
        (true, _) => (None, messages.as_slice()),
        (false, Some((first, rest))) => (Some(first), rest),
    };
    if let Some(first) = first {
        UnicodePstFile::create_in_folder_with_attachments(path, folder, &first.message, first.attachments).map_err(|e| e.to_string())?;
        *created = true;
    }
    if !rest.is_empty() {
        append(path, folder, rest)?;
    }
    Ok(())
}

/// Add `batch` to the file at `path`. A file whose name-to-ID map can't take names (damaged, or
/// made by a program that keeps it some other way) gets the batch without its named properties:
/// the mail goes in, without its follow-up text, due days and categories, and the log says why.
fn append(path: &Path, folder: &[&str], batch: &[UnicodePstBatchMessage<'_>]) -> Result<(), String> {
    let Err(e) = UnicodePstFile::append_many_in_folder_with_attachments(path, folder, batch) else { return Ok(()) };
    let named = |p: &UnicodePstProperty<'_>| matches!(p.key, UnicodePstPropertyKey::Named(..));
    // The fork's writer says so before it writes any of the batch's messages (their folder may be
    // made by then, and stays).
    if !e.to_string().starts_with("name-to-ID map") || !batch.iter().any(|m| m.message.properties.iter().any(named)) {
        return Err(e.to_string());
    }
    log::info!("the data file's name-to-ID map can't be added to ({e}): mail written without its flags' text, due days and categories");
    let tagged: Vec<Vec<UnicodePstProperty<'_>>> =
        batch.iter().map(|m| m.message.properties.iter().filter(|p| !named(p)).copied().collect()).collect();
    let batch: Vec<UnicodePstBatchMessage<'_>> = batch
        .iter()
        .zip(&tagged)
        .map(|(m, p)| UnicodePstBatchMessage { message: UnicodePstMessage { properties: p, ..m.message }, attachments: m.attachments })
        .collect();
    UnicodePstFile::append_many_in_folder_with_attachments(path, folder, &batch).map(drop).map_err(|e| e.to_string())
}

/// A task's rule as the pattern an export writes for a task whose first day is `from`, and
/// whether it reads back as the rule (else the task's text keeps the rule). For the fuzz target.
#[doc(hidden)]
pub fn task_pattern(rule: &str, from: i64) -> Option<(Vec<u8>, bool)> {
    let p = pattern::write(rule, from)?;
    let says = pattern::says(&p, from, rule);
    Some((p, says))
}

/// A contact's and a task's text as the import reads it back, what an export kept there
/// (`pst_pim::rest`) put back in its place. For the fuzz target.
#[doc(hidden)]
pub fn read_back_text(body: &str) -> (mailcraft_model::vcard::Card, mailcraft_model::pim::Task) {
    let x = crate::pst_pim::Extra::default();
    (crate::pst_pim::card_of(&x, body, "c".into()), crate::pst_pim::task_of(&x, "Task", body, 0, "t".into()))
}
