//! Exporting an account's mail to an Outlook data file (`.pst`).
//!
//! The file is written with MailCraft's own `.pst` crate (`mailcraft-pst`; MS-PST, Unicode format):
//! each folder kept on this computer becomes a folder of the same name, and
//! the account's Sent, Trash, Drafts and Junk folders go to the file's own Sent Items, Deleted
//! Items, Drafts and Junk Email ([`place`]). Each message whose body is here goes in with its
//! addresses (Bcc too), text and HTML bodies, attachments and header block, and with what the store
//! keeps of it: read or not, a draft, flagged (and when it's due), answered or forwarded, its
//! categories, importance and sensitivity ([`item`]); a meeting's request, cancellation or answer
//! as Outlook's meeting mail, with the meeting's own properties ([`meeting`]); signed and encrypted
//! mail and reports as Outlook keeps them ([`kinds`]); an attached message as the message it is,
//! and files of any size, a big one kept in a file of its own until its batch is written
//! ([`files`]). Messages only known by their headers (never opened, on an account that keeps
//! headers only) are counted as skipped. The account's calendar, contacts, contact groups, notes
//! and tasks go in as Outlook's own items, in the file's Calendar, Contacts, Notes and Tasks
//! folders ([`pim`], [`calendar`]): nothing stays beside the file.
//!
//! A data file that's already there is added to, as Outlook's export does (issue #18): its mail
//! stays, the account's goes into folders of the same names, and messages already in it (by
//! `Message-ID`) aren't written twice. That is a file in the Unicode format Outlook 2003 and later
//! make or in Outlook 97–2002's ANSI one ([`format`]: its strings 8-bit, in Windows-1252 or, for a
//! message whose text it can't hold, UTF-8), encoded as Outlook encodes them or not. The file is
//! written as a copy that replaces it at the end, so a failed export leaves it as it was.

use std::collections::HashSet;
use std::io;
use std::path::Path;
use std::rc::Rc;

use mailcraft_mime as mime;
use mailcraft_model::organize::FollowUpDays;
use mailcraft_model::{Folder, FolderRole, MessageKey, utf7};
use mailcraft_pst::{
    Format, NodeId,
    write::{self, BatchMessage, NewAttachment, NewMessage, NewProperty, NewRecipient, PropertyKey, RecipientType},
};
use mailcraft_store::Store;

mod calendar;
mod contact;
mod files;
mod format;
mod item;
mod kinds;
mod meeting;
mod pattern;
mod pim;
mod place;
mod task;
#[cfg(test)]
mod tests;
mod writing;

use format::{format_of, store_of};
use item::Owned;
use writing::{append, write};

/// Messages written per commit to the file, at most: the writer rewrites the folder's table of
/// contents with each, so a big folder written a hundred at a time took longer with every batch.
const BATCH: usize = 2000;
/// …and the bytes they hold (their text and files; not the big files waiting in files of their
/// own until they are written, [`files::HELD_UP_TO`]), at most…
const BATCH_BYTES: usize = 48 << 20;
/// …and the bytes of those big files, at most: the room they take beside the data file.
const SPILL_BYTES: u64 = 2 << 30;
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
    /// The account's events written into it as appointments (a series is one)…
    pub events: usize,
    /// …of which occurrences of a series whose rule Outlook's pattern can't say, each written as
    /// an appointment of its own (MS-OXOCAL has no place for the rule).
    pub occurrences: usize,
    /// The account's tasks, notes and people (contact groups among them) written into it…
    pub tasks: usize,
    pub notes: usize,
    pub contacts: usize,
    /// …and those a file added to had already, as they are now (an export marks each it writes).
    pub pim_already: usize,
    /// …and those changed since a file added to had them, written in place of their earlier
    /// copies (among those written).
    pub pim_replaced: usize,
}

