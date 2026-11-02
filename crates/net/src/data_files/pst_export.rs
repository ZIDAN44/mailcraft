//! Exporting an account's mail to an Outlook data file (`.pst`).
//!
//! The file is written with the MIT-licensed `outlook-pst-rw` crate (MS-PST, Unicode format):
//! each folder kept on this computer becomes a folder of the same name, and each message whose
//! body is here goes in with its addresses, text and HTML bodies and attachments. Messages only
//! known by their headers (never opened, on an account that keeps headers only) are counted as
//! skipped. The writer makes mail items only, so the account's calendar and contacts go beside
//! the file in the standard formats every calendar and address book reads (`… Calendar.ics`,
//! `… Contacts.vcf`).
//!
//! A data file that's already there is added to, as Outlook's export does (issue #18): its mail
//! stays, the account's goes into folders of the same names, and messages already in it (by
//! `Message-ID`) aren't written twice. That is a file in the Unicode format Outlook 2003 and later
//! make, encoded as Outlook encodes them (the writer is MailCraft's fork, which adds to those) or
//! not; Outlook 97–2002's ANSI format can't be added to. The file is written as a copy that
//! replaces it at the end, so a failed export leaves it as it was.

use std::collections::HashSet;
use std::io;
use std::path::Path;
use std::rc::Rc;

use mailcraft_mime as mime;
use mailcraft_model::{FolderRole, MessageKey, utf7};
use mailcraft_store::Store;
use outlook_pst::messaging::message::{Message as _, UnicodeMessage};
use outlook_pst::messaging::store::{Store as PstStore, UnicodeStore};
use outlook_pst::ndb::node_id::NodeId;
use pst_rw::{UnicodePstAttachment, UnicodePstBatchMessage, UnicodePstFile, UnicodePstMessage, UnicodePstRecipient, UnicodePstRecipientType};

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
    /// Events and people written beside the data file.
    pub events: usize,
    pub contacts: usize,
}

/// A message, ready to write.
struct Owned {
    subject: String,
    from: (String, String),
    recipients: Vec<(String, String, UnicodePstRecipientType)>,
    body: String,
    html: Option<String>,
    message_id: String,
    time: i64,
    files: Vec<(String, String, Option<String>, Vec<u8>)>,
}

fn owned(raw: &[u8]) -> Option<Owned> {
    let p = mime::parse(raw)?;
    let mut recipients = Vec::new();
    for (list, kind) in [(&p.to, UnicodePstRecipientType::To), (&p.cc, UnicodePstRecipientType::Cc)] {
        recipients.extend(list.iter().filter(|a| !a.email.is_empty()).map(|a| (a.name.clone(), a.email.clone(), kind)));
    }
    let files = p
        .attachments
        .iter()
        .filter(|a| a.size <= MAX_ATTACHMENT)
        .filter_map(|a| Some((a.name.clone(), a.mime.clone(), a.content_id.clone(), mime::attachment_data(raw, a.index)?)))
        .collect();
    Some(Owned {
        subject: p.subject,
        from: (p.from.name, p.from.email),
        recipients,
        body: p.text,
        html: mime::html_body(raw),
        message_id: if p.message_id.is_empty() { String::new() } else { format!("<{}>", p.message_id) },
        // Unix seconds → 100 ns since 1601.
        time: p.date.saturating_add(11_644_473_600).saturating_mul(10_000_000).max(0),
        files,
    })
}

