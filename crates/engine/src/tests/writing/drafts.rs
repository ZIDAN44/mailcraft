use super::*;

/// The draft in Drafts with `subject`.
fn draft_key(s: &mut Session, subject: &str) -> mailcraft_model::MessageKey {
    s.run("folder.open", &json!({"account": "sample", "folder": "Drafts"})).unwrap();
    s.list.iter().find(|m| m.subject == subject).map(|m| m.key.clone()).unwrap_or_else(|| panic!("{subject} in Drafts"))
}

/// The owner's ask, closing gap 6 for good: a draft saved in Drafts opens again to carry on
/// writing it, as Outlook opens one, as it was. A reply to mail laid out in tables comes back with
/// its recipients (Bcc too), importance, its text and formatting, the original under it as it
/// came with what was changed in it, its picture in place, and the message it answers; saved
/// again it replaces its copy, and sent it leaves Drafts and marks the original replied to.
#[test]
fn a_draft_opens_again_as_it_was_saved() {
    let mut s = sample_session("draft-reopen");
    let mill = super::quote::open_mill(&mut s);
    s.run("mail.reply", &json!({})).unwrap();
    s.run("compose.update", &json!({"bcc": "boss@studio.example", "importance": "high"})).unwrap();
    {
        let c = s.compose.as_mut().unwrap();
        // Bold words above the quote, and a change in the quote.
        c.body = format!("All **paid**.{}", c.body);
        c.styles = mailcraft_mime::rich::Styles::plain(&c.body);
        c.styles.restyle(&c.body, 0..4, |st| mailcraft_mime::rich::Style { bold: true, ..st.clone() });
        let q = c.quote.clone().unwrap();
        let n = (0..q.cells()).find(|n| q.cell(*n).is_some_and(|(t, _)| t.contains("A warning"))).unwrap();
        c.in_quote_cell(n, |c| {
            c.body = c.body.replace("A warning in red", "Settled");
            c.styles.fit(&c.body);
        });
    }
    s.run("compose.saveDraft", &json!({})).unwrap();
    s.run("compose.close", &json!({"save": false})).unwrap_or_default();
    s.compose = None;
    let key = draft_key(&mut s, "Re: Invoice");
    s.run("list.open", &json!({"key": key.to_id()})).unwrap();
    s.run("draft.edit", &json!({"key": key.to_id()})).unwrap();
    let c = s.compose.clone().expect("being written again");
    assert_eq!(c.draft.as_ref(), Some(&key), "its copy, to replace");
    assert!(c.to.contains("morgan@mill.example") && c.bcc.contains("boss@studio.example") && c.show_cc, "{} / {}", c.to, c.bcc);
    assert_eq!((c.subject.as_str(), c.importance), ("Re: Invoice", 1));
    assert!(c.body.starts_with("All **paid**.") && c.styles.style(0).bold, "{}", c.body);
    assert!(c.body.contains("Subject: Invoice"), "who sent the original, in the text: {}", c.body);
    let rule = c.body.split('\n').position(|l| l.starts_with("From: ")).and_then(|l| l.checked_sub(1)).map(|l| c.styles.para(l).rule);
    assert_eq!(rule, Some(true), "the line over the original");
    let q = c.quote.clone().expect("the original as it came");
    assert!(!q.parsed.layout.is_empty() && (0..q.cells()).any(|n| q.cell(n).is_some_and(|(t, _)| t.contains("Settled"))));
    assert!(c.inline.iter().any(|p| p.cid == "phone@mill.example") && c.attachments.is_empty(), "its picture in place, not a file");
    assert_eq!(c.source.as_ref(), Some(&mill), "the message it answers");
    // Opened again while it's being written: the same one.
    let id = c.id;
    assert_eq!(s.run("draft.edit", &json!({"key": key.to_id()})).unwrap()["compose"], json!(id));
    // Saved again: one copy in Drafts.
    s.run("compose.saveDraft", &json!({})).unwrap();
    s.run("folder.open", &json!({"account": "sample", "folder": "Drafts"})).unwrap();
    assert_eq!(s.list.iter().filter(|m| m.subject == "Re: Invoice").count(), 1, "replaced, not added");
    // Sent: it leaves Drafts, and the original is marked replied to.
    s.run("compose.send", &json!({})).unwrap();
    s.run("folder.open", &json!({"account": "sample", "folder": "Drafts"})).unwrap();
    assert!(!s.list.iter().any(|m| m.subject == "Re: Invoice"), "gone from Drafts once sent");
    s.run("folder.open", &json!({"account": "sample", "folder": "INBOX"})).unwrap();
    assert!(s.list.iter().find(|m| m.key == mill).is_some_and(|m| m.flags.answered), "the original replied to");
    // What's sent carries neither the Bcc nor what it answers: those stay in the draft's copy.
    s.run("folder.open", &json!({"account": "sample", "folder": "Sent"})).unwrap();
    let sent = s.list.iter().find(|m| m.subject == "Re: Invoice").map(|m| m.key.clone()).expect("sent");
    let raw = String::from_utf8_lossy(&s.store.as_ref().unwrap().read_body(&sent).unwrap()).to_string();
    assert!(!raw.contains("boss@studio.example") && !raw.contains(mailcraft_mime::SOURCE_HEADER), "{raw}");
    // A forward's draft comes back a forward of the same message.
    s.run("folder.open", &json!({"account": "sample", "folder": "INBOX"})).unwrap();
    s.run("list.open", &json!({"key": mill.to_id()})).unwrap();
    s.run("mail.forward", &json!({})).unwrap();
    s.run("compose.update", &json!({"to": "ann@x.example"})).unwrap();
    s.run("compose.saveDraft", &json!({})).unwrap();
    s.compose = None;
    let key = draft_key(&mut s, "Fwd: Invoice");
    s.run("list.open", &json!({"key": key.to_id()})).unwrap();
    s.run("draft.edit", &json!({"key": key.to_id()})).unwrap();
    let c = s.compose.as_ref().unwrap();
    assert_eq!((c.source.as_ref(), c.mode), (Some(&mill), Some(mailcraft_mime::ReplyMode::Forward)));
    assert!(s.run("draft.edit", &json!({"key": "nonsense"})).is_err());
    done(s);
}

