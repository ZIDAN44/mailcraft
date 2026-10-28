//! Engine tests: writing, saving, sending and the Outbox.

use super::*;

#[test]
fn compose_reply_and_send_locally() {
    let mut s = sample_session("compose");
    let k = key_of(&s, "Your proof for the festival poster is ready");
    s.run("list.open", &k).unwrap();
    s.run("mail.reply", &json!({})).unwrap();
    let c = s.compose.clone().unwrap();
    assert_eq!(c.to, "Print Shop <orders@printshop.example>; ", "ready for another address");
    assert_eq!(c.subject, "Re: Your proof for the festival poster is ready");
    assert!(c.body.contains("\nFrom: Print Shop <orders@printshop.example>\nSent: "));
    assert!(c.body.contains("The proof for order 4471"));
    assert!(c.body.contains("Riley Moreno · Studio Lantern"));
    assert!(!c.window, "replies start in the reading pane");
    s.run("compose.popOut", &json!({})).unwrap();
    assert!(s.compose.as_ref().unwrap().window);
    s.run("compose.popIn", &json!({})).unwrap();
    assert!(!s.compose.as_ref().unwrap().window);
    s.run("compose.update", &json!({"body": "Approved, thank you!\n"})).unwrap();
    s.run("compose.send", &json!({})).unwrap();
    assert!(s.compose.is_none());
    assert!(s.list.iter().find(|m| m.subject.starts_with("Your proof")).unwrap().flags.answered);
    s.run("folder.open", &json!({"account": "sample", "folder": "Sent"})).unwrap();
    assert!(s.list.iter().any(|m| m.subject == "Re: Your proof for the festival poster is ready"));

    // Validation.
    s.run("mail.new", &json!({})).unwrap();
    assert!(matches!(s.run("compose.send", &json!({})), Err(CmdError::Params(_))));
    s.run("compose.update", &json!({"to": "not an address"})).unwrap();
    assert!(matches!(s.run("compose.send", &json!({})), Err(CmdError::Params(m)) if m.contains("not an address")));
    s.run("compose.update", &json!({"to": "ana@soundlab.example"})).unwrap();
    assert!(matches!(s.run("compose.send", &json!({})), Err(CmdError::Params(m)) if m.contains("subject")));
    s.run("compose.attach", &json!({"name": "notes.txt", "data": [104, 105]})).unwrap();
    s.run("compose.saveDraft", &json!({})).unwrap();
    s.run("compose.saveDraft", &json!({})).unwrap();
    s.run("folder.open", &json!({"account": "sample", "folder": "Drafts"})).unwrap();
    assert_eq!(s.list.len(), 1, "saving again replaces the earlier draft");
    s.run("compose.send", &json!({"allowEmptySubject": true})).unwrap();
    assert!(s.list.is_empty(), "the draft goes once sent");

    // Forward carries attachments.
    s.run("folder.open", &json!({"account": "sample", "folder": "INBOX"})).unwrap();
    let k = key_of(&s, "Character turnarounds");
    s.run("list.open", &k).unwrap();
    s.run("mail.forward", &json!({})).unwrap();
    assert_eq!(s.compose.as_ref().unwrap().attachments.len(), 1);
    s.run("compose.discard", &json!({})).unwrap();
    assert!(s.compose.is_none());

    // Forward as attachment: the whole original as an .eml, sent as it is (not base64).
    s.run("mail.forwardAsAttachment", &json!({})).unwrap();
    let c = s.compose.clone().unwrap();
    assert_eq!(c.subject, "Fwd: Character turnarounds");
    assert!(!c.body.contains("Here are the first turnarounds"), "the original isn't quoted");
    let a = &c.attachments[0];
    assert_eq!((c.attachments.len(), a.name.as_str(), a.mime.as_str()), (1, "Character turnarounds.eml", "message/rfc822"));
    assert!(String::from_utf8_lossy(&a.data).contains("Subject: Character turnarounds"));
    s.run("compose.update", &json!({"to": "ana@soundlab.example"})).unwrap();
    s.run("compose.send", &json!({})).unwrap();
    s.run("folder.open", &json!({"account": "sample", "folder": "Sent"})).unwrap();
    let k = s.list.iter().find(|m| m.subject == "Fwd: Character turnarounds").unwrap().key.clone();
    let sent = s.store.as_ref().unwrap().read_body(&k).unwrap();
    let sent = String::from_utf8_lossy(&sent);
    let part = sent.split("Content-Type: message/rfc822").nth(1).unwrap();
    assert!(part.contains("Subject: Character turnarounds") && !part[..300.min(part.len())].contains("base64"));
    done(s);
}

