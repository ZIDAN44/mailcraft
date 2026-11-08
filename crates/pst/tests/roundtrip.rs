//! Files written here and read back: new ones, added to, changed, in both formats.

use std::rc::Rc;

use mailcraft_pst::write::{self, *};
use mailcraft_pst::*;

fn temp(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("mailcraft-pst-tests-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    let _ = std::fs::remove_file(&path);
    path
}

fn open(path: &std::path::Path) -> Rc<PstStore> {
    PstStore::read(Rc::new(PstFile::open(path).unwrap())).unwrap()
}

fn text(v: Option<&PropertyValue>) -> String {
    match v {
        Some(PropertyValue::Unicode(s)) => String::from_utf16_lossy(s.buffer()).trim_end_matches('\0').to_string(),
        Some(PropertyValue::String8(s)) => s.buffer().iter().map(|b| char::from(*b)).collect::<String>().trim_end_matches('\0').to_string(),
        _ => String::new(),
    }
}

fn msg<'a>(subject: &'a str, recipients: &'a [NewRecipient<'a>], properties: &'a [NewProperty<'a>]) -> NewMessage<'a> {
    NewMessage {
        subject,
        sender_name: "Ann",
        sender_email: "ann@example.org",
        recipients,
        body: "The body.",
        html_body: None,
        message_id: "<one@example.org>",
        delivery_time: 133_000_000_000_000_000,
        message_class: "IPM.Note",
        properties,
    }
}

/// The child of `folder` named `name`.
fn child(store: &Rc<PstStore>, folder: u32, name: &str) -> Option<u32> {
    let f = store.open_folder(folder).unwrap();
    let t = f.hierarchy_table()?;
    let col = t.columns().iter().position(|c| c.prop_id() == 0x3001)?;
    t.rows_matrix().find(|r| text(t.value(r, col).as_ref()) == name).map(|r| r.id())
}

fn subjects(store: &Rc<PstStore>, folder: u32) -> Vec<String> {
    let f = store.open_folder(folder).unwrap();
    let Some(t) = f.contents_table() else { return vec![] };
    t.rows_matrix().map(|r| text(Message::read(store.clone(), r.id()).unwrap().properties().get(0x0037))).collect()
}

fn count(store: &Rc<PstStore>, folder: u32) -> (i32, i32) {
    let f = store.open_folder(folder).unwrap();
    let get = |id| match f.properties().get(id) {
        Some(PropertyValue::Integer32(n)) => *n,
        _ => -1,
    };
    (get(0x3602), get(0x3603))
}

#[test]
fn a_new_file_opens_with_its_folders() {
    for format in [Format::Unicode, Format::Ansi] {
        let path = temp(&format!("new-{format:?}.pst"));
        write::create_empty(&path, format).unwrap();
        let pst = PstFile::open(&path).unwrap();
        assert_eq!(pst.format(), format);
        let store = open(&path);
        let top = store.ipm_subtree().unwrap();
        assert!(child(&store, top, "Deleted Items").is_some(), "{format:?}");
        assert!(child(&store, top, "Inbox").is_some(), "{format:?}");
    }
}