/// A signature written as HTML code (a card in two columns) comes back as it went in: the same
/// code, sent as written, its photo in it once; its text in the message where it was.
#[test]
fn a_draft_keeps_its_signature_code() {
    let mut s = sample_session("draft-signature-code");
    let png = crate::sample::sample_png();
    let pic = mailcraft_mime::InlinePicture { cid: "photo".into(), name: "photo.png".into(), mime: "image/png".into(), data: png };
    let img = mailcraft_mime::embed_pictures("<img src=\"cid:photo\" width=\"80\" alt=\"Riley\">", &[pic]);
    let code = format!("<table><tr><td>{img}</td><td><b>Riley Moreno</b><br>Producer</td></tr></table>");
    s.run("account.setSignature", &json!({"account": "sample", "signature": "", "code": code})).unwrap();
    s.run("mail.new", &json!({})).unwrap();
    let written = s.compose.clone().unwrap();
    s.run("compose.update", &json!({"to": "ann@x.example", "subject": "Card draft", "body": format!("Hello Ann,{}", written.body)})).unwrap();
    let before = s.compose.as_ref().unwrap().bodies_with(&written.signature_code).1.unwrap_or_default();
    s.run("compose.saveDraft", &json!({})).unwrap();
    s.compose = None;
    let key = draft_key(&mut s, "Card draft");
    s.run("list.open", &json!({"key": key.to_id()})).unwrap();
    s.run("draft.edit", &json!({"key": key.to_id()})).unwrap();
    let c = s.compose.clone().unwrap();
    assert!(c.body.starts_with("Hello Ann,") && c.body.contains("Riley Moreno"), "{}", c.body);
    assert_eq!(c.signature_code, written.signature_code, "the same code");
    assert!(c.signature_at().is_some(), "where it was, as it went in");
    let (_, html, pictures) = c.bodies_with(&c.signature_code);
    let html = html.unwrap_or_default();
    assert_eq!(html.matches("<table").count(), before.matches("<table").count(), "{html}");
    assert_eq!(html.matches("cid:signature1.").count(), 1, "its photo once: {html}");
    assert_eq!(pictures.iter().filter(|p| p.cid.starts_with("signature1.")).count(), 1);
    done(s);
}