#[test]
fn messages_save_as_eml_and_pdf() {
    let mut s = sample_session("export");
    let k = key_of(&s, "Storyboard review on Thursday");
    s.run("list.open", &k).unwrap();
    let dir = s.store.as_ref().unwrap().root().join("export");
    std::fs::create_dir_all(&dir).unwrap();
    let eml = dir.join("m.eml");
    s.run("message.saveAs", &json!({"path": eml.to_string_lossy()})).unwrap();
    assert_eq!(mailcraft_mime::parse(&std::fs::read(&eml).unwrap()).unwrap().subject, "Storyboard review on Thursday");
    if let Some(font) =
        ["C:\\Windows\\Fonts\\segoeui.ttf", "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"].iter().find_map(|p| std::fs::read(p).ok())
    {
        s.print_fonts = Some((font, None));
    }
    let pdf = dir.join("m.pdf");
    s.run("message.exportPdf", &json!({"path": pdf.to_string_lossy()})).unwrap();
    let bytes = std::fs::read(&pdf).unwrap();
    assert!(bytes.starts_with(b"%PDF-") && bytes.ends_with(b"%%EOF\n"));
    if let Ok(keep) = std::env::var("MAILCRAFT_KEEP_PDF") {
        let _ = std::fs::copy(&pdf, keep);
    }
    done(s);
}

#[test]
fn attachments_preview_and_save_all() {
    let mut s = sample_session("attachments");
    let k = key_of(&s, "Your proof for the festival poster is ready");
    s.run("list.open", &k).unwrap();
    let r = s.run("attachment.preview", &json!({"index": 0})).unwrap();
    assert_eq!((r["kind"].as_str(), r["name"].as_str()), (Some("text"), Some("proof-notes.txt")));
    assert!(!r["text"].as_str().unwrap().is_empty());
    let k = key_of(&s, "Character turnarounds");
    s.run("list.open", &k).unwrap();
    assert!(s.run("attachment.preview", &json!({"index": 0})).is_err(), "a picture isn't text");
    let dir = s.store.as_ref().unwrap().root().join("out");
    std::fs::create_dir_all(&dir).unwrap();
    s.run("attachment.saveAll", &json!({"dir": dir.to_string_lossy()})).unwrap();
    let again = s.run("attachment.saveAll", &json!({"dir": dir.to_string_lossy()})).unwrap();
    assert!(again["saved"][0].as_str().unwrap().contains("(2)"), "names already taken get a number");
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
    // Only the ones asked for (those selected).
    let only = s.run("attachment.saveAll", &json!({"dir": dir.to_string_lossy(), "indices": [9999]})).unwrap();
    assert_eq!(only["saved"].as_array().map(Vec::len), Some(0));
    let index = s.reading.as_ref().and_then(|r| r.parsed.as_ref()).map(|p| p.attachments[0].index).unwrap();
    let only = s.run("attachment.saveAll", &json!({"dir": dir.to_string_lossy(), "indices": [index]})).unwrap();
    assert_eq!(only["saved"].as_array().map(Vec::len), Some(1));
    assert!(s.run("attachment.saveAll", &json!({"dir": dir.to_string_lossy(), "indices": "junk"})).is_ok(), "junk means all");
    assert!(s.run("attachment.saveAll", &json!({"dir": "/no/such/folder"})).is_err());
    assert_eq!(mailcraft_mime::craft_app_for("application/pdf", "a.pdf"), Some(("pdfcraft", "PdfCraft")));
    assert_eq!(mailcraft_mime::craft_app_for("application/octet-stream", "q.XLSX").map(|a| a.0), Some("gridcraft"));
    assert_eq!(mailcraft_mime::text_preview_kind("message/rfc822", "fwd.eml"), Some("message"));
    done(s);
}