#[test]
fn messages_are_added_and_read_back_with_all_they_hold() {
    for format in [Format::Unicode, Format::Ansi] {
        let path = temp(&format!("mail-{format:?}.pst"));
        write::create_empty(&path, format).unwrap();
        let people = [
            NewRecipient { name: "Bob", email: "bob@example.org", recipient_type: RecipientType::To, ..Default::default() },
            NewRecipient { name: "", email: "cc@example.org", recipient_type: RecipientType::Cc, flags: Some(3), track_status: Some(2) },
        ];
        let big = vec![b'x'; 20_000];
        let long_body = "word ".repeat(4000);
        let props = [
            NewProperty::tagged(0x0E07, NewValue::Integer32(0)),
            NewProperty::named(PS_PUBLIC_STRINGS, Name::String("Keywords"), NewValue::MultipleUnicode(&["red", "blue"])),
        ];
        let inner_files = [NewAttachment { filename: "deep.txt", mime_type: "text/plain", data: b"found it", ..Default::default() }];
        let attached = NewAttachment {
            filename: "A message",
            content: AttachmentContent::Message(Box::new(EmbeddedMessage {
                message: msg("Inner", &[], &[]),
                attachments: inner_files.into_iter().collect(),
            })),
            ..Default::default()
        };
        let files = vec![
            NewAttachment { filename: "a.bin", mime_type: "application/octet-stream", data: &big, ..Default::default() },
            NewAttachment { filename: "empty.txt", mime_type: "text/plain", ..Default::default() },
            attached,
        ];
        let mut m = msg("First", &people, &props);
        m.body = &long_body;
        let plain = msg("Second", &[], &[]);
        write::append(
            &path,
            &["Projects", "2026"],
            &[BatchMessage { message: m, attachments: &files }, BatchMessage { message: plain, attachments: &[] }],
        )
        .unwrap();

        let store = open(&path);
        let top = store.ipm_subtree().unwrap();
        let projects = child(&store, top, "Projects").unwrap();
        let year = child(&store, projects, "2026").unwrap();
        assert_eq!(subjects(&store, year), vec!["First", "Second"], "{format:?}");
        assert_eq!(count(&store, year), (2, 1), "{format:?}: one went in unread");

        let f = store.open_folder(year).unwrap();
        let contents = f.contents_table().unwrap();
        let row = contents.rows_matrix().next().unwrap();
        let first = Message::read(store.clone(), row.id()).unwrap();
        assert_eq!(text(first.properties().get(0x1000)).len(), long_body.len());
        let r = first.recipient_table().unwrap();
        assert_eq!(r.rows_matrix().count(), 2);
        let email = r.columns().iter().position(|c| c.prop_id() == 0x3003).unwrap();
        let emails: Vec<String> = r.rows_matrix().map(|row| text(r.value(row, email).as_ref())).collect();
        assert_eq!(emails, vec!["bob@example.org", "cc@example.org"]);

        let atts = first.attachment_table().unwrap();
        let ids: Vec<u32> = atts.rows_matrix().map(|r| r.id()).collect();
        assert_eq!(ids.len(), 3);
        let a = Attachment::read(first.clone(), ids[0]).unwrap();
        assert!(matches!(a.data(), Some(AttachmentData::Binary(b)) if b.buffer() == big.as_slice()));
        assert_eq!(text(a.properties().get(0x3704)), "a.bin");
        let a = Attachment::read(first.clone(), ids[1]).unwrap();
        assert!(matches!(a.data(), Some(AttachmentData::Binary(b)) if b.buffer().is_empty()));
        let a = Attachment::read(first.clone(), ids[2]).unwrap();
        let inner = a.embedded_message().unwrap();
        assert_eq!(text(inner.properties().get(0x0037)), "Inner");
        let deep_table = inner.attachment_table().unwrap();
        let deep = Attachment::read(inner.clone(), deep_table.rows_matrix().next().unwrap().id()).unwrap();
        assert!(matches!(deep.data(), Some(AttachmentData::Binary(b)) if b.buffer() == b"found it"));

        // The named property is found through the map.
        let map = store.named_property_map().unwrap();
        let entry = map
            .stream_entry()
            .into_iter()
            .find(|e| matches!(e.id(), NamedPropertyId::StringOffset(_)) && e.guid() == NamedPropertyGuid::PublicStrings)
            .unwrap();
        assert!(first.properties().get(entry.prop_id()).is_some(), "{format:?}");
    }
}

#[test]
fn adding_again_keeps_what_was_there_and_reuses_the_space_let_go_of() {
    let path = temp("again.pst");
    write::create_empty(&path, Format::Unicode).unwrap();
    for i in 0..30 {
        let subject = format!("Message {i}");
        write::append(&path, &["Inbox"], &[BatchMessage { message: msg(&subject, &[], &[]), attachments: &[] }]).unwrap();
    }
    let store = open(&path);
    let inbox = child(&store, store.ipm_subtree().unwrap(), "Inbox").unwrap();
    let all = subjects(&store, inbox);
    assert_eq!(all.len(), 30);
    assert_eq!(all[29], "Message 29");
    let size = std::fs::metadata(&path).unwrap().len();
    assert!(size < 2_000_000, "the file stays small: {size}");
}