/// Saved to a server, a draft's copy gets its place there: saved again, it replaces that copy
/// (found by its Message-ID once listed) instead of adding another, as Outlook keeps one.
#[test]
fn a_draft_saved_twice_to_a_server_replaces_its_copy() {
    let store = temp("draft-server");
    let mut s = Session::open(Some(store.clone()), mailcraft_model::dates::Clock::new(NOW, 0));
    let id = s.run("account.add", &json!({"email": "zoe@example.org", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    s.apply(Event::Folders {
        account: id.clone(),
        folders: vec![
            mailcraft_model::Folder { account: id.clone(), path: "INBOX".into(), role: FolderRole::Inbox, selectable: true, ..Default::default() },
            mailcraft_model::Folder { account: id.clone(), path: "Drafts".into(), role: FolderRole::Drafts, selectable: true, ..Default::default() },
        ],
    });
    s.take_jobs();
    s.run("mail.new", &json!({"account": id})).unwrap();
    s.run("compose.update", &json!({"to": "ann@example.org", "subject": "Plans", "body": "First"})).unwrap();
    s.run("compose.saveDraft", &json!({})).unwrap();
    let Some(Job::SaveDraft { raw, replaces: None, .. }) = s.take_jobs().into_iter().find(|j| matches!(j, Job::SaveDraft { .. })) else {
        panic!("the first save adds a copy")
    };
    // The server filed it; the next sync lists it.
    let mut drafts = store.load_folder(&id, "Drafts").unwrap();
    drafts.validity = 7;
    let key = mailcraft_model::MessageKey { account: id.clone(), folder: "Drafts".into(), validity: 7, uid: 1 };
    drafts
        .messages
        .push(mailcraft_mime::summary_from_full(&raw, key.clone(), mailcraft_model::Flags { draft: true, ..Default::default() }, 0, NOW).into());
    store.save_folder(&id, "Drafts", &drafts).unwrap();
    s.run("compose.update", &json!({"body": "Second"})).unwrap();
    s.run("compose.saveDraft", &json!({})).unwrap();
    let saved = s.take_jobs().into_iter().find_map(|j| match j {
        Job::SaveDraft { replaces, raw, .. } => Some((replaces, raw)),
        _ => None,
    });
    let (replaces, raw) = saved.expect("saved again");
    assert_eq!(replaces, Some(key.clone()), "the copy the server filed");
    // The server swapped them; the next sync lists the new one.
    let key = mailcraft_model::MessageKey { uid: 2, ..key };
    drafts.messages =
        vec![mailcraft_mime::summary_from_full(&raw, key.clone(), mailcraft_model::Flags { draft: true, ..Default::default() }, 0, NOW).into()];
    store.save_folder(&id, "Drafts", &drafts).unwrap();
    s.apply(Event::FolderChanged { account: id.clone(), folder: "Drafts".into() });
    // Sent, the copy leaves Drafts, and the count of what the server has with it (it said there
    // was one more on the server until the next sync).
    s.run("folder.open", &json!({"account": id, "folder": "Drafts"})).unwrap();
    if let Some(f) = s.accounts.iter_mut().find(|a| a.config.id == id).and_then(|a| a.folders.iter_mut().find(|f| f.path == "Drafts")) {
        f.total = 1;
    }
    assert!(s.list.iter().any(|m| m.key == key));
    let compose = s.compose.as_ref().map(|c| c.id).unwrap();
    s.run("compose.send", &json!({})).unwrap();
    s.take_jobs();
    s.apply(Event::Sent { account: id.clone(), id: compose });
    assert!(!s.list.iter().any(|m| m.key == key), "gone from Drafts");
    assert_eq!(s.more_on_server(), None, "nothing more on the server");
    done(s);
}

/// Another message read while one is being written in the reading pane: the one being written
/// makes way for it, kept in Drafts, as Outlook's does (the pane went on showing it, the other
/// message only selected in the list). Nothing written, it just goes; in a window, it stays.
#[test]
fn reading_another_message_keeps_the_one_written_inline_in_drafts() {
    let mut s = sample_session("draft-make-way");
    let keys: Vec<_> = s.list.iter().map(|m| m.key.clone()).take(3).collect();
    s.run("list.open", &json!({"key": keys[0].to_id()})).unwrap();
    s.run("mail.forward", &json!({})).unwrap();
    let body = format!("Please review.{}", s.compose.as_ref().unwrap().body);
    s.run("compose.update", &json!({"to": "zoe@studio.example", "body": body})).unwrap();
    let subject = s.compose.as_ref().map(|c| c.subject.clone()).unwrap();
    assert!(!s.compose.as_ref().unwrap().window, "a forward is written in the reading pane");
    // The message it forwards, opened again: it stays.
    s.run("list.open", &json!({"key": keys[0].to_id()})).unwrap();
    assert!(s.compose.is_some(), "the same message: still being written");
    s.run("list.open", &json!({"key": keys[1].to_id()})).unwrap();
    assert!(s.compose.is_none() && s.composes.is_empty(), "made way for the message read");
    assert_eq!(s.reading.as_ref().map(|r| &r.key), Some(&keys[1]));
    assert!(s.notices.last().is_some_and(|n| n.text == "Draft saved"));
    let key = draft_key(&mut s, &subject);
    s.run("draft.edit", &json!({"key": key.to_id()})).unwrap();
    assert!(s.compose.as_ref().is_some_and(|c| c.to.contains("zoe@studio.example")), "carried on from Drafts");
    assert!(s.compose.as_ref().is_some_and(|c| c.body.starts_with("Please review.")), "the body kept in Drafts");
    s.compose = None;

    // Nothing written: no draft.
    s.run("folder.open", &json!({"account": "sample", "folder": "INBOX"})).unwrap();
    s.run("view.newMailWindow", &json!({"on": false})).unwrap();
    s.run("mail.new", &json!({})).unwrap();
    s.run("list.open", &json!({"key": keys[2].to_id()})).unwrap();
    assert!(s.compose.is_none());
    assert_eq!(draft_key_count(&mut s), 1, "only the forward");

    // A reply left as it was made (its recipients and subject filled in, nothing written): it
    // goes without a draft, as Outlook's (#52).
    s.run("folder.open", &json!({"account": "sample", "folder": "INBOX"})).unwrap();
    s.run("list.open", &json!({"key": keys[0].to_id()})).unwrap();
    s.run("mail.reply", &json!({})).unwrap();
    s.run("list.open", &json!({"key": keys[1].to_id()})).unwrap();
    assert!(s.compose.is_none());
    assert_eq!(draft_key_count(&mut s), 1, "still only the forward");

    // Several selected: the pane says how many, so it makes way too.
    s.run("folder.open", &json!({"account": "sample", "folder": "INBOX"})).unwrap();
    s.run("list.open", &json!({"key": keys[0].to_id()})).unwrap();
    s.run("mail.reply", &json!({})).unwrap();
    let body = format!("Yes.{}", s.compose.as_ref().unwrap().body);
    s.run("compose.update", &json!({"body": body})).unwrap();
    s.run("list.selectRange", &json!({"key": keys[2].to_id()})).unwrap();
    assert!(s.compose.is_none() && s.selection.len() == 3);
    assert_eq!(draft_key_count(&mut s), 2, "the forward and the reply");

    // In a window of its own: it stays as other messages are read.
    s.run("folder.open", &json!({"account": "sample", "folder": "INBOX"})).unwrap();
    s.run("view.newMailWindow", &json!({"on": true})).unwrap();
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"subject": "Plans"})).unwrap();
    s.run("list.open", &json!({"key": keys[1].to_id()})).unwrap();
    assert!(s.compose.as_ref().is_some_and(|c| c.window && c.subject == "Plans"));
}