#[test]
fn formatted_messages_go_out_as_html_with_pictures() {
    let mut s = sample_session("rich");
    s.run("mail.new", &json!({})).unwrap();
    // Agents and the command line can write with marks.
    s.run("compose.update", &json!({"to": "ann@x.example", "subject": "Plan", "body": "Hi **Ann**,\n- frames\n- sound\n", "marks": true})).unwrap();
    let c = s.compose.as_ref().unwrap();
    assert_eq!(c.body, "Hi Ann,\nframes\nsound\n");
    assert!(c.styles.style(3).bold && c.styles.para(1).list == mailcraft_mime::rich::List::Bullet);
    let png = crate::sample::sample_png();
    let r = s.run("compose.insertPicture", &json!({"name": "board.png", "data": png})).unwrap();
    assert!(r["cid"].as_str().is_some());
    assert!(s.compose.as_ref().unwrap().body.ends_with(mailcraft_mime::rich::PICTURE));
    assert!(s.run("compose.insertPicture", &json!({"name": "notes.txt", "data": [1]})).is_err(), "pictures only");
    // Pictures are shown and sent at the width they're given (issue #10), within limits.
    let cid = r["cid"].as_str().unwrap().to_string();
    let width = |s: &Session| {
        let c = s.compose.as_ref().unwrap();
        let at = c.body.chars().count() - 1;
        c.styles.style(at).picture_width
    };
    assert_eq!(width(&s), None, "its own width");
    s.run("compose.resizePicture", &json!({"cid": cid, "width": 240})).unwrap();
    assert_eq!(width(&s), Some(240));
    assert!(s.compose.as_ref().unwrap().bodies().1.unwrap().contains("width=\"240\""));
    s.run("compose.resizePicture", &json!({"cid": cid, "width": 5})).unwrap();
    assert_eq!(width(&s), Some(16), "never narrower than 16");
    s.run("compose.resizePicture", &json!({"cid": cid, "width": 99_999_999})).unwrap();
    assert_eq!(width(&s), Some(4000));
    s.run("compose.resizePicture", &json!({"cid": cid})).unwrap();
    assert_eq!(width(&s), None, "back to its own");
    assert!(s.run("compose.resizePicture", &json!({"cid": "nope@x"})).is_err());
    let wide =
        s.run("compose.insertPicture", &json!({"name": "wide.png", "data": crate::sample::sample_png(), "width": 300, "insert": false})).unwrap();
    assert!(wide["cid"].is_string());
    s.run("compose.resizePicture", &json!({"cid": cid, "width": 320})).unwrap();
    s.run("compose.send", &json!({})).unwrap();
    s.run("folder.open", &json!({"account": "sample", "folder": "Sent"})).unwrap();
    let k = key_of(&s, "Plan");
    s.run("list.open", &k).unwrap();
    let p = s.reading.as_ref().unwrap().parsed.clone().unwrap();
    assert!(p.had_html);
    assert!(p.rich.iter().flatten().any(|sp| sp.bold && sp.text == "Ann"));
    assert!(p.text.starts_with("Hi Ann,\n• frames"), "{}", p.text);
    assert_eq!(p.images.len(), 1);
    assert!(p.attachments.iter().any(|a| a.inline && a.content_id.is_some()));

    // Plain text: the marks go as typed, no HTML.
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"to": "ann@x.example", "subject": "Plain", "body": "Keep **these** stars"})).unwrap();
    s.run("compose.format", &json!({"plain": true})).unwrap();
    s.run("compose.send", &json!({})).unwrap();
    s.run("folder.open", &json!({"account": "sample", "folder": "Sent"})).unwrap();
    let k = key_of(&s, "Plain");
    s.run("list.open", &k).unwrap();
    let p = s.reading.as_ref().unwrap().parsed.clone().unwrap();
    assert!(!p.had_html && p.text.contains("**these**"));
    done(s);
}

#[test]
fn offline_sends_wait_in_the_outbox() {
    let store = temp("outbox");
    let mut s = Session::open(Some(store.clone()), mailcraft_model::dates::Clock::new(NOW, 0));
    let id = s.run("account.add", &json!({"email": "zoe@example.org", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    s.take_jobs();
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"to": "ann@example.org", "bcc": "boss@example.org", "subject": "Plans", "body": "See you"})).unwrap();
    s.run("compose.attach", &json!({"name": "a.txt", "data": [104, 105]})).unwrap();
    s.run("compose.send", &json!({})).unwrap();
    let Some(Job::Send { id: send_id, raw, from, recipients, .. }) = s.take_jobs().into_iter().find(|j| matches!(j, Job::Send { .. })) else {
        panic!()
    };
    // The network layer couldn't send: it put the message in the Outbox.
    let item = mailcraft_store::OutboxItem { id: send_id, from, recipients, queued: NOW, error: Some("offline".into()), ..Default::default() };
    store.outbox_add(&id, &item, &raw).unwrap();
    s.apply(Event::Queued { account: id.clone(), id: send_id, error: "offline".into() });
    assert!(s.compose.is_none(), "a queued message is done for the user");
    assert!(s.notices.last().unwrap().text.contains("Outbox"));
    assert_eq!(s.outbox.len(), 1);

    s.run("folder.outbox", &json!({})).unwrap();
    assert_eq!(s.view_title(), "Outbox");
    assert_eq!(s.list.len(), 1);
    assert_eq!(s.list[0].subject, "Plans");
    let key = s.list[0].key.clone();
    s.run("list.open", &json!({"key": key.to_id()})).unwrap();
    assert!(s.reading.as_ref().unwrap().parsed.as_ref().unwrap().text.contains("See you"));
    // Mail commands leave Outbox entries alone; Send now asks the network layer.
    s.run("mail.flag", &json!({})).unwrap();
    assert!(s.take_jobs().is_empty());
    s.run("outbox.send", &json!({})).unwrap();
    assert!(matches!(s.take_jobs().as_slice(), [Job::FlushOutbox { .. }]));

    // Edit takes it back out, Bcc and attachment included.
    s.run("outbox.edit", &json!({"key": key.to_id()})).unwrap();
    let c = s.compose.clone().unwrap();
    assert_eq!((c.to.as_str(), c.bcc.as_str(), c.subject.as_str()), ("ann@example.org; ", "boss@example.org; ", "Plans"));
    assert_eq!(c.attachments.len(), 1);
    assert!(s.outbox.is_empty() && store.outbox(&id).is_empty());

    // Deleting from the Outbox cancels the send.
    store.outbox_add(&id, &item, &raw).unwrap();
    s.apply(Event::Queued { account: id.clone(), id: send_id, error: "later".into() });
    assert_eq!(s.list.len(), 1);
    s.run("list.selectAll", &json!({})).unwrap();
    s.run("mail.delete", &json!({})).unwrap();
    assert!(s.list.is_empty() && store.outbox(&id).is_empty());
    // A later success from the Outbox is just a notice.
    s.apply(Event::Sent { account: id.clone(), id: 999 });
    assert_eq!(s.notices.last().unwrap().text, "Message sent");
    let _ = std::fs::remove_dir_all(store.root());
}

