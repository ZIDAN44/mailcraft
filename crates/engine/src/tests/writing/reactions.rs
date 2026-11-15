//! Issue #39: reactions (RFC 9078), sent from the message shown and gathered under it.

use mailcraft_model::MessageKey;

use super::*;

/// Put a message in the sample's Inbox, its source kept.
fn put(s: &mut Session, uid: u32, raw: &[u8]) -> MessageKey {
    let key = MessageKey { uid, ..next_key(s, "sample", "INBOX") };
    let summary = mailcraft_mime::summary_from_full(raw, key, mailcraft_model::Flags::default(), 0, NOW - 3600);
    file_in(s, summary, Some(raw))
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

/// What gathering a message's reactions takes with a list of 50,000 and a Sent Items of 20,000
/// cached, every one an answer to something (`cargo test -p mailcraft-engine --release
/// measure_reactions_of_a_big_list -- --ignored --nocapture`): the reading pane asks every frame.
#[test]
#[ignore = "a measurement: run in release"]
fn measure_reactions_of_a_big_list() {
    let store = temp("reactions-measured");
    sample::seed_if_empty(&store, NOW).unwrap();
    let sent = "Sent";
    let fill = |folder: &str, n: u32| {
        let mut st = store.load_folder("sample", folder).unwrap();
        st.validity = 7;
        st.messages = (1..=n)
            .map(|uid| mailcraft_model::Summary {
                key: MessageKey { account: "sample".into(), folder: folder.into(), validity: 7, uid },
                subject: format!("Re: Report {uid}"),
                message_id: format!("m{uid:08}.{folder}@studio.example"),
                in_reply_to: format!("m{:08}.{folder}@studio.example", uid + 1),
                date: NOW - i64::from(uid) * 60,
                ..Default::default()
            })
            .map(Into::into)
            .collect();
        store.save_folder("sample", folder, &st).unwrap();
    };
    fill("Projects", 50_000);
    fill(sent, 20_000);
    let mut s = Session::open(Some(store), mailcraft_model::dates::Clock::new(NOW, 0));
    s.run("folder.open", &json!({"account": "sample", "folder": sent})).unwrap();
    s.run("folder.open", &json!({"account": "sample", "folder": "Projects"})).unwrap();
    assert!(s.cache.contains_key(&("sample".to_string(), sent.to_string())), "Sent Items is cached");
    let key = s.list[100].key.clone();
    let frames = 600u32;
    let began = std::time::Instant::now();
    for _ in 0..frames {
        assert!(s.reactions(&key).is_empty());
    }
    let gathered = began.elapsed();
    // What the window does each frame: the stamp, and gathering them again only when it moved.
    let stamp = s.reactions_stamp(&key);
    let began = std::time::Instant::now();
    for _ in 0..frames {
        assert_eq!(s.reactions_stamp(&key), stamp);
    }
    let stamped = began.elapsed();
    println!(
        "MEASURE reactions of one message, list 50000 + Sent 20000: gathered {} us a frame, stamp {} ns a frame",
        gathered.as_micros() / u128::from(frames),
        stamped.as_nanos() / u128::from(frames)
    );
}

/// What reactions are gathered from moves with what can change them: a reaction filed in Sent
/// Items, another message in the list; and not when nothing did (the window keeps what it found
/// until then).
#[test]
fn the_reactions_stamp_moves_with_what_they_are_gathered_from() {
    let mut s = sample_session("reactions-stamp");
    let plan = "From: Sam Lee <sam@studio.test>\r\nTo: me@x\r\nSubject: Launch plan\r\nMessage-ID: <plan-1@studio.test>\r\n\r\nHere's the plan.\r\n";
    let key = put(&mut s, 9621, plan.as_bytes());
    s.run("folder.open", &json!({"account": "sample", "folder": "Sent"})).unwrap();
    s.run("folder.open", &json!({"account": "sample", "folder": "INBOX"})).unwrap();
    s.run("list.open", &json!({"key": key.to_id()})).unwrap();
    let stamp = s.reactions_stamp(&key);
    assert_eq!(s.reactions_stamp(&key), stamp, "nothing changed");
    assert!(s.reactions(&key).is_empty());
    s.run("mail.react", &json!({"emoji": "👍"})).unwrap();
    let after = s.reactions_stamp(&key);
    assert_ne!(after, stamp, "a reaction filed in Sent Items");
    assert_eq!(s.reactions(&key).len(), 1);
    put(&mut s, 9622, &reacted("Kim <kim@studio.test>", "❤️", 22));
    s.run("folder.open", &json!({"account": "sample", "folder": "INBOX"})).unwrap();
    assert_ne!(s.reactions_stamp(&key), after, "one in the list");
    assert_eq!(s.reactions(&key).len(), 2);
}

/// A reaction in Sent Items listed before its source came (what it answers and its emoji not
/// known yet), then filled from it in place: the stamp moves, though the folder holds the same
/// messages under the same numbers (the window kept showing none).
#[test]
fn the_reactions_stamp_moves_when_a_sent_reaction_is_filled_in() {
    let mut s = sample_session("reactions-filled");
    let plan = "From: Sam Lee <sam@studio.test>\r\nTo: me@x\r\nSubject: Launch plan\r\nMessage-ID: <plan-1@studio.test>\r\n\r\nHere's the plan.\r\n";
    let key = put(&mut s, 9631, plan.as_bytes());
    let me = s.accounts[0].config.email.clone();
    let sent = MessageKey { uid: 9632, ..next_key(&s, "sample", "Sent") };
    let whole = mailcraft_mime::summary_from_full(&reacted(&me, "👍", 32), sent, mailcraft_model::Flags::default(), 0, NOW - 60);
    let listed = mailcraft_model::Summary { in_reply_to: String::new(), references: Vec::new(), reaction: String::new(), ..whole.clone() };
    file_in(&mut s, listed, None);
    s.run("folder.open", &json!({"account": "sample", "folder": "Sent"})).unwrap();
    s.run("folder.open", &json!({"account": "sample", "folder": "INBOX"})).unwrap();
    let stamp = s.reactions_stamp(&key);
    assert!(s.reactions(&key).is_empty());
    s.filled(whole);
    assert_eq!(s.reactions(&key).len(), 1);
    assert_ne!(s.reactions_stamp(&key), stamp);
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
