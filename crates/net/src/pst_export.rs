//! Exporting an account's mail to an Outlook data file (`.pst`).
//!
//! The file is written with the MIT-licensed `outlook-pst-rw` crate (MS-PST, Unicode format):
//! each folder kept on this computer becomes a folder of the same name, and each message whose
//! body is here goes in with its addresses, text and HTML bodies and attachments. Messages only
//! known by their headers (never opened, on an account that keeps headers only) are counted as
//! skipped. The writer makes mail items only, so the account's calendar and contacts go beside
//! the file in the standard formats every calendar and address book reads (`… Calendar.ics`,
//! `… Contacts.vcf`).

use std::path::Path;

use mailcraft_mime as mime;
use mailcraft_model::{FolderRole, MessageKey, utf7};
use mailcraft_store::Store;
use pst_rw::{UnicodePstAttachment, UnicodePstBatchMessage, UnicodePstFile, UnicodePstMessage, UnicodePstRecipient, UnicodePstRecipientType};

/// Messages written per commit to the file, at most: the writer rewrites the folder's table of
/// contents with each, so a big folder written a hundred at a time took longer with every batch.
const BATCH: usize = 2000;
/// …and their bytes (attachments and all), at most.
const BATCH_BYTES: usize = 48 << 20;
/// Attachments larger than this are left out.
const MAX_ATTACHMENT: usize = 100 * 1024 * 1024;

/// What an export did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Exported {
    pub messages: usize,
    /// Messages whose body isn't on this computer, or that couldn't be read.
    pub skipped: usize,
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

/// Write `account`'s mail to a new data file at `path` (replacing a file already there), named
/// `name` when opened. Calls `progress` with the number of messages written so far.
pub fn export(store: &Store, account: &str, name: &str, path: &Path, progress: &mut dyn FnMut(usize)) -> Result<Exported, String> {
    let folders = store.load_folders(account).map_err(|e| e.to_string())?;
    let mut done = Exported::default();
    let mut created = false;
    let temp = path.with_extension("pst.part");
    let _ = std::fs::remove_file(&temp);
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
            unread: 0,
            total: 0,
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
        std::fs::write(&file, b"an older file").unwrap();
        let mut ticks = 0;
        let done = export(&store, "a1", "Riley's mail", &file, &mut |_| ticks += 1).unwrap();
        assert_eq!(done, Exported { messages: 3, skipped: 1, events: 1, contacts: 1 });
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

        // Nothing to write: an error, and the old file stays.
        let empty = dir.join("Empty.pst");
        assert!(export(&store, "nobody", "x", &empty, &mut |_| {}).is_err());
        assert!(!empty.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