#[test]
fn new_mail_opens_in_a_window_and_closing_keeps_a_draft() {
    let mut s = sample_session("compose-window");
    s.run("mail.new", &json!({})).unwrap();
    assert!(s.compose.as_ref().unwrap().window, "new mail opens in its own window");
    // Nothing written (just the signature): closing leaves nothing behind.
    let v = s.run("compose.close", &json!({})).unwrap();
    assert_eq!(v["saved"], false);
    assert!(s.compose.is_none());

    s.run("view.newMailWindow", &json!({"on": false})).unwrap();
    s.run("mail.new", &json!({})).unwrap();
    assert!(!s.compose.as_ref().unwrap().window, "unless the option is off");
    s.run("compose.update", &json!({"to": "zoe@studio.example", "subject": "Plans"})).unwrap();
    let v = s.run("compose.close", &json!({})).unwrap();
    assert_eq!(v["saved"], true);
    assert!(s.compose.is_none());
    assert!(s.notices.last().is_some_and(|n| n.text == "Saved in Drafts"));
    s.run("folder.open", &json!({"account": "sample", "folder": "Drafts"})).unwrap();
    assert!(s.list.iter().any(|m| m.subject == "Plans"));
    // Closing without keeping it.
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"subject": "Gone"})).unwrap();
    assert_eq!(s.run("compose.close", &json!({"save": false})).unwrap()["saved"], false);
    done(s);
}

/// View › Arrange by, as Outlook's: sorted and grouped by sender, size, flag…, and the other way
/// round.
#[test]
fn the_list_is_arranged_by_sender_size_flag_and_reversed() {
    let mut s = sample_session("arrange");
    let dates: Vec<i64> = s.list.iter().map(|m| m.date).collect();
    assert!(dates.windows(2).all(|w| w[0] >= w[1]), "newest first by default");
    s.run("view.arrange", &json!({"by": "from"})).unwrap();
    let names: Vec<String> =
        s.list.iter().map(|m| if m.from.name.is_empty() { m.from.email.to_lowercase() } else { m.from.name.to_lowercase() }).collect();
    assert!(names.windows(2).all(|w| w[0] <= w[1]), "senders A to Z: {names:?}");
    assert!(s.rows().iter().any(|r| matches!(r, Row::Group(g) if g == "Zoe Park")), "under each sender");
    s.run("view.reverseSort", &json!({})).unwrap();
    let names: Vec<String> =
        s.list.iter().map(|m| if m.from.name.is_empty() { m.from.email.to_lowercase() } else { m.from.name.to_lowercase() }).collect();
    assert!(names.windows(2).all(|w| w[0] >= w[1]), "Z to A reversed");
    s.run("view.arrange", &json!({"by": "size"})).unwrap();
    assert!(!s.prefs.reverse_sort, "a new arrangement starts in its own order");
    assert!(s.list.windows(2).all(|w| w[0].size >= w[1].size), "largest first");
    assert!(s.rows().iter().any(|r| matches!(r, Row::Header(h) if crate::ARRANGE_HEADINGS.contains(h))));
    s.run("view.arrange", &json!({"by": "flag"})).unwrap();
    let rows = s.rows();
    let at = |h: &'static str| rows.iter().position(|r| *r == Row::Header(h));
    assert!(matches!((at("Flagged"), at("Not flagged")), (Some(a), Some(b)) if a < b), "{rows:?}");
    assert!(s.list.first().is_some_and(|m| m.flags.flagged), "flagged first");
    assert!(s.run("view.arrange", &json!({"by": "colour"})).is_err());
    s.run("view.arrange", &json!({"by": "date"})).unwrap();
    assert_eq!(s.rows()[0], Row::Header("Today"));
}

