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
//! `Message-ID`) aren't written twice. The file is written as a copy that replaces it at the end,
//! so a failed export leaves it as it was.

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

/// The `Message-ID`s in an existing data file's folders (a file the writer can add to: the Unicode
/// format, not Outlook's offline copy of a server account).
fn ids_in(path: &Path) -> Result<HashSet<String>, String> {
    let mut head = [0u8; 12];
    std::fs::File::open(path).and_then(|mut f| io::Read::read_exact(&mut f, &mut head)).map_err(|e| e.to_string())?;
    if !head.starts_with(b"!BDN") || head.get(8..10) != Some(b"SM") {
        return Err("it isn't an Outlook data file: choose another name to make a new one".into());
    }
    let read = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> io::Result<HashSet<String>> {
        let store = UnicodeStore::read(Rc::new(outlook_pst::UnicodePstFile::open(path)?))?;
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
        Ok(Err(_)) | Err(_) => Err("mail can only be added to a data file in Outlook's current format: choose another name to make a new one".into()),
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
            let _ = std::fs::remove_file(path);
            std::fs::rename(&temp, path).map_err(|e| e.to_string())?;
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
                    vec![mime::OutAttachment { name: "notes.txt".into(), mime: "text/plain".into(), data: b"remember".to_vec() }]
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

        // Nothing to write: an error, and the old file stays.
        let empty = dir.join("Empty.pst");
        assert!(export(&store, "nobody", "x", &empty, &mut |_| {}).is_err());
        assert!(!empty.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
