//! Mail that is more than a note through a data file: attached messages, exported as Outlook
//! keeps them and read back by the import.

use super::*;
use outlook_pst::messaging::attachment::{Attachment as _, AttachmentData, UnicodeAttachment};

fn scratch(label: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("mailcraft-pst-export-kinds-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn draft(subject: &str, body: &str, attachments: Vec<mime::OutAttachment>) -> mime::Draft {
    mime::Draft {
        to: vec![EmailAddress::new("Ana", "ana@studio.example")],
        subject: subject.into(),
        body: body.into(),
        attachments,
        ..Default::default()
    }
}

fn build(d: &mime::Draft, id: &str) -> Vec<u8> {
    mime::build(d, &EmailAddress::new("Riley", "riley@studio.example"), 1_790_000_000, id).unwrap()
}

fn file(name: &str, mime: &str, data: &[u8]) -> mime::OutAttachment {
    mime::OutAttachment { name: name.into(), mime: mime.into(), data: data.to_vec(), ..Default::default() }
}

/// `raws` in an account's Inbox, exported to `dir/Mail.pst` and imported back into `dir/back`:
/// the file, and the cache it was read into.
fn round_trip(dir: &Path, raws: &[Vec<u8>]) -> (std::path::PathBuf, Store) {
    let store = Store::open(dir.join("cache")).unwrap();
    let mut messages = Vec::new();
    for (uid, raw) in (1..).zip(raws) {
        let key = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid };
        store.write_body(&key, raw).unwrap();
        messages.push(mime::summary_from_full(raw, key, Flags::default(), 10, 0).into());
    }
    let n = u32::try_from(messages.len()).unwrap();
    store.save_folder("a1", "INBOX", &FolderState { validity: 1, uid_next: n + 1, messages, ..Default::default() }).unwrap();
    let inbox =
        mailcraft_model::Folder { account: "a1".into(), path: "INBOX".into(), role: FolderRole::Inbox, selectable: true, ..Default::default() };
    store.save_folders("a1", &[inbox]).unwrap();
    let path = dir.join("Mail.pst");
    let done = export(&store, "a1", "Mail", &path, NONE, &mut |_| {}).unwrap();
    assert_eq!(done.messages, raws.len(), "{done:?}");
    let back = Store::open(dir.join("back")).unwrap();
    crate::pst::import(&path, &back, "b1", &mut |_| {}).unwrap();
    (path, back)
}

/// The source of the message with `subject` in the imported Inbox.
fn imported(back: &Store, subject: &str) -> Vec<u8> {
    let inbox = back.load_folder("b1", "INBOX").unwrap();
    let m = inbox.messages.iter().find(|m| m.subject == subject).unwrap_or_else(|| panic!("no {subject}"));
    back.read_body(&m.key).unwrap()
}

/// The message with `subject` in the file's Inbox, as the writer's reader reads it.
fn in_file(path: &Path, subject: &str) -> Rc<UnicodeMessage> {
    let store = UnicodeStore::read(Rc::new(outlook_pst::UnicodePstFile::open(path).unwrap())).unwrap();
    let any: Rc<dyn PstStore> = store.clone();
    let top = any.open_folder(&any.properties().ipm_sub_tree_entry_id().unwrap()).unwrap();
    let table = top.hierarchy_table().unwrap().clone();
    let inbox = table
        .rows_matrix()
        .filter_map(|row| any.open_folder(&any.properties().make_entry_id(NodeId::from(u32::from(row.id()))).ok()?).ok())
        .find(|f| f.properties().display_name().is_ok_and(|n| n == "Inbox"))
        .unwrap();
    for row in inbox.contents_table().unwrap().rows_matrix() {
        let entry = any.properties().make_entry_id(NodeId::from(u32::from(row.id()))).unwrap();
        let m = UnicodeMessage::read(store.clone(), &entry, None).unwrap();
        if m.properties().get(0x0037).and_then(crate::pst::text_of).as_deref() == Some(subject) {
            return m;
        }
    }
    panic!("no {subject} in the file")
}

