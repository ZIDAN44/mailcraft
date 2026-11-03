//! Issue #39: reactions (RFC 9078), sent from the message shown and gathered under it.

use mailcraft_model::MessageKey;

use super::*;

/// Put a message in the sample's Inbox, its source kept.
fn put(s: &mut Session, uid: u32, raw: &[u8]) -> MessageKey {
    let store = s.store.clone().unwrap();
    let mut inbox = store.load_folder("sample", "INBOX").unwrap();
    let key = MessageKey { account: "sample".into(), folder: "INBOX".into(), validity: inbox.validity, uid };
    inbox.messages.insert(0, mailcraft_mime::summary_from_full(raw, key.clone(), mailcraft_model::Flags::default(), 0, NOW - 3600).into());
    store.save_folder("sample", "INBOX", &inbox).unwrap();
    store.write_body(&key, raw).unwrap();
    s.apply(Event::FolderChanged { account: "sample".into(), folder: "INBOX".into() });
    key
}

fn reacted(from: &str, emoji: &str, id: u32) -> Vec<u8> {
    format!(
        "From: {from}\r\nTo: me@x\r\nSubject: Re: Launch plan\r\nMessage-ID: <react-{id}@studio.test>\r\nIn-Reply-To: <plan-1@studio.test>\r\n\
         Content-Type: text/plain; charset=utf-8\r\nContent-Disposition: reaction\r\n\r\n{emoji}\r\n"
    )
    .into_bytes()
}

/// Reacting to the message shown sends the emoji as a reaction to its sender (here, an account
/// with no server, it's filed in Sent Items); the reactions that answer it are gathered by
/// emoji, the most sent first, the user's own marked; words aren't a reaction.
#[test]
fn reactions_are_sent_and_gathered_under_the_message() {
    let mut s = sample_session("reactions");
    let me = s.accounts[0].config.email.clone();
    let plan = format!(
        "From: Sam Lee <sam@studio.test>\r\nTo: {me}, Ben <ben@studio.test>\r\nSubject: Launch plan\r\nMessage-ID: <plan-1@studio.test>\r\n\r\nHere's the plan.\r\n"
    );
    let key = put(&mut s, 9601, plan.as_bytes());
    put(&mut s, 9602, &reacted("Ben Ode <ben@studio.test>", "❤️", 2));
    put(&mut s, 9603, &reacted("Kim <kim@studio.test>", "👍", 3));
    put(&mut s, 9604, &reacted("Kim <kim@studio.test>", "👍", 4));
    put(&mut s, 9605, &reacted("Lou <lou@studio.test>", "Great, thanks", 5));
    s.run("folder.open", &json!({"account": "sample", "folder": "INBOX"})).unwrap();
    s.run("list.open", &json!({"key": key.to_id()})).unwrap();
    assert!(s.run("mail.react", &json!({"emoji": "ok"})).is_err(), "only emoji");
    let r = s.run("mail.react", &json!({"emoji": "👍"})).unwrap();
    assert_eq!(r["to"], json!(["sam@studio.test", "ben@studio.test"]));
    let sent = s.account("sample").and_then(|a| a.folder(mailcraft_model::FolderRole::Sent)).map(|f| f.path.clone()).unwrap();
    let kept = s.store.clone().unwrap().load_folder("sample", &sent).unwrap();
    let mine = kept.messages.iter().find(|m| m.reaction == "👍").expect("filed in Sent Items");
    assert_eq!(mine.in_reply_to, "plan-1@studio.test");
    s.run("folder.open", &json!({"account": "sample", "folder": "INBOX"})).unwrap();
    s.run("folder.open", &json!({"account": "sample", "folder": sent})).unwrap();
    s.run("folder.open", &json!({"account": "sample", "folder": "INBOX"})).unwrap();
    let got: Vec<(String, usize, bool)> = s.reactions(&key).into_iter().map(|r| (r.emoji, r.people.len(), r.mine)).collect();
    assert_eq!(got, [("👍".to_string(), 2, true), ("❤️".to_string(), 1, false)], "Kim's twice is once; Lou's words aren't one");
    // Nothing for a message no one reacted to, or with no id.
    let other = put(&mut s, 9606, b"From: z@studio.test\r\nSubject: Quiet\r\n\r\nx\r\n");
    assert!(s.reactions(&other).is_empty());
}

/// No reaction to a list's message (it would go to the whole list).
#[test]
fn no_reaction_to_a_list() {
    let mut s = sample_session("reactions-list");
    let key = put(
        &mut s,
        9611,
        b"From: News <news@lists.example>\r\nTo: list@lists.example\r\nList-Id: <news.lists.example>\r\nSubject: Weekly\r\nMessage-ID: <w@lists.example>\r\n\r\nIssue 12.\r\n",
    );
    s.run("folder.open", &json!({"account": "sample", "folder": "INBOX"})).unwrap();
    s.run("list.open", &json!({"key": key.to_id()})).unwrap();
    assert!(s.run("mail.react", &json!({"emoji": "👍"})).is_err());
    assert!(!s.take_jobs().iter().any(|j| matches!(j, Job::Send { .. })));
}