/// The `Message-ID`s in an existing data file's folders (a file the writer can add to: the Unicode
/// format or Outlook 97–2002's ANSI one, encoded or not, not Outlook's offline copy of a server
/// account).
fn ids_in(path: &Path) -> Result<HashSet<String>, String> {
    let mut head = [0u8; 12];
    std::fs::File::open(path).and_then(|mut f| io::Read::read_exact(&mut f, &mut head)).map_err(|e| e.to_string())?;
    if !head.starts_with(b"!BDN") || head.get(8..10) != Some(b"SM") {
        return Err("it isn't an Outlook data file: choose another name to make a new one".into());
    }
    let read = || -> io::Result<HashSet<String>> {
        let any = store_of(path)?;
        let mut ids = HashSet::new();
        let top = any.open_folder(any.ipm_subtree()?)?;
        let mut stack = vec![(top, 0usize)];
        while let Some((folder, depth)) = stack.pop() {
            if let Some(contents) = folder.contents_table() {
                for row in contents.rows_matrix() {
                    let Ok(props) = any.message_properties(row.id(), &[0x1035]) else { continue };
                    if let Some(id) = props.get(0x1035).and_then(crate::pst::text_of) {
                        ids.insert(mailcraft_model::thread::norm_id(&id));
                    }
                }
            }
            if depth < MAX_DEPTH
                && let Some(table) = folder.hierarchy_table()
            {
                for row in table.rows_matrix() {
                    if let Ok(sub) = any.open_folder(row.id()) {
                        stack.push((sub, depth + 1));
                    }
                }
            }
        }
        Ok(ids)
    };
    match read() {
        Ok(ids) => Ok(ids),
        Err(e) if matches!(e.raw_os_error(), Some(32 | 33)) => {
            Err("Outlook (or another program) has this data file open: close Outlook, including from the notification area, and try again".into())
        }
        Err(_) => Err("this data file can't be read, so mail can't be added to it: choose another name to make a new one".into()),
    }
}

/// Write `account`'s mail to a data file at `path`, named `name` when it's new; a file already
/// there is added to. `follow_up`: when flagged messages are due and completed ones were done
/// (local days, by [`mailcraft_model::Summary::follow_up_id`]). Calls `progress` with the number of messages
/// written so far.
pub fn export(
    store: &Store,
    account: &str,
    name: &str,
    path: &Path,
    follow_up: &FollowUpDays,
    progress: &mut dyn FnMut(usize),
) -> Result<Exported, String> {
    let folders = store.load_folders(account).map_err(|e| e.to_string())?;
    let mut done = Exported::default();
    let temp = path.with_extension("pst.part");
    let _ = std::fs::remove_file(&temp);
    let spill = files::Spill::new(path.with_extension("pst.part-files"));
    // Added to: a copy of it, with the messages it has already left out.
    let existing = path.is_file();
    let had = if existing { ids_in(path)? } else { HashSet::new() };
    if existing {
        std::fs::copy(path, &temp).map_err(|e| e.to_string())?;
    }
    let mut created = existing;
    // A file added to keeps its format; a new one is Unicode.
    let format = if existing { format_of(&temp) } else { Format::Unicode };
    // The special folders a file added to records, else the names a new one gets.
    let targets = place::Targets::of(existing.then_some(temp.as_path()));
    let pim: mailcraft_model::pim::Pim = store.load_account_doc(account, mailcraft_model::pim::PIM_DOC).unwrap_or_default();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| i64::try_from(d.as_secs()).unwrap_or(0)).unwrap_or(0);
    let owner = owner(store, account, &pim);
    // Where the mail went, so the contacts, notes and tasks of a new file don't go there.
    let mut taken: Vec<Vec<String>> = Vec::new();
    let mut inside = pim::Written::default();
    // Where each folder goes; in an ANSI file, folders whose names it would write alike apart.
    let placed: Vec<Vec<String>> = folders.iter().map(|f| place::place(f, &folders, &targets)).collect();
    let placed = if format == Format::Ansi { place::apart(&placed, &targets) } else { placed };
    let mut write_all = || -> Result<(), String> {
        for (f, folder) in folders.iter().zip(&placed).filter(|(f, _)| f.selectable) {
            let Ok(st) = store.load_folder(account, &f.path) else { continue };
            let folder = folder.clone();
            taken.push(folder.clone());
            let folder: Vec<&str> = folder.iter().map(String::as_str).collect();
            let mut batch = Vec::new();
            let mut bytes = 0usize;
            for s in &st.messages {
                let key = MessageKey { account: account.to_string(), folder: f.path.clone(), validity: s.key.validity, uid: s.key.uid };
                let state = item::state(s, f.role, follow_up);
                match store.read_body(&key).and_then(|raw| item::owned(&raw, s.received, state, &owner, Some(&spill))) {
                    Some(m) if !m.message_id.is_empty() && had.contains(&mailcraft_model::thread::norm_id(&m.message_id)) => done.already += 1,
                    Some(m) => {
                        bytes = bytes.saturating_add(m.held());
                        batch.push(m);
                    }
                    None => done.skipped += 1,
                }
                if batch.len() >= BATCH || bytes >= BATCH_BYTES || spill.bytes() >= SPILL_BYTES {
                    let n = batch.len();
                    write(&temp, &folder, &std::mem::take(&mut batch), &mut created)?;
                    spill.clear();
                    bytes = 0;
                    done.messages += n;
                    progress(done.messages);
                }
            }
            if !batch.is_empty() {
                write(&temp, &folder, &batch, &mut created)?;
                spill.clear();
                done.messages += batch.len();
                progress(done.messages);
            }
        }
        record_special(&temp, &folders, &targets, created);
        inside = pim::write(&temp, &pim, &taken, existing, &mut created, now, &owner, account)?;
        Ok(())
    };
    // Stored mail is parsed and calendars built in there: whatever fails, the export ends and says so.
    let written = std::panic::catch_unwind(std::panic::AssertUnwindSafe(&mut write_all));
    let result = match written {
        Ok(Ok(())) if !created => Err("there is no mail on this computer to export".to_string()),
        // A file added to keeps its own name.
        Ok(Ok(())) if existing => Ok(()),
        Ok(Ok(())) => write::set_store_display_name(&temp, name).map_err(|e| e.to_string()),
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
            if inside.occurrences > 0 {
                log::info!("{} occurrences of series whose rule Outlook can't keep went in one by one", inside.occurrences);
            }
            Ok(Exported {
                events: inside.events,
                occurrences: inside.occurrences,
                tasks: inside.tasks,
                notes: inside.notes,
                contacts: inside.contacts,
                pim_already: inside.already,
                pim_replaced: inside.replaced,
                ..done
            })
        }
        Err(e) => {
            let _ = std::fs::remove_file(&temp);
            Err(e)
        }
    }
}