fn attachments_of(m: &Rc<UnicodeMessage>) -> Vec<Rc<UnicodeAttachment>> {
    let Some(table) = m.attachment_table() else { return Vec::new() };
    table.rows_matrix().map(|row| UnicodeAttachment::read(m.clone(), NodeId::from(u32::from(row.id())), None).unwrap()).collect()
}

fn text(value: Option<&outlook_pst::ltp::prop_context::PropertyValue>) -> Option<String> {
    value.and_then(crate::pst::text_of)
}

/// A message attached three deep, a file at the bottom, goes in as attached messages (method 5,
/// no `.eml`), each with its own recipients and files, and comes back through the import as the
/// messages it was, down to the file.
#[test]
fn attached_messages_go_in_as_messages() {
    use outlook_pst::ltp::prop_context::PropertyValue as V;
    let dir = scratch("attached");
    let deep = build(&draft("Level 3", "The bottom.", vec![file("deep.txt", "text/plain", b"found it")]), "l3@studio.example");
    let two = build(&draft("Level 2", "Two.", vec![file("Level 3.eml", "message/rfc822", &deep)]), "l2@studio.example");
    let one = build(&draft("Level 1", "One.", vec![file("Level 2.eml", "message/rfc822", &two)]), "l1@studio.example");
    let long = build(&draft(&"Long ".repeat(1000), "Long.", Vec::new()), "long@studio.example");
    let top = build(
        &draft(
            "Forwarded",
            "See attached.",
            vec![file("Level 1.eml", "message/rfc822", &one), file("a.txt", "text/plain", b"beside"), file("long.eml", "message/rfc822", &long)],
        ),
        "top@studio.example",
    );
    let (path, back) = round_trip(&dir, &[top]);

    let mut m = in_file(&path, "Forwarded");
    let files = attachments_of(&m);
    assert_eq!(files.len(), 3);
    assert!(matches!(files[1].properties().get(0x3705), Some(V::Integer32(1))));
    // A subject too long for an attachment's name is cut short, not the export refused.
    assert!(matches!(files[2].properties().get(0x3705), Some(V::Integer32(5))));
    assert_eq!(text(files[2].properties().get(0x3001)).map(|n| n.chars().count()), Some(255));
    for level in ["Level 1", "Level 2", "Level 3"] {
        let attached = attachments_of(&m).into_iter().find(|a| matches!(a.properties().get(0x3705), Some(V::Integer32(5)))).unwrap();
        assert_eq!(text(attached.properties().get(0x3001)).as_deref(), Some(level));
        m = attached.embedded_message().unwrap();
        assert_eq!(text(m.properties().get(0x0037)).as_deref(), Some(level));
        assert_eq!(m.recipient_table().map(|t| t.rows_matrix().count()), Some(1));
    }
    let bottom = attachments_of(&m);
    assert_eq!(text(bottom[0].properties().get(0x3707)).as_deref(), Some("deep.txt"));
    assert!(matches!(bottom[0].data(), Some(AttachmentData::Binary(b)) if b.buffer() == b"found it"));

    // Back through the import: an attached message at each level, the file at the bottom.
    let mut raw = imported(&back, "Forwarded");
    for level in ["Level 1", "Level 2", "Level 3"] {
        let p = mime::parse(&raw).unwrap();
        let a = p.attachments.iter().find(|a| a.mime == "message/rfc822").unwrap_or_else(|| panic!("{level}: {:?}", p.attachments));
        raw = mime::attachment_data(&raw, a.index).unwrap();
        assert_eq!(mime::parse(&raw).unwrap().subject, level);
    }
    let p = mime::parse(&raw).unwrap();
    assert_eq!(p.attachments.len(), 1);
    assert_eq!(mime::attachment_data(&raw, p.attachments[0].index).as_deref(), Some(&b"found it"[..]));
    let _ = std::fs::remove_dir_all(&dir);
}