/// Found in review: two saves of one message on the way to the server, and the first said done:
/// the next save, or the draft kept as MailCraft closed, was made a new draft (another Message-ID,
/// replacing nothing), and Drafts ended with two. It's the same draft until the last save is said
/// done; a message closed meanwhile is held until then too.
#[test]
fn a_draft_saved_twice_on_the_way_stays_one_draft() {
    let store = temp("draft-twice");
    let mut s = Session::open(Some(store), mailcraft_model::dates::Clock::new(NOW, 0));
    let id = s.run("account.add", &json!({"email": "zoe@example.org", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    let drafts =
        mailcraft_model::Folder { account: id.clone(), path: "Drafts".into(), role: FolderRole::Drafts, selectable: true, ..Default::default() };
    s.apply(Event::Folders { account: id.clone(), folders: vec![drafts] });
    s.take_jobs();
    let saved = |s: &mut Session| {
        let raw = s.take_jobs().into_iter().find_map(|j| match j {
            Job::SaveDraft { raw, .. } | Job::KeepDraft { raw, .. } => Some(raw),
            _ => None,
        });
        mailcraft_model::thread::written_message_id(&raw.expect("a draft saved"))
    };
    s.run("mail.new", &json!({"account": id})).unwrap();
    s.run("compose.update", &json!({"to": "ann@example.org", "subject": "Plans", "body": "One"})).unwrap();
    s.save_draft().unwrap();
    let first = saved(&mut s);
    s.run("compose.update", &json!({"body": "One two"})).unwrap();
    s.save_draft().unwrap();
    assert_eq!(saved(&mut s), first);
    s.apply(Event::DraftSaved { account: id.clone(), message_id: first.clone(), error: None });
    s.run("compose.update", &json!({"body": "One two three"})).unwrap();
    s.save_draft().unwrap();
    assert_eq!(saved(&mut s), first, "the next save, the same draft");
    // Kept as MailCraft closes: the same draft too.
    assert_eq!(s.keep_drafts(), 1);
    assert_eq!(saved(&mut s), first, "kept as MailCraft closes, the same draft");
    // Closed with two saves on the way: held until the last is said done.
    s.apply(Event::DraftSaved { account: id.clone(), message_id: first.clone(), error: None });
    s.run("compose.update", &json!({"body": "One two three four"})).unwrap();
    s.run("compose.close", &json!({})).unwrap();
    assert_eq!(saved(&mut s), first);
    assert_eq!(s.saving.len(), 1, "held");
    s.apply(Event::DraftSaved { account: id.clone(), message_id: first.clone(), error: None });
    assert_eq!(s.saving.len(), 1, "held for the last save");
    s.apply(Event::DraftSaved { account: id.clone(), message_id: first, error: Some("disk full".into()) });
    assert!(s.saving.is_empty());
    assert!(s.composes.iter().any(|c| c.window && c.body.starts_with("One two three four")), "open again to go on with");
    done(s);
}

/// Issue #53: a server account's message closed with Close › keep in Drafts, or made way for in
/// the reading pane, while its draft is still on the way to the server, is held until the server
/// or this computer has kept the draft; when neither could, it comes back in a window, saying
/// why, instead of being lost. Held, it's still kept as a draft if MailCraft closes.
#[test]
fn a_message_is_held_until_its_draft_is_kept() {
    let store = temp("draft-held");
    let mut s = Session::open(Some(store.clone()), mailcraft_model::dates::Clock::new(NOW, 0));
    let id = s.run("account.add", &json!({"email": "zoe@example.org", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    s.apply(Event::Folders {
        account: id.clone(),
        folders: vec![
            mailcraft_model::Folder { account: id.clone(), path: "INBOX".into(), role: FolderRole::Inbox, selectable: true, ..Default::default() },
            mailcraft_model::Folder { account: id.clone(), path: "Drafts".into(), role: FolderRole::Drafts, selectable: true, ..Default::default() },
        ],
    });
    // Two messages to read.
    let mut inbox = store.load_folder(&id, "INBOX").unwrap();
    inbox.validity = 3;
    let mut keys = Vec::new();
    for uid in 1..=2u32 {
        let key = mailcraft_model::MessageKey { account: id.clone(), folder: "INBOX".into(), validity: 3, uid };
        let raw = format!(
            "From: Ann <ann@example.org>\r\nTo: zoe@example.org\r\nSubject: Plans {uid}\r\nMessage-ID: <p{uid}@example.org>\r\n\r\nShall we?\r\n"
        );
        store.write_body(&key, raw.as_bytes()).unwrap();
        inbox.messages.push(mailcraft_mime::summary_from_full(raw.as_bytes(), key.clone(), mailcraft_model::Flags::default(), 0, NOW).into());
        keys.push(key);
    }
    store.save_folder(&id, "INBOX", &inbox).unwrap();
    s.apply(Event::FolderChanged { account: id.clone(), folder: "INBOX".into() });
    s.run("folder.open", &json!({"account": id, "folder": "INBOX"})).unwrap();
    s.take_jobs();
    let saved_id = |s: &mut Session| {
        let raw = s.take_jobs().into_iter().find_map(|j| if let Job::SaveDraft { raw, .. } = j { Some(raw) } else { None });
        mailcraft_model::thread::written_message_id(&raw.expect("a draft saved"))
    };
    let write = |s: &mut Session, body: &str| {
        s.run("mail.new", &json!({"account": id})).unwrap();
        s.run("compose.update", &json!({"to": "ann@example.org", "subject": body, "body": body})).unwrap();
    };

    // Saved with Save and still open: said saved once the server has it, not before.
    write(&mut s, "Kept open");
    s.save_draft().unwrap();
    let open_id = saved_id(&mut s);
    assert!(s.notices.last().is_none_or(|n| n.text != "Draft saved"), "not yet");
    s.apply(Event::DraftSaved { account: id.clone(), message_id: open_id, error: None });
    assert!(s.notices.last().is_some_and(|n| n.text == "Draft saved"));
    s.compose = None;

    // Saved with Save, then MailCraft closed (or the graphics lost) before the server answered:
    // kept as the same draft, its Message-ID and the copy it replaces, not a second one (found in
    // review: built again with a new Message-ID and nothing to replace).
    write(&mut s, "Open and on its way");
    s.save_draft().unwrap();
    let sent = s.take_jobs().into_iter().find_map(|j| if let Job::SaveDraft { raw, replaces, .. } = j { Some((raw, replaces)) } else { None });
    let (sent, replaces) = sent.expect("a draft saved");
    assert_eq!(s.keep_drafts(), 1);
    let kept = s.take_jobs().into_iter().find_map(|j| if let Job::KeepDraft { raw, replaces, .. } = j { Some((raw, replaces)) } else { None });
    let (kept, kept_replaces) = kept.expect("kept");
    assert_eq!(mailcraft_model::thread::written_message_id(&kept), mailcraft_model::thread::written_message_id(&sent));
    assert_eq!(kept_replaces, replaces);
    s.compose = None;

    // Closed, kept in Drafts: held until the server has it, then gone.
    write(&mut s, "First");
    s.run("compose.close", &json!({})).unwrap();
    let first = saved_id(&mut s);
    assert!(s.compose.is_none() && s.saving.len() == 1 && s.composes.is_empty(), "held, out of sight");
    assert!(s.notices.last().is_none_or(|n| n.text != "Saved in Drafts"), "not yet");
    s.apply(Event::DraftSaved { account: id.clone(), message_id: format!("<{first}>"), error: None });
    assert!(s.saving.is_empty() && s.composes.is_empty(), "kept: let go of");
    assert!(s.notices.last().is_some_and(|n| n.text == "Saved in Drafts"));

    // Closed, and neither the server nor this computer could keep it: back, with why.
    write(&mut s, "Second");
    s.run("compose.close", &json!({})).unwrap();
    let second = saved_id(&mut s);
    s.apply(Event::DraftSaved { account: id.clone(), message_id: "someone-else@example.org".into(), error: Some("disk full".into()) });
    assert_eq!(s.saving.len(), 1, "another draft's failure isn't this one's");
    s.apply(Event::DraftSaved { account: id.clone(), message_id: second, error: Some("disk full".into()) });
    assert!(s.saving.is_empty());
    assert!(s.composes.iter().any(|c| c.window && c.body.starts_with("Second")), "open again to go on with");
    assert!(s.notices.last().is_some_and(|n| n.text.contains("disk full")));
    s.composes.clear();

    // Written in the reading pane, made way for by another message read: held the same way, and
    // kept as a draft if MailCraft closes meanwhile.
    s.run("view.newMailWindow", &json!({"on": false})).unwrap();
    s.run("list.open", &json!({"key": keys[0].to_id()})).unwrap();
    write(&mut s, "Third");
    assert!(!s.compose.as_ref().unwrap().window, "in the reading pane");
    s.run("list.open", &json!({"key": keys[1].to_id()})).unwrap();
    let on_its_way = s.take_jobs().into_iter().find_map(|j| if let Job::SaveDraft { raw, replaces, .. } = j { Some((raw, replaces)) } else { None });
    let on_its_way = on_its_way.expect("a draft saved");
    let third = mailcraft_model::thread::written_message_id(&on_its_way.0);
    assert!(s.compose.is_none() && s.saving.len() == 1);
    assert_eq!(s.keep_drafts(), 1);
    assert!(s.saving.is_empty(), "kept: let go of");
    // Kept as the very draft on its way (its Message-ID and the copy it replaces), so if the save
    // gets there too it's one draft, not two (found in review: built again with a new Message-ID).
    let kept = s.take_jobs().into_iter().find_map(|j| if let Job::KeepDraft { raw, replaces, .. } = j { Some((raw, replaces)) } else { None });
    assert_eq!(kept.as_ref(), Some(&on_its_way), "kept as MailCraft closes");
    // The save on its way failing after that: the draft is on this computer already.
    s.apply(Event::DraftSaved { account: id.clone(), message_id: third, error: Some("no connection".into()) });
    assert!(s.composes.is_empty());

    // A front end that never answers: past 64 held, the oldest is kept on this computer.
    for n in 0..=64 {
        write(&mut s, &format!("Many {n}"));
        s.run("compose.close", &json!({})).unwrap();
    }
    assert_eq!(s.saving.len(), 64);
    let kept: Vec<String> = s
        .take_jobs()
        .into_iter()
        .filter_map(|j| if let Job::KeepDraft { raw, .. } = j { Some(String::from_utf8_lossy(&raw).into_owned()) } else { None })
        .collect();
    assert!(kept.len() == 1 && kept[0].contains("Subject: Many 0\r\n"), "{kept:?}");

    // The account removed while messages are held: they go with it, not kept as MailCraft closes
    // into a removed account's folder (found in review).
    assert!(!s.saving.is_empty());
    s.run("account.remove", &json!({"account": id})).unwrap();
    assert!(s.saving.is_empty());
    assert_eq!(s.keep_held(), 0);
    done(s);
}

/// Issue #52: a message whose reply was left in Drafts is known to have one (the list marks it
/// `[Draft]`, as Outlook does), from MailCraft's start on and once the draft is saved; one opened
/// and left empty isn't kept, so isn't marked; the draft deleted, the mark goes.
#[test]
fn a_message_answered_by_a_draft_is_known() {
    let mut s = sample_session("drafts-answer");
    let m = s.list[0].clone();
    let answered = |s: &Session| s.answered_by_drafts().contains(&("sample".to_string(), mailcraft_model::thread::norm_id(&m.message_id)));
    // A reply opened and closed with nothing written: no draft, no mark.
    s.run("list.open", &json!({"key": m.key.to_id()})).unwrap();
    s.run("mail.reply", &json!({})).unwrap();
    s.run("compose.close", &json!({})).unwrap();
    assert!(!answered(&s));
    // Written, and kept in Drafts.
    s.run("mail.reply", &json!({})).unwrap();
    let body = format!("Soon.{}", s.compose.as_ref().unwrap().body);
    s.run("compose.update", &json!({"body": body})).unwrap();
    let subject = s.compose.as_ref().map(|c| c.subject.clone()).unwrap();
    s.run("compose.close", &json!({})).unwrap();
    assert!(answered(&s), "a reply in Drafts");
    // Known from the start, Drafts never opened.
    let store = s.store.clone().unwrap();
    drop(s);
    let mut s = Session::open(Some(store), mailcraft_model::dates::Clock::new(NOW, 0));
    assert!(answered(&s), "read from Drafts as MailCraft starts");
    assert!(std::sync::Arc::ptr_eq(&s.answered_by_drafts(), &s.answered_by_drafts()), "asked again, Drafts the same: as found");
    // The draft deleted: no more.
    let key = draft_key(&mut s, &subject);
    s.run("list.select", &json!({"key": key.to_id()})).unwrap();
    s.run("mail.delete", &json!({})).unwrap();
    assert!(!answered(&s), "the draft deleted");
    done(s);
}

/// How many drafts are in Drafts.
fn draft_key_count(s: &mut Session) -> usize {
    s.run("folder.open", &json!({"account": "sample", "folder": "Drafts"})).unwrap();
    s.list.len()
}

/// A reply counts as written once anything the user can change in it changed, not only its
/// words and recipients: its text's look, who it's from, its options, a picture or a file swapped
/// for another of the same name and size (found in review: those closed asking nothing, and were
/// lost). Saved, then edited back to how it was saved, it's what Drafts has: nothing asked.
#[test]
fn every_change_to_a_reply_is_something_written() {
    let mut s = sample_session("reply-changes");
    s.run("folder.open", &json!({"account": "sample", "folder": "INBOX"})).unwrap();
    let key = s.list[0].key.clone();
    s.run("list.open", &json!({"key": key.to_id()})).unwrap();
    s.run("mail.reply", &json!({})).unwrap();
    s.run("compose.attach", &json!({"name": "plan.txt", "data": b"AAAA".to_vec()})).unwrap();
    s.compose.as_mut().unwrap().mark_made();
    assert!(!s.compose_written(), "as made");
    let changes: Vec<(&str, fn(&mut crate::Compose))> = vec![
        ("bold", |c| c.styles.restyle(&c.body, 0..1, |st| mailcraft_mime::rich::Style { bold: true, ..st.clone() })),
        ("from", |c| c.account = "other".into()),
        ("delay", |c| c.send_after = Some(NOW + 3600)),
        ("encrypt", |c| (c.encrypt, c.encrypt_auto) = (true, false)),
        ("plain", |c| c.plain = true),
        ("file", |c| c.attachments[0].data = b"BBBB".to_vec()),
    ];
    for (what, change) in changes {
        let mut c = s.compose.clone().unwrap();
        change(&mut c);
        assert!(!c.unchanged(), "{what} counts as written");
    }
    // Encryption that follows the recipients on its own isn't the user's change.
    let mut c = s.compose.clone().unwrap();
    c.encrypt_auto = true;
    c.mark_made();
    c.encrypt = !c.encrypt;
    assert!(c.unchanged());

    // Saved, edited, edited back: as Drafts has it.
    s.run("compose.update", &json!({"subject": "Middle"})).unwrap();
    s.save_draft().unwrap();
    s.run("compose.update", &json!({"subject": "Later"})).unwrap();
    assert!(s.compose_written());
    s.run("compose.update", &json!({"subject": "Middle"})).unwrap();
    assert!(!s.compose_written(), "as saved");

    // A server's save counts once the server has it; one that failed is still to be saved.
    let id = s.run("account.add", &json!({"email": "zoe@example.org", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    s.run("mail.new", &json!({"account": id})).unwrap();
    s.run("compose.update", &json!({"to": "ann@example.org", "subject": "Middle"})).unwrap();
    s.save_draft().unwrap();
    let mid = s.compose.as_ref().unwrap().draft_id.clone();
    s.run("compose.update", &json!({"subject": "Later"})).unwrap();
    s.run("compose.update", &json!({"subject": "Middle"})).unwrap();
    assert!(s.compose_written(), "not on the server yet");
    s.apply(Event::DraftSaved { account: id.clone(), message_id: mid.clone(), error: None });
    assert!(!s.compose_written(), "as the server has it");
    s.run("compose.update", &json!({"subject": "Last"})).unwrap();
    s.save_draft().unwrap();
    let mid = s.compose.as_ref().unwrap().draft_id.clone();
    s.apply(Event::DraftSaved { account: id, message_id: mid, error: Some("disk full".into()) });
    assert!(s.compose_written(), "not saved after all");
    done(s);
}
