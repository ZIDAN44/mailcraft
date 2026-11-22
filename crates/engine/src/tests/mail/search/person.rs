//! Engine tests: everything with one person (issue #178).

use super::*;

use mailcraft_model::{EmailAddress, Folder, MessageKey, Summary};

fn addr(name: &str, email: &str) -> EmailAddress {
    EmailAddress { name: name.into(), email: email.into() }
}

/// One account (`me@studio.example`) whose mail with Pat Example is: their message in the Inbox
/// (a file, a link and a signature), a copy of it in the Archive, the user's answer in Sent with
/// Lee on copy, and their offer in Junk; and a message with no one of them in it.
fn pats_mailbox() -> Session {
    let store = Store::memory();
    store.save_accounts(&[mailcraft_model::AccountConfig { id: "a1".into(), email: "me@studio.example".into(), ..Default::default() }]).unwrap();
    let folder = |path: &str, role: FolderRole| Folder { account: "a1".into(), path: path.into(), role, selectable: true, ..Default::default() };
    store
        .save_folders(
            "a1",
            &[
                folder("INBOX", FolderRole::Inbox),
                folder("Sent", FolderRole::Sent),
                folder("Archive", FolderRole::Archive),
                folder("Junk", FolderRole::Junk),
            ],
        )
        .unwrap();
    let pat = addr("Pat Example", "Pat@Far.example");
    let me = addr("Me", "me@studio.example");
    let theirs = "From: Pat Example <Pat@Far.example>\r\nTo: me@studio.example\r\nSubject: The cut\r\nMessage-ID: <p1@far.example>\r\n\
        MIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=\"b\"\r\n\r\n--b\r\nContent-Type: text/plain\r\n\r\n\
        The second cut is at https://share.example/cut-2 for you.\r\n\r\n-- \r\nPat Example\r\nTel +44 20 7946 0958\r\nhttps://pat.example/portfolio\r\n\
        --b\r\nContent-Type: application/pdf; name=\"notes.pdf\"\r\nContent-Disposition: attachment; filename=\"notes.pdf\"\r\nContent-Transfer-Encoding: base64\r\n\r\nJVBERi0xLjQK\r\n--b--\r\n";
    let answer = "From: Me <me@studio.example>\r\nTo: Pat Example <pat@far.example>\r\nCc: Lee <lee@far.example>\r\nSubject: Re: The cut\r\n\r\nLooks good, Pat.\r\n";
    let summary = |folder: &str, uid: u32, id: &str, subject: &str, from: EmailAddress, to: Vec<EmailAddress>, ago: i64| Summary {
        key: MessageKey { account: "a1".into(), folder: folder.into(), validity: 1, uid },
        message_id: id.into(),
        subject: subject.into(),
        from,
        to,
        date: NOW - ago,
        has_attachments: subject == "The cut",
        ..Default::default()
    };
    let mut answer_summary = summary("Sent", 1, "p2@studio.example", "Re: The cut", me.clone(), vec![addr("Pat Example", "pat@far.example")], 60);
    answer_summary.in_reply_to = "p1@far.example".into();
    answer_summary.cc = vec![addr("Lee", "lee@far.example")];
    let folders: Vec<(&str, Vec<(Summary, Option<&str>)>)> = vec![
        (
            "INBOX",
            vec![
                (summary("INBOX", 1, "p1@far.example", "The cut", pat.clone(), vec![me.clone()], 3_600), Some(theirs)),
                (summary("INBOX", 2, "x1@else.example", "Lunch", addr("Kim", "kim@else.example"), vec![me.clone()], 120), None),
            ],
        ),
        ("Sent", vec![(answer_summary, Some(answer))]),
        ("Archive", vec![(summary("Archive", 1, "p1@far.example", "The cut", pat.clone(), vec![me.clone()], 3_600), Some(theirs))]),
        ("Junk", vec![(summary("Junk", 1, "j1@far.example", "A prize", pat, vec![me], 30), None)]),
    ];
    for (path, list) in folders {
        let mut st = mailcraft_store::FolderState { validity: 1, ..Default::default() };
        for (m, raw) in list {
            if let Some(raw) = raw {
                store.write_body(&m.key, raw.as_bytes()).unwrap();
            }
            st.messages.push(m.into());
        }
        mailcraft_model::sort_newest_first(&mut st.messages);
        store.save_folder("a1", path, &st).unwrap();
    }
    let mut s = Session::open(Some(store), mailcraft_model::dates::Clock::new(NOW, 0));
    s.take_jobs();
    s
}