#[test]
fn many_messages_fill_the_b_trees_past_one_page() {
    for format in [Format::Unicode, Format::Ansi] {
        let path = temp(&format!("many-{format:?}.pst"));
        write::create_empty(&path, format).unwrap();
        let names: Vec<String> = (0..600).map(|i| format!("Message {i}")).collect();
        let batch: Vec<BatchMessage<'_>> = names.iter().map(|s| BatchMessage { message: msg(s, &[], &[]), attachments: &[] }).collect();
        write::append(&path, &["Inbox"], &batch).unwrap();
        let store = open(&path);
        let inbox = child(&store, store.ipm_subtree().unwrap(), "Inbox").unwrap();
        assert_eq!(subjects(&store, inbox), names, "{format:?}");
        assert_eq!(count(&store, inbox), (600, 0));
    }
}

#[test]
fn messages_taken_out_are_gone_and_the_folder_counts_again() {
    let path = temp("delete.pst");
    write::create_empty(&path, Format::Unicode).unwrap();
    let names = ["One", "Two", "Three"];
    let batch: Vec<BatchMessage<'_>> = names.iter().map(|s| BatchMessage { message: msg(s, &[], &[]), attachments: &[] }).collect();
    write::append(&path, &["Inbox"], &batch).unwrap();
    let store = open(&path);
    let inbox = child(&store, store.ipm_subtree().unwrap(), "Inbox").unwrap();
    let f = store.open_folder(inbox).unwrap();
    let ids: Vec<u32> = f.contents_table().unwrap().rows_matrix().map(|r| r.id()).collect();
    drop((f, store));
    write::delete_messages(&path, &[ids[1]]).unwrap();
    let store = open(&path);
    assert_eq!(subjects(&store, inbox), vec!["One", "Three"]);
    assert_eq!(count(&store, inbox), (2, 0));
    assert!(Message::read(store.clone(), ids[1]).is_err());
    assert!(write::delete_messages(&path, &[ids[1]]).is_err(), "gone already");
    assert!(write::delete_messages(&path, &[0x7777]).is_err(), "not a message");
}

#[test]
fn folders_get_their_class_and_special_folders_are_recorded() {
    let path = temp("special.pst");
    write::create_empty(&path, Format::Unicode).unwrap();
    write::create_folder(&path, &["Tasks"], "IPF.Task").unwrap();
    write::set_special_folder(&path, SpecialFolder::Tasks, &["Tasks"]).unwrap();
    write::set_special_folder(&path, SpecialFolder::JunkEmail, &["Junk Email"]).unwrap();
    write::set_special_folder(&path, SpecialFolder::SentItems, &["Sent Items"]).unwrap();
    assert_eq!(write::special_folder(&path, SpecialFolder::Tasks).unwrap(), Some(vec!["Tasks".to_string()]));
    assert_eq!(write::special_folder(&path, SpecialFolder::JunkEmail).unwrap(), Some(vec!["Junk Email".to_string()]));
    assert_eq!(write::special_folder(&path, SpecialFolder::SentItems).unwrap(), Some(vec!["Sent Items".to_string()]));
    assert_eq!(write::special_folder(&path, SpecialFolder::Drafts).unwrap(), None);
    let store = open(&path);
    let t = child(&store, store.ipm_subtree().unwrap(), "Tasks").unwrap();
    assert_eq!(text(store.open_folder(t).unwrap().properties().get(0x3613)), "IPF.Task");
}

#[test]
fn the_store_can_be_renamed() {
    let path = temp("rename.pst");
    write::create_empty(&path, Format::Unicode).unwrap();
    write::set_store_display_name(&path, "Archive 2026").unwrap();
    assert_eq!(text(open(&path).properties().get(0x3001)), "Archive 2026");
}