/// Send / Receive › Work Offline: ahead of anything queued, kept in the settings, and nothing
/// connects meanwhile.
#[test]
fn working_offline_comes_first_and_stops_send_receive() {
    let store = temp("offline");
    let mut s = Session::open(Some(store), mailcraft_model::dates::Clock::new(NOW, 0));
    s.run("account.add", &json!({"email": "zoe@example.org", "password": "pw"})).unwrap();
    s.run("mail.workOffline", &json!({})).unwrap();
    assert!(s.prefs.work_offline);
    assert!(s.run("mail.sync", &json!({})).is_err(), "nothing connects");
    assert!(matches!(s.take_jobs().first(), Some(Job::WorkOffline { on: true })), "before the account starts");
    s.run("mail.workOffline", &json!({"on": false})).unwrap();
    assert!(matches!(s.take_jobs().as_slice(), [Job::WorkOffline { on: false }]));
    assert!(s.run("mail.sync", &json!({})).is_ok());
    let _ = std::fs::remove_dir_all(s.store.as_ref().unwrap().root());
}

#[test]
fn delay_delivery_holds_mail_in_the_outbox_until_its_time() {
    let mut s = Session::open(Some(temp("delay")), mailcraft_model::dates::Clock::new(NOW, 0));
    s.run("account.add", &json!({"email": "zoe@example.org", "password": "pw"})).unwrap();
    s.take_jobs();
    s.run("mail.new", &json!({})).unwrap();
    s.run(
        "compose.update",
        &json!({"to": "ann@example.org", "subject": "Later", "body": "Hi", "replyTo": "Desk <desk@example.org>", "sendAfter": NOW + 3600}),
    )
    .unwrap();
    assert!(s.run("compose.update", &json!({"replyTo": "not an address"})).is_ok());
    assert!(s.run("compose.send", &json!({})).is_err(), "a bad Replies to address is caught");
    s.run("compose.update", &json!({"replyTo": "Desk <desk@example.org>"})).unwrap();
    assert_eq!(s.run("compose.send", &json!({})).unwrap()["sendAfter"], NOW + 3600);
    assert!(s.compose.is_none());
    assert!(!s.take_jobs().iter().any(|j| matches!(j, Job::Send { .. })), "held back, not sent");
    let (account, item) = s.outbox.first().cloned().unwrap();
    assert_eq!(item.not_before, NOW + 3600);
    let raw = s.store.as_ref().unwrap().outbox_raw(&account, item.id).unwrap();
    assert!(String::from_utf8_lossy(&raw).contains("Reply-To: \"Desk\" <desk@example.org>"));
    // Not yet: nothing asks for it. Its time: the account is asked, once.
    s.tick();
    assert!(s.take_jobs().is_empty());
    s.set_clock(NOW + 3601, 0);
    s.tick();
    s.tick();
    assert_eq!(s.take_jobs().iter().filter(|j| matches!(j, Job::FlushOutbox { .. })).count(), 1);
    // Send now sends a held-back message at once.
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"to": "ann@example.org", "subject": "Tomorrow", "body": "x", "sendAfter": NOW + 86_400})).unwrap();
    s.run("compose.send", &json!({})).unwrap();
    s.run("folder.outbox", &json!({})).unwrap();
    let k = s.list.iter().find(|m| m.subject == "Tomorrow").unwrap().key.clone();
    assert!(s.list.iter().find(|m| m.subject == "Tomorrow").unwrap().preview.starts_with("Will be sent"));
    s.run("outbox.send", &json!({"key": k.to_id()})).unwrap();
    assert!(s.outbox.iter().all(|(_, i)| i.not_before == 0 || i.not_before <= NOW + 3601));
    assert!(s.take_jobs().iter().any(|j| matches!(j, Job::FlushOutbox { .. })));
    done(s);
}