/// Their mail in every folder but Junk, once each, by conversation; the file, the links and the
/// signature's details from what's on this computer; the others on it, never the user.
#[test]
fn everything_with_someone_from_their_mail() {
    let mut s = pats_mailbox();
    let a = s.person_activity(" PAT@far.example ");
    assert_eq!(a.email, "pat@far.example");
    assert_eq!(a.name, "Pat Example", "the name their mail gives, with no contact");
    assert_eq!(a.contact, None);
    assert_eq!(a.message_count(), 2, "the Archive's copy once, the Junk not, Kim's lunch not");
    assert_eq!(a.threads.len(), 1);
    let subjects: Vec<&str> = a.threads[0].messages.iter().map(|m| m.subject.as_str()).collect();
    assert_eq!(subjects, ["Re: The cut", "The cut"], "the newest first");
    assert_eq!(a.threads[0].subject, "The cut");
    let files: Vec<(&str, bool)> = a.files.iter().map(|f| (f.name.as_str(), f.from_them)).collect();
    assert_eq!(files, [("notes.pdf", true)]);
    assert_eq!(a.files[0].key.folder, "INBOX");
    let links: Vec<&str> = a.links.iter().map(|l| l.url.as_str()).collect();
    assert_eq!(links, ["https://share.example/cut-2", "https://pat.example/portfolio"]);
    assert_eq!(a.found.phones, ["+44 20 7946 0958"]);
    assert_eq!(a.found.websites, ["https://pat.example/portfolio"]);
    let people: Vec<(&str, usize)> = a.people.iter().map(|p| (p.email.as_str(), p.count)).collect();
    assert_eq!(people, [("lee@far.example", 1)], "not the user, not them");
    assert_eq!(a.not_downloaded, 0);
    assert!(!a.cut_short);

    // Narrowed: the mail as a search does, the rest by words.
    let r = a.refined("from:pat");
    assert_eq!(r.message_count(), 1);
    assert_eq!(r.threads[0].messages[0].subject, "The cut");
    let r = a.refined("portfolio");
    assert_eq!(r.links.len(), 1);
    assert!(r.files.is_empty() && r.people.is_empty());
    assert_eq!(r.message_count(), 1, "the message the link is in");
    assert_eq!(a.refined("  ").message_count(), 2, "an empty search narrows nothing");
    // A word of a message's text its summary doesn't have: found in the text read; a file's name.
    let r = a.refined("second");
    assert_eq!((r.message_count(), r.threads[0].messages[0].subject.as_str()), (1, "The cut"));
    assert_eq!(a.refined("notes").message_count(), 1, "by its file's name");
    assert_eq!(a.refined("nowhere-word").message_count(), 0);

    // Someone with no mail, and no address at all.
    let none = s.person_activity("nobody@nowhere.example");
    assert_eq!((none.message_count(), none.name.as_str()), (0, "nobody@nowhere.example"));
    assert_eq!(s.person_activity("not an address").message_count(), 0);
}