/// A folder's path as names, the way Outlook shows them.
fn names(path: &str, delimiter: Option<char>, role: FolderRole) -> Vec<String> {
    let parts: Vec<&str> = match delimiter {
        Some(d) => path.split(d).collect(),
        None => vec![path],
    };
    let mut out: Vec<String> = parts.iter().map(|p| utf7::decode(p).trim().to_string()).collect();
    if let Some(first) = out.first_mut()
        && (role == FolderRole::Inbox || first.eq_ignore_ascii_case("INBOX"))
    {
        *first = "Inbox".into();
    }
    for n in &mut out {
        if n.is_empty() {
            *n = "Folder".into();
        }
    }
    out
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
/// there is added to. Calls `progress` with the number of messages written so far.
pub fn export(store: &Store, account: &str, name: &str, path: &Path, progress: &mut dyn FnMut(usize)) -> Result<Exported, String> {
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
    let written = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<(), String> {
        for f in folders.iter().filter(|f| f.selectable) {
            let Ok(st) = store.load_folder(account, &f.path) else { continue };
            let folder = names(&f.path, f.delimiter, f.role);
            let folder: Vec<&str> = folder.iter().map(String::as_str).collect();
            let mut batch = Vec::new();
            let mut bytes = 0usize;
            for s in &st.messages {
                let key = MessageKey { account: account.to_string(), folder: f.path.clone(), validity: s.key.validity, uid: s.key.uid };
                match store.read_body(&key).and_then(|raw| owned(&raw)) {
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
            let (events, contacts) = pim_beside(store, account, path).map_err(|e| e.to_string())?;
            Ok(Exported { events, contacts, ..done })
        }
        Err(e) => {
            let _ = std::fs::remove_file(&temp);
            Err(e)
        }
    }
}

/// The account's calendar and contacts beside the data file: (events, people) written.
fn pim_beside(store: &Store, account: &str, path: &Path) -> mailcraft_store::Result<(usize, usize)> {
    let pim: mailcraft_model::pim::Pim = store.load_account_doc(account, mailcraft_model::pim::PIM_DOC).unwrap_or_default();
    let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "Mail".into());
    let dir = path.parent().unwrap_or(Path::new("."));
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| i64::try_from(d.as_secs()).unwrap_or(0)).unwrap_or(0);
    let events: Vec<mailcraft_model::ical::CalEvent> = pim.events.iter().map(|e| e.event.clone()).collect();
    if !events.is_empty() {
        mailcraft_store::write_atomic(&dir.join(format!("{stem} Calendar.ics")), mailcraft_model::ical::write(&events, None, now).as_bytes())?;
    }
    if !pim.cards.is_empty() {
        let cards: String = pim.cards.iter().map(|c| mailcraft_model::vcard::write(&c.card)).collect();
        mailcraft_store::write_atomic(&dir.join(format!("{stem} Contacts.vcf")), cards.as_bytes())?;
    }
    Ok((events.len(), pim.cards.len()))
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
                .map(|(filename, mime_type, cid, data)| UnicodePstAttachment { filename, mime_type, content_id: cid.as_deref(), data })
                .collect()
        })
        .collect();
    let messages: Vec<UnicodePstBatchMessage<'_>> = batch
        .iter()
        .zip(&recipients)
        .zip(&files)
        .map(|((m, r), f)| UnicodePstBatchMessage {
            message: UnicodePstMessage {
                subject: &m.subject,
                sender_name: &m.from.0,
                sender_email: &m.from.1,
                recipients: r,
                body: &m.body,
                html_body: m.html.as_deref(),
                message_id: &m.message_id,
                delivery_time: m.time,
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
        UnicodePstFile::append_many_in_folder_with_attachments(path, folder, rest).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mailcraft_model::{EmailAddress, Flags};
    use mailcraft_store::FolderState;

    #[test]
    fn folder_names() {
        assert_eq!(names("INBOX/Projects/&AMk-t&AOk-", Some('/'), FolderRole::Other), ["Inbox", "Projects", "Été"]);
        assert_eq!(names("Sent", None, FolderRole::Sent), ["Sent"]);
        assert_eq!(names("a//b", Some('/'), FolderRole::Other), ["a", "Folder", "b"]);
    }

    #[test]
    fn an_account_round_trips_through_a_data_file() {
        let dir = std::env::temp_dir().join(format!("mailcraft-pst-export-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = Store::open(dir.join("cache")).unwrap();
        let mk = |folder: &str, uid: u32, subject: &str, with_file: bool| {
            let d = mime::Draft {
                to: vec![EmailAddress::new("Ana", "ana@example.com")],
                cc: vec![EmailAddress::new("", "ben@example.com")],
                subject: subject.into(),
                body: "Hello there.".into(),
                html: Some("<p>Hello <b>there</b>.</p>".into()),
                attachments: if with_file {
                    vec![mime::OutAttachment {
                        name: "notes.txt".into(),
                        mime: "text/plain".into(),
                        data: b"remember".to_vec(),
                        ..Default::default()
                    }]
                } else {
                    Vec::new()
                },
                ..Default::default()
            };
            let raw = mime::build(&d, &EmailAddress::new("Riley", "riley@example.com"), 1_704_067_200, &format!("m{uid}@example.com")).unwrap();
            let key = MessageKey { account: "a1".into(), folder: folder.into(), validity: 1, uid };
            store.write_body(&key, &raw).unwrap();
            mime::summary_from_full(&raw, key, Flags::default(), 10, 0)
        };
        let folder = |path: &str, role| mailcraft_model::Folder {
            account: "a1".into(),
            path: path.into(),
            delimiter: Some('/'),
            role,
            selectable: true,
            ..Default::default()
        };
        let inbox = FolderState {
            validity: 1,
            uid_next: 3,
            messages: vec![mk("INBOX", 1, "Plan — café", true), mk("INBOX", 2, "Second", false)],
            ..Default::default()
        };
        let mut projects = FolderState { validity: 1, uid_next: 2, messages: vec![mk("INBOX/Projects", 1, "Nested", false)], ..Default::default() };
        // Known by its header only: skipped.
        let mut header_only = projects.messages[0].clone();
        header_only.key.uid = 7;
        projects.messages.push(header_only);
        store.save_folder("a1", "INBOX", &inbox).unwrap();
        store.save_folder("a1", "INBOX/Projects", &projects).unwrap();
        store.save_folders("a1", &[folder("INBOX", FolderRole::Inbox), folder("INBOX/Projects", FolderRole::Other)]).unwrap();

        let mut pim = mailcraft_model::pim::Pim::default();
        pim.events.push(mailcraft_model::pim::StoredEvent {
            event: mailcraft_model::ical::CalEvent {
                uid: "e1".into(),
                summary: "Screening".into(),
                start: 1_792_000_000,
                end: 1_792_003_600,
                ..Default::default()
            },
            ..Default::default()
        });
        pim.cards.push(mailcraft_model::pim::StoredCard {
            card: mailcraft_model::vcard::Card { uid: "c1".into(), name: "Ana Lima".into(), ..Default::default() },
            ..Default::default()
        });
        store.save_account_doc("a1", mailcraft_model::pim::PIM_DOC, &pim).unwrap();
        let file = dir.join("Out.pst");
        // Something there that isn't a data file: left alone, and said.
        std::fs::write(&file, b"an older file").unwrap();
        assert!(export(&store, "a1", "Riley's mail", &file, &mut |_| {}).is_err_and(|e| e.contains("isn't an Outlook data file")));
        assert_eq!(std::fs::read(&file).unwrap(), b"an older file");
        std::fs::remove_file(&file).unwrap();
        let mut ticks = 0;
        let done = export(&store, "a1", "Riley's mail", &file, &mut |_| ticks += 1).unwrap();
        assert_eq!(done, Exported { messages: 3, skipped: 1, already: 0, events: 1, contacts: 1 });
        let ics = std::fs::read_to_string(dir.join("Out Calendar.ics")).unwrap();
        assert!(ics.contains("SUMMARY:Screening") && ics.contains("BEGIN:VCALENDAR"), "{ics}");
        let vcf = std::fs::read_to_string(dir.join("Out Contacts.vcf")).unwrap();
        assert!(vcf.contains("FN:Ana Lima"), "{vcf}");
        assert!(ticks > 0);

        // Read back with the importer.
        let back = Store::open(dir.join("back")).unwrap();
        let imported = crate::pst::import(&file, &back, "b1", &mut |_| {}).unwrap();
        assert_eq!(imported.messages, 3, "{imported:?}");
        let all = back.load_folders("b1").unwrap();
        let inbox = back.load_folder("b1", "INBOX").unwrap();
        assert_eq!(inbox.messages.len(), 2, "{all:?}");
        let plan = inbox.messages.iter().find(|m| m.subject == "Plan — café").unwrap();
        let raw = back.read_body(&MessageKey { account: "b1".into(), folder: "INBOX".into(), validity: 1, uid: plan.key.uid }).unwrap();
        let p = mime::parse(&raw).unwrap();
        assert_eq!(p.from.email, "riley@example.com");
        assert_eq!(p.cc[0].email, "ben@example.com");
        assert_eq!(p.message_id, "m1@example.com");
        assert_eq!(p.date, 1_704_067_200);
        assert!(p.attachments.iter().any(|a| a.name == "notes.txt"));
        assert!(all.iter().any(|f| f.path.ends_with("Projects") && f.total == 1), "{all:?}");

        // Exported again into the same file (issue #18): added to, nothing twice; a new message goes
        // in beside the ones there.
        let again = export(&store, "a1", "Riley's mail", &file, &mut |_| {}).unwrap();
        assert_eq!((again.messages, again.already), (0, 3));
        let mut inbox = store.load_folder("a1", "INBOX").unwrap();
        inbox.messages.push(mk("INBOX", 3, "Third", false));
        store.save_folder("a1", "INBOX", &inbox).unwrap();
        let more = export(&store, "a1", "Riley's mail", &file, &mut |_| {}).unwrap();
        assert_eq!((more.messages, more.already), (1, 3));
        let back = Store::open(dir.join("back2")).unwrap();
        assert_eq!(crate::pst::import(&file, &back, "b2", &mut |_| {}).unwrap().messages, 4);
        let subjects: Vec<String> = back.load_folder("b2", "INBOX").unwrap().messages.iter().map(|m| m.subject.clone()).collect();
        assert_eq!(subjects.iter().filter(|s| *s == "Second").count(), 1, "{subjects:?}");
        assert!(subjects.contains(&"Third".to_string()), "{subjects:?}");
        assert!(!dir.join("Out.pst.part").exists());

        // A file that can't be replaced: the error says where the mail is, and it is there.
        let busy = dir.join("Busy.pst");
        std::fs::create_dir_all(busy.join("inside")).unwrap();
        let e = export(&store, "a1", "Riley's mail", &busy, &mut |_| {}).unwrap_err();
        assert!(e.contains("Busy.pst.part"), "{e}");
        let back = Store::open(dir.join("back3")).unwrap();
        assert_eq!(crate::pst::import(&dir.join("Busy.pst.part"), &back, "b3", &mut |_| {}).unwrap().messages, 4);

        // Nothing to write: an error, and the old file stays.
        let empty = dir.join("Empty.pst");
        assert!(export(&store, "nobody", "x", &empty, &mut |_| {}).is_err());
        assert!(!empty.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// MS-PST 5.1's permutation (`mpbbCrypt`'s first 256 bytes), which encodes a data block's bytes.
    const PERMUTE: [u8; 256] = [
        0x41, 0x36, 0x13, 0x62, 0xA8, 0x21, 0x6E, 0xBB, 0xF4, 0x16, 0xCC, 0x04, 0x7F, 0x64, 0xE8, 0x5D, 0x1E, 0xF2, 0xCB, 0x2A, 0x74, 0xC5, 0x5E,
        0x35, 0xD2, 0x95, 0x47, 0x9E, 0x96, 0x2D, 0x9A, 0x88, 0x4C, 0x7D, 0x84, 0x3F, 0xDB, 0xAC, 0x31, 0xB6, 0x48, 0x5F, 0xF6, 0xC4, 0xD8, 0x39,
        0x8B, 0xE7, 0x23, 0x3B, 0x38, 0x8E, 0xC8, 0xC1, 0xDF, 0x25, 0xB1, 0x20, 0xA5, 0x46, 0x60, 0x4E, 0x9C, 0xFB, 0xAA, 0xD3, 0x56, 0x51, 0x45,
        0x7C, 0x55, 0x00, 0x07, 0xC9, 0x2B, 0x9D, 0x85, 0x9B, 0x09, 0xA0, 0x8F, 0xAD, 0xB3, 0x0F, 0x63, 0xAB, 0x89, 0x4B, 0xD7, 0xA7, 0x15, 0x5A,
        0x71, 0x66, 0x42, 0xBF, 0x26, 0x4A, 0x6B, 0x98, 0xFA, 0xEA, 0x77, 0x53, 0xB2, 0x70, 0x05, 0x2C, 0xFD, 0x59, 0x3A, 0x86, 0x7E, 0xCE, 0x06,
        0xEB, 0x82, 0x78, 0x57, 0xC7, 0x8D, 0x43, 0xAF, 0xB4, 0x1C, 0xD4, 0x5B, 0xCD, 0xE2, 0xE9, 0x27, 0x4F, 0xC3, 0x08, 0x72, 0x80, 0xCF, 0xB0,
        0xEF, 0xF5, 0x28, 0x6D, 0xBE, 0x30, 0x4D, 0x34, 0x92, 0xD5, 0x0E, 0x3C, 0x22, 0x32, 0xE5, 0xE4, 0xF9, 0x9F, 0xC2, 0xD1, 0x0A, 0x81, 0x12,
        0xE1, 0xEE, 0x91, 0x83, 0x76, 0xE3, 0x97, 0xE6, 0x61, 0x8A, 0x17, 0x79, 0xA4, 0xB7, 0xDC, 0x90, 0x7A, 0x5C, 0x8C, 0x02, 0xA6, 0xCA, 0x69,
        0xDE, 0x50, 0x1A, 0x11, 0x93, 0xB9, 0x52, 0x87, 0x58, 0xFC, 0xED, 0x1D, 0x37, 0x49, 0x1B, 0x6A, 0xE0, 0x29, 0x33, 0x99, 0xBD, 0x6C, 0xD9,
        0x94, 0xF3, 0x40, 0x54, 0x6F, 0xF0, 0xC6, 0x73, 0xB8, 0xD6, 0x3E, 0x65, 0x18, 0x44, 0x1F, 0xDD, 0x67, 0x10, 0xF1, 0x0C, 0x19, 0xEC, 0xAE,
        0x03, 0xA1, 0x14, 0x7B, 0xA9, 0x0B, 0xFF, 0xF8, 0xA3, 0xC0, 0xA2, 0x01, 0xF7, 0x2E, 0xBC, 0x24, 0x68, 0x75, 0x0D, 0xFE, 0xBA, 0x2F, 0xB5,
        0xD0, 0xDA, 0x3D,
    ];

    /// MS-PST's CRC: CRC-32's table, started at 0 and not inverted at the end.
    fn crc(data: &[u8]) -> u32 {
        let mut c = 0u32;
        for &b in data {
            c ^= u32::from(b);
            for _ in 0..8 {
                c = if c & 1 == 1 { (c >> 1) ^ 0xEDB8_8320 } else { c >> 1 };
            }
        }
        c
    }

    /// Encode a data file the writer made the way Outlook encodes its own by default: every data
    /// block's bytes permuted (internal blocks never are), found from the 16-byte trailers that end
    /// blocks on 64-byte boundaries, with their CRCs, and the header's `bCryptMethod` (0x201) set
    /// to permute, with the CRC over the header (0x20C, of the 516 bytes from 8).
    fn permute_encode(path: &Path) {
        let mut f = std::fs::read(path).unwrap();
        let u32_at = |f: &[u8], at: usize| u32::from_le_bytes(f[at..at + 4].try_into().unwrap());
        for end in (64..=f.len()).step_by(64) {
            let size = usize::from(u16::from_le_bytes([f[end - 16], f[end - 15]]));
            let bid = u64::from_le_bytes(f[end - 8..end].try_into().unwrap());
            let Some(start) = end.checked_sub((size + 16).div_ceil(64) * 64) else { continue };
            if size == 0 || size > 8176 || bid & 2 != 0 || crc(&f[start..start + size]) != u32_at(&f, end - 12) {
                continue;
            }
            for b in &mut f[start..start + size] {
                *b = PERMUTE[usize::from(*b)];
            }
            let c = crc(&f[start..start + size]);
            f[end - 12..end - 8].copy_from_slice(&c.to_le_bytes());
        }
        f[0x201] = 1;
        let c = crc(&f[8..8 + 516]);
        f[0x20C..0x210].copy_from_slice(&c.to_le_bytes());
        std::fs::write(path, f).unwrap();
    }

    /// A data file Outlook made, as Outlook encodes them, is added to (the writer from MailCraft's
    /// fork): what it had stays, a message it already has isn't written twice, it stays encoded,
    /// and the import reads it all back.
    #[test]
    fn an_export_adds_to_an_encoded_data_file() {
        let dir = std::env::temp_dir().join(format!("mailcraft-pst-export-encoded-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("Outlook.pst");
        let theirs = |subject, message_id| UnicodePstMessage {
            subject,
            sender_name: "Ana",
            sender_email: "ana@example.com",
            recipients: &[],
            body: "Kept as it was.",
            html_body: None,
            message_id,
            delivery_time: 133_485_408_000_000_000,
        };
        drop(UnicodePstFile::create_in_folder(&file, &["Inbox"], &theirs("Theirs", "<theirs@example.com>")).unwrap());
        drop(UnicodePstFile::append_in_folder(&file, &["Inbox"], &theirs("Both", "<m1@example.com>")).unwrap());
        permute_encode(&file);

        let store = Store::open(dir.join("cache")).unwrap();
        let mk = |uid: u32, subject: &str| {
            let d = mime::Draft { subject: subject.into(), body: "Hello there.".into(), ..Default::default() };
            let raw = mime::build(&d, &EmailAddress::new("Riley", "riley@example.com"), 1_704_067_200, &format!("m{uid}@example.com")).unwrap();
            let key = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid };
            store.write_body(&key, &raw).unwrap();
            mime::summary_from_full(&raw, key, Flags::default(), 10, 0)
        };
        let inbox = FolderState { validity: 1, uid_next: 3, messages: vec![mk(1, "Both"), mk(2, "Ours")], ..Default::default() };
        store.save_folder("a1", "INBOX", &inbox).unwrap();
        let folder =
            mailcraft_model::Folder { account: "a1".into(), path: "INBOX".into(), role: FolderRole::Inbox, selectable: true, ..Default::default() };
        store.save_folders("a1", &[folder]).unwrap();

        let done = export(&store, "a1", "Riley's mail", &file, &mut |_| {}).unwrap();
        assert_eq!((done.messages, done.already), (1, 1));
        assert_eq!(std::fs::read(&file).unwrap()[0x201], 1, "still permute-encoded");
        let back = Store::open(dir.join("back")).unwrap();
        assert_eq!(crate::pst::import(&file, &back, "b1", &mut |_| {}).unwrap().messages, 3);
        let mut subjects: Vec<String> = back.load_folder("b1", "INBOX").unwrap().messages.iter().map(|m| m.subject.clone()).collect();
        subjects.sort();
        assert_eq!(subjects, ["Both", "Ours", "Theirs"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Outlook 97–2002's ANSI files say so, before anything is read or written.
    #[test]
    fn an_ansi_data_file_says_which_format_it_is() {
        let dir = std::env::temp_dir().join(format!("mailcraft-pst-export-ansi-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("Old.pst");
        let mut head = b"!BDN\0\0\0\0SM".to_vec();
        head.extend_from_slice(&14u16.to_le_bytes());
        head.resize(1024, 0);
        std::fs::write(&file, &head).unwrap();
        assert!(ids_in(&file).is_err_and(|e| e.contains("Outlook 97–2002's format")));
        assert_eq!(std::fs::read(&file).unwrap(), head);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