/// A new message to someone (a contact card's Send email): To filled in, ready for more.
#[test]
fn new_mail_to_someone() {
    let mut s = sample_session("new-mail-to");
    s.run("mail.new", &json!({"to": "Zoe Park <zoe@studio.example>"})).unwrap();
    assert_eq!(s.compose.as_ref().unwrap().to, "Zoe Park <zoe@studio.example>; ");
    s.compose = None;
    assert!(s.run("mail.new", &json!({"to": "not an address"})).is_err());
    assert!(s.compose.is_none(), "nothing opens for a bad address");
    s.run("mail.new", &json!({"to": "  "})).unwrap();
    assert_eq!(s.compose.as_ref().unwrap().to, "");
    done(s);
}

/// Several messages written at once, as Outlook keeps every New Email window: starting another
/// sets the one being written aside (in a window of its own), never drops it; an empty one goes.
#[test]
fn new_mail_keeps_the_message_being_written() {
    let mut s = sample_session("several-composes");
    let first = s.run("mail.new", &json!({})).unwrap()["compose"].as_u64().unwrap();
    // Nothing typed yet: a second New mail replaces it.
    let second = s.run("mail.new", &json!({})).unwrap()["compose"].as_u64().unwrap();
    assert!(s.composes.is_empty(), "an empty message isn't kept");
    assert_ne!(first, second);
    s.run("compose.update", &json!({"to": "ana@soundlab.example", "subject": "One"})).unwrap();
    let third = s.run("mail.new", &json!({})).unwrap()["compose"].as_u64().unwrap();
    s.run("compose.update", &json!({"subject": "Two", "body": "Second text"})).unwrap();
    // A reply too: both written ones stay.
    let k = key_of(&s, "Sound mix, version 3");
    s.run("list.open", &k).unwrap();
    s.run("mail.reply", &json!({})).unwrap();
    let list = s.run("compose.list", &json!({})).unwrap();
    let subjects: Vec<String> = list.as_array().unwrap().iter().map(|c| c["subject"].as_str().unwrap().to_string()).collect();
    assert_eq!(subjects.len(), 3, "{subjects:?}");
    assert!(subjects.contains(&"One".into()) && subjects.contains(&"Two".into()));
    assert!(s.composes.iter().all(|c| c.window), "set aside in windows of their own");
    // Back to the first: edits go to it; the reply is set aside in turn.
    s.run("compose.activate", &json!({"id": second})).unwrap();
    assert_eq!(s.compose.as_ref().unwrap().subject, "One");
    s.run("compose.update", &json!({"subject": "One, changed"})).unwrap();
    assert!(s.composes.iter().any(|c| c.id == third && c.subject == "Two"));
    // Closed (kept in Drafts): the others stay.
    s.run("compose.close", &json!({})).unwrap();
    assert_eq!(s.all_composes().count(), 2);
    assert!(s.run("compose.activate", &json!({"id": second})).is_err(), "closed");
    assert!(s.run("compose.activate", &json!({})).is_err());
    assert!(s.activate_compose(third));
    assert_eq!(s.compose.as_ref().unwrap().subject, "Two");
}

/// Closing with messages written: each one with something in it is kept as a draft (on this
/// computer at once; a server account's goes to Drafts at the next connection).
#[test]
fn closing_keeps_every_message_being_written() {
    let mut s = sample_session("keep-on-close");
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"subject": "Kept one"})).unwrap();
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"subject": "Kept two"})).unwrap();
    s.run("mail.new", &json!({})).unwrap(); // nothing written in this one
    assert_eq!(s.run("compose.keepAll", &json!({})).unwrap()["kept"], 2);
    let drafts = s.account("sample").and_then(|a| a.folder(FolderRole::Drafts)).map(|f| f.path.clone()).unwrap();
    let subjects: Vec<String> = s.folder_state("sample", &drafts).messages.iter().map(|m| m.subject.clone()).collect();
    assert!(subjects.contains(&"Kept one".into()) && subjects.contains(&"Kept two".into()), "{subjects:?}");
    // A server account's: handed to the network side, kept before MailCraft goes.
    s.accounts[0].config.imap.host = "imap.studio.example".into();
    s.take_jobs();
    s.activate_compose(s.composes[0].id);
    s.keep_drafts();
    assert!(s.take_jobs().iter().any(|j| matches!(j, mailcraft_model::Job::KeepDraft { .. })));
}