#[test]
fn what_can_t_be_written_is_refused_before_the_file_is_touched() {
    let path = temp("refused.pst");
    write::create_empty(&path, Format::Unicode).unwrap();
    let before = std::fs::read(&path).unwrap();
    let mut no_class = msg("x", &[], &[]);
    no_class.message_class = "";
    assert!(write::append(&path, &["Inbox"], &[BatchMessage { message: no_class, attachments: &[] }]).is_err());
    let twice = [NewProperty::tagged(0x0E2B, NewValue::Integer32(1)), NewProperty::tagged(0x0E2B, NewValue::Integer32(2))];
    assert!(write::append(&path, &["Inbox"], &[BatchMessage { message: msg("x", &[], &twice), attachments: &[] }]).is_err());
    assert!(write::append(&path, &["Inbox"], &[]).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

/// A character Windows-1252 lacks anywhere in a message, an attached message's subject or a list
/// of categories among them, has the whole message written in UTF-8 in an ANSI file.
#[test]
fn text_an_ansi_message_can_only_hold_in_utf8_is_found_anywhere_in_it() {
    let path = temp("ansi-greek.pst");
    write::create_empty(&path, Format::Ansi).unwrap();
    let inner = msg("Ελληνικά", &[], &[]);
    let attached = NewAttachment {
        filename: "Forwarded",
        content: AttachmentContent::Message(Box::new(EmbeddedMessage { message: inner, attachments: vec![] })),
        ..Default::default()
    };
    write::append_message(&path, &["Inbox"], &msg("Plain", &[], &[]), std::slice::from_ref(&attached)).unwrap();
    let categories = ["Κατηγορία"];
    let tagged = [NewProperty::tagged(0x6100, NewValue::MultipleUnicode(&categories))];
    write::append_message(&path, &["Inbox"], &msg("Plain too", &[], &tagged), &[]).unwrap();
    let store = open(&path);
    let inbox = child(&store, store.ipm_subtree().unwrap(), "Inbox").unwrap();
    let f = store.open_folder(inbox).unwrap();
    let rows: Vec<u32> = f.contents_table().unwrap().rows_matrix().map(|r| r.id()).collect();
    assert_eq!(rows.len(), 2);
    for id in rows {
        let m = Message::read(store.clone(), id).unwrap();
        assert_eq!(m.properties().get(0x3FFD), Some(&PropertyValue::Integer32(65001)));
        let Some(table) = m.attachment_table() else { continue };
        let row = table.rows_matrix().next().unwrap().id();
        let inner = Attachment::read(m.clone(), row).unwrap().embedded_message().unwrap();
        match inner.properties().get(0x0037) {
            Some(PropertyValue::String8(s)) => assert_eq!(s.buffer(), "Ελληνικά".as_bytes()),
            other => panic!("{other:?}"),
        }
    }
}

/// `PidTagAttachSize` counts the attachment's names as well as what it holds, on the attachment
/// and on its row of the table.
#[test]
fn an_attachments_size_counts_its_names() {
    let path = temp("attach-size.pst");
    write::create_empty(&path, Format::Unicode).unwrap();
    let a = NewAttachment { filename: "a.txt", mime_type: "text/plain", data: b"12345678", ..Default::default() };
    write::append_message(&path, &["Inbox"], &msg("With a file", &[], &[]), std::slice::from_ref(&a)).unwrap();
    let store = open(&path);
    let inbox = child(&store, store.ipm_subtree().unwrap(), "Inbox").unwrap();
    let f = store.open_folder(inbox).unwrap();
    let id = f.contents_table().unwrap().rows_matrix().next().unwrap().id();
    let m = Message::read(store.clone(), id).unwrap();
    let table = m.attachment_table().unwrap();
    let row = table.rows_matrix().next().unwrap();
    let column = table.columns().iter().position(|c| c.prop_id() == 0x0E20).unwrap();
    // 8 held + 12 + "a.txt" and its NUL in UTF-16, twice + "text/plain" and its NUL in UTF-16.
    assert_eq!(table.value(row, column), Some(PropertyValue::Integer32(66)));
    let attachment = Attachment::read(m.clone(), row.id()).unwrap();
    assert_eq!(attachment.properties().get(0x0E20), Some(&PropertyValue::Integer32(66)));
}

/// A folder's name too long for the heap is kept in a subnode, and reads back.
#[test]
fn a_very_long_folder_name_reads_back() {
    let path = temp("long-name.pst");
    write::create_empty(&path, Format::Unicode).unwrap();
    let name = "n".repeat(2500);
    write::create_folder(&path, &[&name], "IPF.Note").unwrap();
    let store = open(&path);
    let folder = child(&store, store.ipm_subtree().unwrap(), &name).unwrap();
    assert_eq!(store.open_folder(folder).unwrap().properties().display_name().unwrap(), name);
}
