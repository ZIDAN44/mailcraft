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

    // Several selected: the pane says how many, so it makes way too.
    s.run("folder.open", &json!({"account": "sample", "folder": "INBOX"})).unwrap();
    s.run("list.open", &json!({"key": keys[0].to_id()})).unwrap();
    s.run("mail.reply", &json!({})).unwrap();
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

/// How many drafts are in Drafts.
fn draft_key_count(s: &mut Session) -> usize {
    s.run("folder.open", &json!({"account": "sample", "folder": "Drafts"})).unwrap();
    s.list.len()
}