/// Attaching from a path reads a plain file only: a folder, a device that reads forever
/// (`/dev/zero`, which reports a size of 0) or a pipe that blocks are refused, at once.
#[test]
fn attaching_reads_plain_files_only() {
    let mut s = sample_session("attach-paths");
    s.run("mail.new", &json!({})).unwrap();
    let dir = s.store.as_ref().unwrap().root().to_path_buf();
    let e = s.run("compose.attach", &json!({"path": dir.to_string_lossy()})).unwrap_err().to_string();
    assert!(e.contains("isn't a file"), "{e}");
    let device = if cfg!(windows) { r"\.\NUL" } else { "/dev/zero" };
    let started = std::time::Instant::now();
    assert!(s.run("compose.attach", &json!({"path": device})).is_err());
    assert!(started.elapsed() < std::time::Duration::from_secs(5), "refused, not read");
    let file = dir.join("ok.txt");
    std::fs::write(&file, b"hi").unwrap();
    s.run("compose.attach", &json!({"path": file.to_string_lossy()})).unwrap();
    assert_eq!(s.compose.as_ref().map(|c| c.attachments.len()), Some(1));
    done(s);
}

/// Issue #11 (#16's test): a signature can hold pictures. Kept in its HTML with the account, they
/// go into new messages (each under a name of the message's own) and are sent with them; a
/// signature put in again doesn't add them twice.
#[test]
fn a_signature_holds_pictures() {
    use crate::session::Compose;
    use mailcraft_mime::{self as mime, rich::Styles};
    let mut s = sample_session("signature-pictures");
    let png = sample::sample_png();
    // The signature editor's text: a logo before the name.
    let mut sig = Compose { body: "Riley".into(), styles: Styles::plain("Riley"), ..Default::default() };
    sig.inline.push(mime::InlinePicture { cid: "logo@x".into(), name: "logo.png".into(), mime: "image/png".into(), data: png.clone() });
    crate::cmd::insert_picture_at(&mut sig, 0, "logo@x", None);
    assert_eq!(sig.body.as_str(), "\u{FFFC}Riley");
    let html = mime::embed_pictures(&mime::rich::to_html(&sig.body, &sig.styles), &sig.inline);
    s.run("account.setSignature", &json!({"account": "sample", "signature": "Riley", "html": html})).unwrap();
    let config = s.account("sample").unwrap().config.clone();
    assert!(config.signature_html.contains("src=\"data:image/png;base64,"), "{}", config.signature_html);
    assert_eq!(config.signature, "Riley", "the plain-text twin has no picture");

    s.run("mail.new", &json!({})).unwrap();
    let c = s.compose.clone().unwrap();
    let at = c.body.find('\u{FFFC}').map(|b| c.body[..b].chars().count()).expect("the logo is in the message");
    let cid = c.styles.style(at).picture.expect("as a picture");
    assert!(cid.starts_with("signature1.") && cid.ends_with("@mailcraft"), "{cid}");
    assert_eq!(c.inline.iter().filter(|p| p.cid == cid).map(|p| &p.data).collect::<Vec<_>>(), [&png]);
    s.run("compose.insertSignature", &json!({})).unwrap();
    let c = s.compose.clone().unwrap();
    assert_eq!((c.body.matches('\u{FFFC}').count(), c.inline.len()), (1, 1), "put in again: replaced, not added twice");

    // Sent with the message: a part of its own, which the HTML shows.
    s.run("compose.update", &json!({"to": "ann@x.example", "subject": "With the logo"})).unwrap();
    s.run("compose.send", &json!({})).unwrap();
    s.run("folder.open", &json!({"account": "sample", "folder": "Sent"})).unwrap();
    let k = s.list.iter().find(|m| m.subject == "With the logo").map(|m| m.key.clone()).unwrap();
    let raw = s.store.as_ref().unwrap().read_body(&k).unwrap();
    let p = mime::parse(&raw).unwrap();
    let logo = p.attachments.iter().find(|a| a.content_id.as_deref() == Some(cid.as_str())).expect("the logo goes with it");
    assert_eq!(mime::attachment_data(&raw, logo.index), Some(png));
    assert!(p.images.contains(&format!("cid:{cid}")), "{:?}", p.images);
}