/// Whose mail and calendar `account`'s are: its addresses (the one it sends from, the one it
/// signs in as, the one its provider says it signed in as), the meetings its calendar keeps as a
/// guest, and this computer's zone.
fn owner(store: &Store, account: &str, pim: &mailcraft_model::pim::Pim) -> calendar::Owner {
    let config = store.load_accounts().unwrap_or_default().into_iter().find(|a| a.id == account);
    let me =
        config.map(|a| [a.email, a.username, a.oauth_user.unwrap_or_default()].into_iter().filter(|m| m.contains('@')).collect()).unwrap_or_default();
    let zone = iana_time_zone::get_timezone().ok().and_then(|z| mailcraft_model::zone::Zone::named(&z)).unwrap_or_default();
    calendar::Owner { me, guest: pim.guest_copies.clone(), zone }
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
        match write::special_folder(path, special) {
            Ok(Some(_)) => continue,
            Ok(None) => {}
            Err(e) => {
                log::info!("the data file's {special:?} can't be read: {e}");
                continue;
            }
        }
        let names = targets.get(special);
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        if let Err(e) = write::set_special_folder(path, special, &names) {
            log::info!("the data file's {special:?} can't be recorded: {e}");
        }
    }
}

/// The class an export gives the message in `raw` (signed, encrypted, a report, meeting mail or
/// a note), its files and attached messages read as it reads them. For the fuzz target.
#[doc(hidden)]
pub fn message_class(raw: &[u8]) -> Option<&'static str> {
    item::owned(raw, 0, item::State::default(), &calendar::Owner::default(), None).map(|m| m.class())
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

/// The pattern an export writes for a repeating appointment starting at `start` (Unix seconds,
/// in the zone `tzid`, an hour long) with `rule`, each of `moves` a changed occurrence: (which
/// occurrence, how far it moved in seconds, its title); `None` when the pattern can't say the
/// series. For the fuzz target, whose reader reads it back.
#[doc(hidden)]
pub fn appointment_pattern(rule: &str, start: i64, tzid: &str, moves: &[(usize, i64, &str)]) -> Option<Vec<u8>> {
    use mailcraft_model::ical::{self, CalEvent};
    let e = CalEvent { uid: "u".into(), start, end: start.saturating_add(3600), tzid: tzid.into(), rrule: Some(rule.into()), ..Default::default() };
    let all = ical::starts(&e, i64::MAX);
    let own: Vec<CalEvent> = moves
        .iter()
        .filter_map(|(i, by, title)| {
            let r = *all.get(i.checked_rem(all.len())?)?;
            let s = r.saturating_add(*by);
            Some(CalEvent { recurrence_id: Some(r), start: s, end: s.saturating_add(3600), summary: (*title).into(), rrule: None, ..e.clone() })
        })
        .collect();
    let own: Vec<&CalEvent> = own.iter().collect();
    calendar::series(&e, &own, &calendar::Owner::default(), false)?.into_iter().find_map(|(k, v)| match (k, v) {
        (pim::Key::Lid(_, 0x8216), pim::Value::Bin(b)) => Some(b),
        _ => None,
    })
}