/// In the sample mailbox: Zoe's two conversations (one with Sam's answer, one in the Archive),
/// her file, the meeting she organises, Sam beside her; her contact, whose phone isn't offered
/// again.
#[test]
fn everything_with_zoe_in_the_sample() {
    let mut s = sample_session("person_zoe");
    let a = s.person_activity("Zoe@Studio.example");
    assert_eq!(a.name, "Zoe Park");
    assert!(a.contact.is_some(), "her contact");
    let subjects: Vec<&str> = a.threads.iter().map(|t| t.subject.as_str()).collect();
    assert_eq!(subjects, ["Storyboard review on Thursday", "Kick-off notes"]);
    assert_eq!(a.threads[0].messages.len(), 2, "Sam's answer to her in her conversation");
    assert!(a.files.iter().any(|f| f.name == "colour-key.png" && f.from_them));
    let meetings: Vec<(&str, bool)> = a.meetings.iter().map(|m| (m.summary.as_str(), m.theirs)).collect();
    assert_eq!(meetings, [("Storyboard review", true)]);
    let people: Vec<&str> = a.people.iter().map(|p| p.email.as_str()).collect();
    assert_eq!(people, ["sam@studio.example"], "not Riley, the user");
    assert!(a.found.phones.is_empty());

    // Sam is a guest of that meeting, not its organiser.
    let sam = s.person_activity("sam@studio.example");
    assert!(sam.meetings.iter().any(|m| m.summary == "Storyboard review" && !m.theirs));
    assert!(a.refined("kick-off").threads.iter().all(|t| t.subject == "Kick-off notes"));
    // The end of her message's text, past its preview.
    let r = a.refined("consistent");
    assert_eq!(r.message_count(), 1);
    assert_eq!(r.threads[0].messages[0].from.email, "zoe@studio.example");
    assert_eq!(a.refined("kick-off").threads.len(), 1);
}

/// The command: each part's first `limit`, its count; an address it can't be is refused.
#[test]
fn the_person_command() {
    let mut s = sample_session("person_cmd");
    let r = s.run("person.activity", &json!({"email": "zoe@studio.example", "limit": 1})).unwrap();
    assert_eq!(r["name"], "Zoe Park");
    assert_eq!(r["messages"], 3);
    assert_eq!(r["threadCount"], 2);
    assert_eq!(r["threads"].as_array().unwrap().len(), 1, "the limit");
    assert_eq!(r["threads"][0]["subject"], "Storyboard review on Thursday");
    assert_eq!(r["meetings"][0]["theirs"], true);
    let r = s.run("person.activity", &json!({"email": "zoe@studio.example", "query": "kick-off"})).unwrap();
    assert_eq!(r["threadCount"], 1);
    assert!(matches!(s.run("person.activity", &json!({"email": "zoe"})), Err(CmdError::Params(_))));
    assert!(s.run("person.activity", &json!({})).is_err());
}

/// How long everything with someone takes on a mailbox the size of a real file's
/// ([`sample::big`]: 50,000 messages in 40 folders, on disk), for a sender of about a thousand of
/// them: gathered (on opening it), and narrowed (each letter typed in its search). Run in release
/// under the machine's lock: `cargo test --release -p mailcraft-engine --lib
/// measure_everything_with_someone -- --ignored --nocapture`.
#[test]
#[ignore = "a measurement: run in release"]
fn measure_everything_with_someone() {
    use std::time::Instant;
    let store = temp("person_big");
    sample::big::generate(&store, NOW, sample::big::Shape::default()).unwrap();
    let mut s = Session::open(Some(store), mailcraft_model::dates::Clock::new(NOW, 0));
    s.take_jobs();
    for round in 0..3 {
        let t = Instant::now();
        let a = s.person_activity("member0@studio.example");
        let gathered = t.elapsed();
        let t = Instant::now();
        let r = a.refined("thank");
        let narrowed = t.elapsed();
        println!(
            "MEASURE round {round}: gathered {} messages, {} conversations, {} files in {:.1} ms; narrowed to {} in {:.1} ms",
            a.message_count(),
            a.threads.len(),
            a.files.len(),
            gathered.as_secs_f64() * 1000.0,
            r.message_count(),
            narrowed.as_secs_f64() * 1000.0
        );
    }
}