/// A wide picture goes in on a line of its own (issue #10: a second one after it went past the
/// editor's edge), the cursor after it; on an empty line already, no lines are added.
#[test]
fn wide_pictures_go_on_a_line_of_their_own() {
    let mut s = sample_session("picture-line");
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"body": "the frames are in\n\nthanks"})).unwrap();
    let before = s.compose.as_ref().unwrap().body.clone();
    let at = before.find("frames").map(|b| before[..b].chars().count() + 3).unwrap();
    let r = s.run("compose.insertPicture", &json!({"name": "shot.png", "data": crate::sample::sample_png(), "at": at, "line": true})).unwrap();
    let body = s.compose.as_ref().unwrap().body.clone();
    assert!(body.contains("fra\n\u{FFFC}\nmes"), "{body:?}");
    assert_eq!(r["end"].as_u64(), Some(at as u64 + 2));
    let shot = r["cid"].as_str().unwrap().to_string();
    let c = s.compose.as_ref().unwrap();
    assert_eq!(c.styles.style(at + 1).picture.as_deref(), Some(shot.as_str()));
    assert!(c.styles.style(at).picture.is_none() && c.styles.style(at + 2).picture.is_none(), "the line breaks aren't pictures");
    // At the end, after text: a line break before it only.
    let n = c.body.chars().count();
    let r = s.run("compose.insertPicture", &json!({"name": "end.png", "data": crate::sample::sample_png(), "at": n, "line": true})).unwrap();
    assert_eq!(r["end"].as_u64(), Some(n as u64 + 2));
    let body = s.compose.as_ref().unwrap().body.clone();
    assert!(body.ends_with("\n\u{FFFC}"), "{body:?}");
    // On an empty line already: no lines added.
    let empty = body.find("\n\n").map(|b| body[..b].chars().count() + 1).unwrap();
    let n = body.chars().count();
    s.run("compose.insertPicture", &json!({"name": "in.png", "data": crate::sample::sample_png(), "at": empty, "line": true})).unwrap();
    assert_eq!(s.compose.as_ref().unwrap().body.chars().count(), n + 1, "the picture alone");
}

/// Issue #11: a signature keeps its pictures: saved with it, in new messages and replies with
/// it, and sent with the message; the plain text one has none.
#[test]
fn a_signature_has_pictures() {
    let mut s = sample_session("signature-picture");
    let png = crate::sample::sample_png();
    let pic = mailcraft_mime::InlinePicture { cid: "logo@x".into(), name: "logo.png".into(), mime: "image/png".into(), data: png.clone() };
    let html = mailcraft_mime::embed_pictures("<p><b>Riley</b> <img src=\"cid:logo@x\" width=\"96\"></p>", &[pic]);
    s.run("account.setSignature", &json!({"account": "sample", "signature": "Riley", "html": html})).unwrap();
    let a = s.account("sample").unwrap();
    assert!(a.config.signature_html.contains("data:image/png;base64,"), "kept with it");
    assert_eq!(a.config.signature, "Riley ", "its text has no picture mark");
    s.run("mail.new", &json!({})).unwrap();
    let c = s.compose.as_ref().unwrap();
    let at = c.body.chars().position(|ch| ch == mailcraft_mime::rich::PICTURE).expect("the picture in the signature");
    let style = c.styles.style(at);
    assert_eq!(style.picture_width, Some(96));
    assert_eq!(c.inline.iter().map(|p| p.data.clone()).collect::<Vec<_>>(), vec![png.clone()]);
    s.run("compose.update", &json!({"to": "ann@x.example", "subject": "Logo"})).unwrap();
    s.run("compose.send", &json!({})).unwrap();
    s.run("folder.open", &json!({"account": "sample", "folder": "Sent"})).unwrap();
    let k = key_of(&s, "Logo");
    s.run("list.open", &k).unwrap();
    let p = s.reading.as_ref().unwrap().parsed.clone().unwrap();
    assert_eq!(p.images.len(), 1, "sent with the message");
    assert!(p.attachments.iter().any(|a| a.inline && a.mime == "image/png"));
    // Insert › Signature in a reply brings it too.
    s.run("mail.reply", &json!({})).unwrap();
    assert!(s.compose.as_ref().unwrap().inline.len() == 1);
    done(s);
}

/// Issue #11: replies and forwards can open in a window of their own each time (they start in
/// the reading pane, as Outlook's); new messages keep their own setting.
#[test]
fn replies_can_open_in_a_window_of_their_own() {
    let mut s = sample_session("reply-window");
    s.run("view.newMailWindow", &json!({"on": false})).unwrap();
    let first = s.list.first().map(|m| m.subject.clone()).unwrap();
    s.run("list.open", &key_of(&s, &first)).unwrap();
    s.run("mail.reply", &json!({})).unwrap();
    assert!(!s.compose.as_ref().unwrap().window, "in the reading pane, as before");
    s.run("compose.discard", &json!({})).unwrap();
    assert_eq!(s.run("view.replyWindow", &json!({})).unwrap()["on"], json!(true));
    for cmd in ["mail.reply", "mail.replyAll", "mail.forward"] {
        s.run(cmd, &json!({})).unwrap();
        assert!(s.compose.as_ref().unwrap().window, "{cmd}: a window of its own");
        s.run("compose.discard", &json!({})).unwrap();
    }
    s.run("mail.new", &json!({})).unwrap();
    assert!(!s.compose.as_ref().unwrap().window, "a new message as its own setting says");
    assert_eq!(s.run("view.favorites", &json!({"on": false})).unwrap()["on"], json!(false));
    assert!(!s.prefs.show_favorites && s.prefs.reply_window);
    done(s);
}
