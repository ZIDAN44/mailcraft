//! Engine tests: categories, rules, junk lists, Clean Up, Search Folders and Quick Steps.

use super::*;

#[test]
fn every_command_survives_junk_params() {
    let mut s = sample_session("junk");
    let ids: Vec<&str> = s.registry().all().iter().map(|c| c.id).collect();
    let junk = [
        json!(null),
        json!({}),
        json!({"key": "x"}),
        json!({"key": 5, "folder": [], "account": {"a": 1}}),
        json!({"value": "sideways", "index": -1, "path": "", "data": [999, -4]}),
        json!({"query": "\"unterminated from: -", "scope": 3}),
        json!({"email": "@", "password": "", "imapPort": 99_999_999}),
        json!({"index": 1e300, "limit": -5, "on": "yes"}),
    ];
    for id in &ids {
        for p in &junk {
            let _ = s.run(id, p);
        }
        // Junk params again with a message open and a compose window up.
        let k = s.list.first().map(|m| m.key.clone());
        if let Some(k) = k {
            s.open_message(&k);
        }
        let _ = s.run("mail.new", &json!({}));
        for p in &junk {
            let _ = s.run(id, p);
        }
    }
    assert!(matches!(s.run("no.such", &json!({})), Err(CmdError::Unknown(_))));
    done(s);
}

#[test]
fn flags_filters_and_conversations() {
    let mut s = sample_session("flags");
    s.run("list.selectAll", &json!({})).unwrap();
    s.run("mail.markRead", &json!({})).unwrap();
    assert!(s.list.iter().all(|m| m.flags.seen));
    s.run("list.filter", &json!({"value": "unread"})).unwrap();
    assert!(s.list.is_empty());
    s.run("list.filter", &json!({"value": "all"})).unwrap();
    let k = key_of(&s, "Character turnarounds");
    s.run("list.select", &k).unwrap();
    s.run("mail.flag", &json!({})).unwrap();
    s.run("list.filter", &json!({"value": "flagged"})).unwrap();
    assert_eq!(s.list.len(), 2);
    assert_eq!(s.filter, ListFilter::Flagged);
    s.run("list.filter", &json!({"value": "attachments"})).unwrap();
    assert_eq!(s.list.len(), 4, "a picture, a proof, a turnaround and an invitation");
    s.run("list.filter", &json!({"value": "all"})).unwrap();

    // Follow Up: flagged with a day it's due; Mark Complete and unflagging forget it.
    let turnaround = |s: &Session| s.list.iter().find(|m| m.subject == "Character turnarounds").cloned().unwrap();
    s.run("list.select", &k).unwrap();
    let today = mailcraft_model::dates::today(&s.clock);
    s.run("mail.flag", &json!({"due": "tomorrow"})).unwrap();
    assert_eq!(s.follow_up_due(&turnaround(&s)), Some(today + 1));
    s.run("mail.flag", &json!({"due": "2026-12-01"})).unwrap();
    assert_eq!(s.follow_up_due(&turnaround(&s)).map(mailcraft_model::dates::day_string).as_deref(), Some("2026-12-01"));
    assert!(s.run("mail.flag", &json!({"due": "someday"})).is_err());
    s.run("mail.flag", &json!({"complete": true})).unwrap();
    assert!(!turnaround(&s).flags.flagged && s.prefs.follow_up.is_empty());
    s.run("mail.flag", &json!({"due": "thisWeek"})).unwrap();
    s.run("mail.flag", &json!({"on": false})).unwrap();
    assert!(s.follow_up_due(&turnaround(&s)).is_none() && s.prefs.follow_up.is_empty());
    s.run("mail.flag", &json!({"due": "none"})).unwrap();
    assert!(turnaround(&s).flags.flagged && s.follow_up_due(&turnaround(&s)).is_none());

    s.run("view.conversations", &json!({"on": true})).unwrap();
    let msgs = s.rows().into_iter().filter(|r| matches!(r, Row::Message { .. })).count();
    assert_eq!(msgs, 6, "the reply folds into its conversation");
    assert!(s.rows().iter().any(|r| matches!(r, Row::Message { count: 2, .. })));
    s.run("list.next", &json!({})).unwrap();
    s.run("list.next", &json!({})).unwrap();
    s.run("list.prev", &json!({})).unwrap();
    assert!(s.focus.is_some());
    done(s);
}

#[test]
fn categories_pins_and_focused_inbox() {
    let mut s = sample_session("organize");
    let k = key_of(&s, "Character turnarounds");
    s.run("list.select", &k).unwrap();
    s.run("mail.categorize", &json!({"category": "work"})).unwrap();
    assert!(s.selected()[0].flags.has("$label2"), "Work is Thunderbird's $label2");
    assert!(s.run("mail.categorize", &json!({"category": "Nope"})).is_err());
    s.run("category.create", &json!({"name": "Festival"})).unwrap();
    s.run("mail.categorize", &json!({"category": "festival"})).unwrap();
    let names: Vec<String> = s.categories_of(&s.selected()[0]).iter().map(|c| c.name.clone()).collect();
    assert_eq!(names, vec!["Work".to_string(), "Festival".to_string()]);
    // Search by category name or keyword.
    assert_eq!(s.run("search.run", &json!({"query": "category:work"})).unwrap()["results"], 1);
    assert_eq!(s.run("search.run", &json!({"query": "category:festival"})).unwrap()["results"], 1);
    s.run("search.clear", &json!({})).unwrap();
    // Stored with the message (local account: in the cache).
    let st = s.store.as_ref().unwrap().load_folder("sample", "INBOX").unwrap();
    assert!(st.messages.iter().find(|m| m.subject == "Character turnarounds").unwrap().flags.has("festival"));

    // Pinned mail comes first, under its own heading.
    s.run("list.select", &k).unwrap();
    s.run("mail.pin", &json!({})).unwrap();
    let rows = s.rows();
    assert_eq!(rows[0], Row::Header("Pinned"));
    let Row::Message { index, .. } = rows[1] else { panic!() };
    assert_eq!(s.list[index].subject, "Character turnarounds");
    assert_eq!(s.run("search.run", &json!({"query": "is:pinned"})).unwrap()["results"], 1);
    s.run("search.clear", &json!({})).unwrap();

    // Focused Inbox: list mail goes to Other; a sender can be moved for good.
    s.run("view.focused", &json!({"on": true})).unwrap();
    let focused: Vec<String> = s.list.iter().map(|m| m.subject.clone()).collect();
    assert!(!focused.contains(&"Open call: winter group show".to_string()), "{focused:?}");
    s.run("list.focusTab", &json!({"value": "other"})).unwrap();
    assert_eq!(s.list.len(), 1);
    assert_eq!(s.list[0].subject, "Open call: winter group show");
    let k2 = json!({"key": s.list[0].key.to_id()});
    s.run("list.select", &k2).unwrap();
    s.run("mail.moveToFocused", &json!({})).unwrap();
    assert!(s.list.is_empty());
    s.run("list.focusTab", &json!({"value": "focused"})).unwrap();
    assert_eq!(s.list.len(), 7);
    done(s);
}

#[test]
fn snoozed_mail_comes_back_unread() {
    let mut s = sample_session("snooze");
    let k = key_of(&s, "Character turnarounds");
    s.run("list.select", &k).unwrap();
    s.run("mail.markRead", &json!({})).unwrap();
    s.run("mail.snooze", &json!({"until": NOW + 3600})).unwrap();
    assert!(!s.list.iter().any(|m| m.subject == "Character turnarounds"));
    let snoozed = s.account("sample").unwrap().folders.iter().find(|f| f.path == "Snoozed").cloned().expect("a Snoozed folder is made");
    s.run("folder.open", &json!({"account": "sample", "folder": snoozed.path})).unwrap();
    assert_eq!(s.list.len(), 1);
    assert_eq!(s.snoozes.len(), 1);
    // Saved, so a restart still brings it back.
    let root = s.store.as_ref().unwrap().root().to_path_buf();
    let mut s = Session::open(Some(Store::open(&root).unwrap()), mailcraft_model::dates::Clock::new(NOW + 60, 0));
    assert_eq!(s.snoozes.len(), 1);
    s.tick();
    assert_eq!(s.snoozes.len(), 1, "not due yet");
    s.set_clock(NOW + 3601, 0);
    s.tick();
    assert!(s.snoozes.is_empty());
    let back = s.folder_state("sample", "INBOX").messages.iter().find(|m| m.subject == "Character turnarounds").cloned().unwrap();
    assert!(!back.flags.seen, "it comes back unread");
    assert!(s.run("mail.snooze", &json!({"preset": "someday"})).is_err());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn rules_file_new_mail_as_it_arrives() {
    let mut s = sample_session("rules");
    s.run(
        "rules.add",
        &json!({"rule": {"name": "Zoe to Archive", "conditions": [{"if": "from", "value": "zoe@"}], "actions": [{"do": "categorize", "value": "Work"}, {"do": "moveToRole", "value": "archive"}]}}),
    )
    .unwrap();
    assert!(s.run("rules.add", &json!({"rule": {"name": "x", "conditions": [], "actions": [{"do": "categorize", "value": "Nope"}]}})).is_err());
    // A message from Zoe arrives in the Inbox.
    let store = s.store.clone().unwrap();
    let mut st = store.load_folder("sample", "INBOX").unwrap();
    let mut m = st.messages[0].clone();
    m.key.uid = 500;
    m.subject = "Fresh from Zoe".into();
    m.from = mailcraft_model::EmailAddress::new("Zoe Park", "zoe@studio.example");
    m.flags.seen = false;
    m.date = NOW;
    st.messages.insert(0, m);
    store.save_folder("sample", "INBOX", &st).unwrap();
    s.apply(Event::FolderChanged { account: "sample".into(), folder: "INBOX".into() });
    assert!(!s.list.iter().any(|m| m.subject == "Fresh from Zoe"), "filed away by the rule");
    // Filed in a folder, it's still new mail: announced, as Outlook does.
    assert!(s.take_new_mail().iter().any(|m| m.subject == "Fresh from Zoe"), "filed mail is announced");
    s.run("folder.open", &json!({"account": "sample", "folder": "Archive"})).unwrap();
    let filed = s.list.iter().find(|m| m.subject == "Fresh from Zoe").unwrap();
    assert!(filed.flags.has("$label2"));
    // Rules persist, and can run on a folder by hand.
    let s2 = Session::open(Some(Store::open(store.root()).unwrap()), mailcraft_model::dates::Clock::new(NOW, 0));
    assert_eq!(s2.rules.len(), 1);
    s.run("folder.open", &json!({"account": "sample", "folder": "INBOX"})).unwrap();
    assert_eq!(s.run("rules.run", &json!({})).unwrap()["matched"], 1, "Zoe's older message in the Inbox");
    assert!(!s.list.iter().any(|m| m.from.email == "zoe@studio.example"));
    s.run("rules.delete", &json!({"index": 0})).unwrap();
    assert!(s.rules.is_empty());
    done(s);
}

/// Undo (Ctrl+Z) puts back the last move, delete, flag or category: moved mail is found again
/// by its Message-ID, as a move gives it a new key.
#[test]
fn undo_puts_back_moves_flags_and_categories() {
    let mut s = sample_session("undo");
    assert!(s.run("edit.undo", &json!({})).is_err(), "nothing to undo yet");
    let k = key_of(&s, "Storyboard review on Thursday");
    s.run("list.select", &k).unwrap();
    let was_flagged = s.selected()[0].flags.flagged;
    s.run("mail.flag", &json!({})).unwrap();
    s.run("edit.undo", &json!({})).unwrap();
    let m = s.list.iter().find(|m| m.subject == "Storyboard review on Thursday").unwrap();
    assert_eq!(m.flags.flagged, was_flagged, "the flag is back");
    s.run("list.select", &k).unwrap();
    s.run("mail.delete", &json!({})).unwrap();
    assert!(!s.list.iter().any(|m| m.subject == "Storyboard review on Thursday"), "gone from the Inbox");
    s.run("edit.undo", &json!({})).unwrap();
    assert!(s.list.iter().any(|m| m.subject == "Storyboard review on Thursday"), "back in the Inbox");
    let trash = s.account("sample").and_then(|a| a.folder(FolderRole::Trash)).map(|f| f.path.clone()).unwrap();
    assert!(!s.folder_state("sample", &trash).messages.iter().any(|m| m.subject == "Storyboard review on Thursday"));
    assert!(s.undo.is_empty());
}

/// Filter Email, as Outlook's: categorized mail, and mail sent or copied to the account.
#[test]
fn filters_for_categorized_mail_and_mail_to_me() {
    let mut s = sample_session("filters");
    let all = s.list.len();
    s.run("list.filter", &json!({"value": "categorized"})).unwrap();
    let before = s.list.len();
    s.run("list.filter", &json!({"value": "all"})).unwrap();
    let k = key_of(&s, "Character turnarounds");
    s.run("list.select", &k).unwrap();
    let cat = s.prefs.categories[0].name.clone();
    s.run("mail.categorize", &json!({"category": cat})).unwrap();
    s.run("list.filter", &json!({"value": "categorized"})).unwrap();
    assert_eq!(s.list.len(), before + 1);
    assert!(s.list.iter().any(|m| m.subject == "Character turnarounds"));
    s.run("list.filter", &json!({"value": "toMe"})).unwrap();
    assert!(!s.list.is_empty() && s.list.len() <= all);
    assert!(s.list.iter().all(|m| m.to.iter().chain(&m.cc).any(|r| r.email.eq_ignore_ascii_case("riley@studio.example"))));
    assert!(s.run("list.filter", &json!({"value": "nonsense"})).is_err());
}

/// Delete › Ignore, as Outlook's: the conversation goes to Deleted Items, and so do replies to it
/// that arrive later; ignoring it again stops that.
#[test]
fn ignored_conversations_stay_in_deleted_items() {
    let mut s = sample_session("ignore");
    let trash = s.account("sample").and_then(|a| a.folder(FolderRole::Trash)).map(|f| f.path.clone()).unwrap();
    let k = key_of(&s, "Storyboard review on Thursday");
    s.run("list.select", &k).unwrap();
    assert_eq!(s.run("mail.ignore", &json!({})).unwrap()["moved"], 2, "Zoe's message and Sam's reply");
    assert!(!s.list.iter().any(|m| m.subject.contains("Storyboard review")));
    // A reply in it arrives.
    let store = s.store.clone().unwrap();
    let mut st = store.load_folder("sample", "INBOX").unwrap();
    let mut m = st.messages[0].clone();
    let sam = s.folder_state("sample", &trash).messages.iter().find(|m| m.subject.starts_with("Re:")).cloned().unwrap();
    m.key.uid = 600;
    m.subject = "Re: Storyboard review on Thursday".into();
    m.message_id = "late-reply@studio.example".into();
    m.in_reply_to = sam.message_id.clone();
    m.references = vec![sam.message_id.clone()];
    m.flags.seen = false;
    m.date = NOW;
    st.messages.insert(0, m);
    store.save_folder("sample", "INBOX", &st).unwrap();
    s.apply(Event::FolderChanged { account: "sample".into(), folder: "INBOX".into() });
    assert!(!s.list.iter().any(|m| m.subject.contains("Storyboard review")), "went to Deleted Items");
    assert!(s.take_new_mail().is_empty(), "not announced");
    assert!(s.folder_state("sample", &trash).messages.iter().any(|m| m.message_id == "late-reply@studio.example"));
    // Stop ignoring (from Deleted Items).
    s.run("folder.open", &json!({"account": "sample", "folder": trash})).unwrap();
    let k = key_of(&s, "Storyboard review on Thursday");
    s.run("list.select", &k).unwrap();
    assert_eq!(s.run("mail.ignore", &json!({})).unwrap()["ignoring"], false);
    assert!(!s.selected().iter().any(|m| s.is_ignored(m)), "no longer ignored");
}

/// A rule can run right away on the mail already in the Inbox, whatever folder is on screen, as
/// Outlook's "run this rule now".
#[test]
fn a_rule_runs_now_on_the_inbox() {
    let mut s = sample_session("rule-now");
    s.run("rules.add", &json!({"rule": {"name": "Zoe to Archive", "conditions": [{"if": "from", "value": "zoe@"}], "actions": [{"do": "moveToRole", "value": "archive"}]}})).unwrap();
    s.run("folder.open", &json!({"account": "sample", "folder": "Archive"})).unwrap();
    let r = s.run("rules.run", &json!({"index": 0, "inbox": true})).unwrap();
    assert_eq!(r["matched"], 1);
    assert!(s.notices.last().is_some_and(|n| n.text.contains("1 message matched")));
    assert!(!s.folder_state("sample", "INBOX").messages.iter().any(|m| m.from.email == "zoe@studio.example"), "moved out of the Inbox");
    assert!(s.list.iter().any(|m| m.from.email == "zoe@studio.example"), "and into Archive, on screen");
}

/// Mail a rule deletes isn't announced (mail it files in a folder is).
#[test]
fn mail_a_rule_deletes_is_not_announced() {
    let mut s = sample_session("rule-quiet");
    s.run("rules.add", &json!({"rule": {"name": "Zoe away", "conditions": [{"if": "from", "value": "zoe@"}], "actions": [{"do": "delete"}]}}))
        .unwrap();
    let store = s.store.clone().unwrap();
    let mut st = store.load_folder("sample", "INBOX").unwrap();
    let mut m = st.messages[0].clone();
    m.key.uid = 700;
    m.subject = "Deleted on arrival".into();
    m.from = mailcraft_model::EmailAddress::new("Zoe Park", "zoe@studio.example");
    m.flags.seen = false;
    m.date = NOW;
    st.messages.insert(0, m);
    store.save_folder("sample", "INBOX", &st).unwrap();
    s.apply(Event::FolderChanged { account: "sample".into(), folder: "INBOX".into() });
    assert!(!s.take_new_mail().iter().any(|m| m.subject == "Deleted on arrival"));
}

/// Rules go to a file and come back, on this computer or another: an account by its address,
/// a missing category made, a rule already here not doubled, junk refused.
#[test]
fn rules_export_and_import() {
    let mut s = sample_session("rules-io");
    s.run("category.create", &json!({"name": "Clients"})).unwrap();
    let rule = |name: &str, account: Option<&str>| json!({"rule": {"name": name, "conditions": [{"if": "from", "value": "printshop"}], "actions": [{"do": "categorize", "value": "Clients"}], "account": account}});
    s.run("rules.add", &rule("Print shop", Some("sample"))).unwrap();
    s.run("rules.add", &rule("Everywhere", None)).unwrap();
    let file = s.store.as_ref().unwrap().root().join("rules.mailcraft-rules");
    s.run("rules.export", &json!({"path": file.to_string_lossy()})).unwrap();
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.contains("\"accountEmail\"") && !text.contains("\"account\": \"sample\""), "an account by its address: {text}");

    // Here again: nothing doubled.
    let r = s.run("rules.import", &json!({"path": file.to_string_lossy()})).unwrap();
    assert_eq!((r["added"].as_u64(), s.rules.len()), (Some(0), 2));

    // Another computer: the same address, no Clients category yet.
    let mut other = sample_session("rules-io-2");
    assert!(other.category("Clients").is_none());
    let r = other.run("rules.import", &json!({"path": file.to_string_lossy()})).unwrap();
    assert_eq!((r["added"].as_u64(), r["turnedOff"].as_u64()), (Some(2), Some(0)));
    assert!(other.category("Clients").is_some(), "the category is made");
    assert_eq!(other.rules[0].account.as_deref(), Some("sample"));
    assert!(other.rules[0].enabled);

    // An account this computer doesn't have: the rule comes in turned off. (Saved again by an
    // editor that starts it with a byte order mark.)
    let foreign = text.replacen("riley@studio.example", "nobody@elsewhere.example", 1);
    std::fs::write(&file, [&b"\xEF\xBB\xBF"[..], foreign.as_bytes()].concat()).unwrap();
    let r = other.run("rules.import", &json!({"path": file.to_string_lossy(), "replace": true})).unwrap();
    assert_eq!((r["turnedOff"].as_u64(), other.rules.len()), (Some(1), 2));
    assert!(!other.rules[0].enabled && other.rules[0].account.is_none());

    // Junk, and a file that isn't one.
    for junk in [&b"{\"rules\": []}"[..], b"not json", b"{\"mailcraftRules\": 1, \"rules\": [{\"name\": 3}]}", &[0xff; 64][..]] {
        std::fs::write(&file, junk).unwrap();
        assert!(other.run("rules.import", &json!({"path": file.to_string_lossy()})).is_err());
    }
    assert_eq!(other.rules.len(), 2, "a bad file changes nothing");
    assert!(other.run("rules.import", &json!({"path": "/no/such/file"})).is_err());
    done(other);
    done(s);
}

/// Junk Email lists, as Outlook's: a blocked sender's mail goes to Junk as it arrives, a safe
/// sender's comes back out of it; a domain covers every address at it; safe wins over blocked.
#[test]
fn blocked_and_safe_senders_sort_mail_as_it_arrives() {
    let mut s = sample_session("junk-lists");
    let junk = s.account("sample").and_then(|a| a.folder(FolderRole::Junk)).map(|f| f.path.clone()).unwrap();
    let store = s.store.clone().unwrap();
    let arrive = |s: &mut Session, folder: &str, uid: u32, from: &str, subject: &str| {
        let mut st = store.load_folder("sample", folder).unwrap();
        let mut m = store.load_folder("sample", "INBOX").unwrap().messages[0].clone();
        m.key.folder = folder.into();
        m.key.uid = uid;
        m.key.validity = st.validity;
        m.from = mailcraft_model::EmailAddress::new("", from);
        m.subject = subject.into();
        m.message_id = format!("{uid}@junk.example");
        m.in_reply_to.clear();
        m.references.clear();
        m.flags.seen = false;
        m.date = NOW;
        st.messages.insert(0, m);
        store.save_folder("sample", folder, &st).unwrap();
        s.apply(Event::FolderChanged { account: "sample".into(), folder: folder.into() });
    };
    let inbox_has = |s: &mut Session, subject: &str| s.folder_state("sample", "INBOX").messages.iter().any(|m| m.subject == subject);
    let junk_has = |s: &mut Session, subject: &str| s.folder_state("sample", &junk).messages.iter().any(|m| m.subject == subject);
    // Block Zoe from the selection: her mail goes to Junk now, and as it comes.
    let k = key_of(&s, "Storyboard review on Thursday");
    s.run("list.select", &k).unwrap();
    let r = s.run("junk.blockSender", &json!({})).unwrap();
    assert_eq!(r["added"], json!(["zoe@studio.example"]));
    assert!(junk_has(&mut s, "Storyboard review on Thursday"));
    s.take_new_mail();
    arrive(&mut s, "INBOX", 700, "ZOE@Studio.Example", "Again");
    assert!(junk_has(&mut s, "Again") && !inbox_has(&mut s, "Again"), "an address matches whatever its case");
    assert!(s.take_new_mail().is_empty(), "blocked mail isn't announced");
    // A domain: every address at it, not its look-alikes.
    s.run("junk.edit", &json!({"list": "blocked", "entry": "Spam.Example"})).unwrap();
    arrive(&mut s, "INBOX", 701, "a@spam.example", "Offer");
    arrive(&mut s, "INBOX", 702, "b@notspam.example", "Fine");
    arrive(&mut s, "INBOX", 703, "c@spam.example.org", "Fine too");
    assert!(junk_has(&mut s, "Offer") && inbox_has(&mut s, "Fine") && inbox_has(&mut s, "Fine too"));
    // Safe wins: never blocking a@ lets just that address through.
    s.run("junk.edit", &json!({"list": "safe", "entry": "<a@spam.example>"})).unwrap();
    arrive(&mut s, "INBOX", 704, "a@spam.example", "Through");
    arrive(&mut s, "INBOX", 705, "d@spam.example", "Still junk");
    assert!(inbox_has(&mut s, "Through") && junk_has(&mut s, "Still junk"));
    // A safe sender's mail filed in Junk by the server comes back to the Inbox: Junk's mark is
    // set as soon as its folder is known, so the first change it sees is already an arrival
    // (found against a real server: Junk empty at the first sync sent no change).
    s.seen_uid.clear();
    let folders = s.account("sample").unwrap().folders.clone();
    s.apply(Event::Folders { account: "sample".into(), folders });
    arrive(&mut s, &junk, 900, "a@spam.example", "Misfiled");
    assert!(inbox_has(&mut s, "Misfiled") && !junk_has(&mut s, "Misfiled"));
    // Never block from Junk: on the safe list, off the blocked one, and back to the Inbox.
    s.run("folder.open", &json!({"account": "sample", "folder": junk})).unwrap();
    let k = key_of(&s, "Again");
    s.run("list.select", &k).unwrap();
    s.run("junk.neverBlockSender", &json!({})).unwrap();
    assert!(inbox_has(&mut s, "Again"));
    assert!(s.prefs.safe_senders.contains("zoe@studio.example") && !s.prefs.blocked_senders.contains("zoe@studio.example"));
    // Not junk, always trusting the domain's sender; the domain itself.
    let k = key_of(&s, "Still junk");
    s.run("list.select", &k).unwrap();
    s.run("junk.neverBlockDomain", &json!({})).unwrap();
    assert!(s.prefs.safe_senders.contains("@spam.example") && !s.prefs.blocked_senders.contains("@spam.example"));
    assert!(inbox_has(&mut s, "Still junk"));
    // Junk entries and params.
    for bad in ["", "@", "nobody", "a@", "@-x.example", "x..example", "a@b@c.example", &"x".repeat(400)] {
        assert!(s.run("junk.edit", &json!({"list": "blocked", "entry": bad})).is_err(), "{bad:?}");
    }
    assert!(s.run("junk.edit", &json!({"list": "nope", "entry": "a@b.example"})).is_err());
    assert!(s.run("junk.edit", &json!({"entry": "a@b.example"})).is_err());
    let r = s.run("junk.edit", &json!({"list": "safe", "entry": "a@spam.example", "on": false})).unwrap();
    assert!(!r["safe"].as_array().unwrap().iter().any(|e| e == "a@spam.example"));
    // The lists are kept with the settings.
    let lists = s.run("junk.lists", &json!({})).unwrap();
    assert!(lists["safe"].as_array().unwrap().iter().any(|e| e == "@spam.example"));
    let back: crate::session::Prefs = serde_json::from_value(serde_json::to_value(&s.prefs).unwrap()).unwrap();
    assert_eq!((back.safe_senders, back.blocked_senders), (s.prefs.safe_senders.clone(), s.prefs.blocked_senders.clone()));
}

/// Clean Up: in a conversation, a message a later reply quotes in full goes to Deleted Items
/// (undoably); one that isn't, isn't downloaded, or is flagged, stays.
#[test]
fn clean_up_moves_only_what_a_reply_quotes() {
    let mut s = sample_session("clean-up");
    let trash = s.account("sample").and_then(|a| a.folder(FolderRole::Trash)).map(|f| f.path.clone()).unwrap();
    let store = s.store.clone().unwrap();
    let mut st = store.load_folder("sample", "INBOX").unwrap();
    let template = st.messages[0].clone();
    let thread = [
        (801, "Frames for Thursday", "Can you send the frames?\r\n\r\nZoe", true),
        (802, "Re: Frames for Thursday", "Sure.\r\n\r\n> Can you send the frames?\r\n>\r\n> Zoe", true),
        (803, "Re: Frames for Thursday", "Thanks!\r\n\r\n> Sure.\r\n>\r\n>> Can you send the frames?\r\n>>\r\n>> Zoe", true),
        // Another conversation: one not downloaded, and a flagged one.
        (811, "Budget", "Numbers attached.", false),
        (812, "Re: Budget", "Got them.\r\n> Numbers attached.", true),
    ];
    for (i, (uid, subject, body, downloaded)) in thread.iter().enumerate() {
        let mut m = template.clone();
        m.key.uid = *uid;
        m.subject = subject.to_string();
        m.message_id = format!("c{uid}@studio.example");
        let parent = if *uid == 801 || *uid == 811 { None } else { Some(format!("c{}@studio.example", uid - 1)) };
        m.in_reply_to = parent.clone().unwrap_or_default();
        m.references = if *uid == 803 { vec!["c801@studio.example".into(), "c802@studio.example".into()] } else { parent.into_iter().collect() };
        m.date = NOW - 1000 + i as i64;
        m.flags.seen = true;
        m.has_attachments = false;
        st.messages.insert(0, m.clone());
        if *downloaded {
            let raw = format!(
                "From: a@b.example\r\nSubject: {subject}\r\nMessage-ID: <c{uid}@studio.example>\r\nContent-Type: text/plain\r\n\r\n{body}\r\n"
            );
            store.write_body(&m.key, raw.as_bytes()).unwrap();
        }
    }
    store.save_folder("sample", "INBOX", &st).unwrap();
    s.apply(Event::FolderChanged { account: "sample".into(), folder: "INBOX".into() });
    let in_trash = |s: &mut Session, uid_subject: &str| s.folder_state("sample", &trash).messages.iter().any(|m| m.message_id == uid_subject);
    // The selection's conversation only.
    s.run("folder.open", &json!({"account": "sample", "folder": "INBOX"})).unwrap();
    let k = s.list.iter().find(|m| m.message_id == "c803@studio.example").map(|m| json!({"key": m.key.to_id()})).unwrap();
    s.run("list.select", &k).unwrap();
    assert_eq!(s.run("mail.cleanUp", &json!({})).unwrap()["moved"], 2);
    assert!(in_trash(&mut s, "c801@studio.example") && in_trash(&mut s, "c802@studio.example") && !in_trash(&mut s, "c803@studio.example"));
    // Undone: back.
    s.run("edit.undo", &json!({})).unwrap();
    assert!(!in_trash(&mut s, "c801@studio.example"));
    // The folder: the budget thread's first isn't downloaded, so it stays; flag the first of
    // the frames thread and only the reply in the middle goes.
    let k = s.list.iter().find(|m| m.message_id == "c801@studio.example").map(|m| json!({"key": m.key.to_id()})).unwrap();
    s.run("list.select", &k).unwrap();
    s.run("mail.flag", &json!({})).unwrap();
    let r = s.run("folder.cleanUp", &json!({})).unwrap();
    assert_eq!(r["moved"], 1, "{r}");
    assert!(in_trash(&mut s, "c802@studio.example") && !in_trash(&mut s, "c801@studio.example") && !in_trash(&mut s, "c811@studio.example"));
    // Again: nothing more; Deleted Items isn't cleaned; bad params.
    assert_eq!(s.run("folder.cleanUp", &json!({"account": "sample", "folder": "INBOX"})).unwrap()["moved"], 0);
    assert_eq!(s.run("folder.cleanUp", &json!({"account": "sample", "folder": trash})).unwrap()["moved"], 0);
    assert!(s.run("folder.cleanUp", &json!({"account": "sample", "folder": "Nope"})).is_err());
    assert!(s.run("folder.cleanUp", &json!({"account": "nobody", "folder": "INBOX"})).is_err());
}

/// Search Folders: saved searches over every folder but Deleted Items and Junk, kept with the
/// settings; renamed, changed, deleted (its mail stays); names and queries checked.
#[test]
fn search_folders_find_mail_everywhere_but_deleted_and_junk() {
    let mut s = sample_session("search-folders");
    let r = s.run("searchFolder.create", &json!({"name": "Unread mail", "query": "is:unread"})).unwrap();
    let unread_everywhere: Vec<String> = s.list.iter().map(|m| m.subject.clone()).collect();
    assert_eq!(r["messages"], unread_everywhere.len());
    assert!(!unread_everywhere.is_empty());
    assert_eq!(s.open_search_folder_name(), Some("Unread mail"));
    // Junk's unread one isn't in it; the plain search finds it.
    let junk = s.account("sample").and_then(|a| a.folder(FolderRole::Junk)).map(|f| f.path.clone()).unwrap();
    let junk_unread = s.folder_state("sample", &junk).messages.iter().filter(|m| !m.flags.seen).count();
    assert!(junk_unread > 0, "the sample's Junk has unread mail");
    assert!(s.list.iter().all(|m| m.key.folder != junk));
    s.run("search.run", &json!({"query": "is:unread"})).unwrap();
    assert!(s.list.iter().any(|m| m.key.folder == junk), "a search of its own isn't the Search Folder");
    assert_eq!(s.open_search_folder_name(), None);
    // From certain people, any of them; larger than.
    s.run("searchFolder.create", &json!({"name": "From Zoe or Ana", "query": "from:zoe,ana"})).unwrap();
    let froms: Vec<String> = s.list.iter().map(|m| m.from.email.clone()).collect();
    assert!(froms.iter().any(|f| f.starts_with("zoe")) && froms.iter().any(|f| f.starts_with("ana")), "{froms:?}");
    assert!(froms.iter().all(|f| f.starts_with("zoe") || f.starts_with("ana")));
    // Changed and renamed while open: shows what it finds now.
    let r = s.run("searchFolder.edit", &json!({"name": "from zoe or ana", "newName": "Zoe", "query": "from:zoe"})).unwrap();
    assert_eq!(r["name"], "Zoe");
    assert!(s.list.iter().all(|m| m.from.email.starts_with("zoe")));
    assert_eq!(s.open_search_folder_name(), Some("Zoe"));
    // Checked: names unique whatever their case, a query that searches, limits.
    for bad in [
        json!({"name": "", "query": "is:unread"}),
        json!({"name": "  ", "query": "is:unread"}),
        json!({"name": "UNREAD MAIL", "query": "is:flagged"}),
        json!({"name": "Nothing", "query": "   "}),
        json!({"name": "x".repeat(101), "query": "is:unread"}),
        json!({"name": "Long", "query": "a".repeat(1001)}),
        json!({"name": "Ctl\u{7}", "query": "is:unread"}),
        json!({"name": "No query"}),
    ] {
        assert!(s.run("searchFolder.create", &bad).is_err(), "{bad}");
    }
    assert!(s.run("searchFolder.edit", &json!({"name": "Zoe", "newName": "Unread mail"})).is_err(), "taken");
    assert!(s.run("searchFolder.open", &json!({"name": "Nope"})).is_err());
    assert!(s.run("searchFolder.delete", &json!({"name": "Nope"})).is_err());
    // Deleted while open: the search goes, the mail stays.
    let before = s.folder_state("sample", "INBOX").messages.len();
    s.run("searchFolder.delete", &json!({"name": "zoe"})).unwrap();
    assert!(s.search.is_none());
    assert_eq!(s.folder_state("sample", "INBOX").messages.len(), before);
    // Kept with the settings; capped.
    let back: crate::session::Prefs = serde_json::from_value(serde_json::to_value(&s.prefs).unwrap()).unwrap();
    assert_eq!(back.search_folders, s.prefs.search_folders);
    for n in 0..200 {
        let _ = s.run("searchFolder.create", &json!({"name": format!("F{n}"), "query": "is:flagged"}));
    }
    assert_eq!(s.prefs.search_folders.len(), crate::session::search_folders::SEARCH_FOLDERS_MAX);
}

/// Quick Steps: their marks, then their one move (undoable); by name, place or shortcut;
/// checked as they're made; a category deleted since is passed over.
#[test]
fn quick_steps_mark_then_move_the_selection() {
    let mut s = sample_session("quick-steps");
    s.run("category.create", &json!({"name": "Done"})).unwrap();
    let archive = s.account("sample").and_then(|a| a.folder(FolderRole::Archive)).map(|f| f.path.clone()).unwrap();
    let r = s
        .run(
            "quickStep.create",
            &json!({"name": "Done", "actions": [{"do": "moveToRole", "value": "archive"}, {"do": "markRead"}, {"do": "categorize", "value": "Done"}, {"do": "markRead"}]}),
        )
        .unwrap();
    assert_eq!(r["index"], 0);
    assert_eq!(s.prefs.quick_steps[0].actions.len(), 3, "the same action twice is done once");
    s.run("quickStep.create", &json!({"name": "Flag it", "actions": [{"do": "flag"}, {"do": "pin"}]})).unwrap();
    // Ctrl+Shift+1: the first, on Ana's unread message.
    let k = key_of(&s, "Sound mix, version 3");
    s.run("list.select", &k).unwrap();
    let id = s.summary(&crate::parse_key(&k).unwrap()).unwrap().message_id.clone();
    let by_key = crate::cmd::registry().all().iter().find(|c| c.shortcut == "Mod+Shift+1").map(|c| c.id);
    assert_eq!(by_key, Some("quickStep.1"));
    s.run("quickStep.1", &json!({})).unwrap();
    let moved = s.folder_state("sample", &archive).messages.iter().find(|m| m.message_id == id).cloned().unwrap();
    assert!(moved.flags.seen && !moved.flags.keywords.is_empty(), "read and categorized before it left: {:?}", moved.flags);
    // Undone: back in the Inbox.
    s.run("edit.undo", &json!({})).unwrap();
    assert!(s.folder_state("sample", "INBOX").messages.iter().any(|m| m.message_id == id));
    // By name: flagged and pinned, staying where it is.
    let k = key_of(&s, "Character turnarounds");
    s.run("list.select", &k).unwrap();
    s.run("quickStep.run", &json!({"name": "flag IT"})).unwrap();
    let m = s.folder_state("sample", "INBOX").messages.iter().find(|m| m.subject == "Character turnarounds").cloned().unwrap();
    assert!(m.flags.flagged && m.flags.pinned());
    // A category deleted since: the rest is still done.
    s.run("category.delete", &json!({"name": "Done"})).unwrap();
    let k = key_of(&s, "Open call: winter group show");
    s.run("list.select", &k).unwrap();
    s.run("quickStep.run", &json!({"index": 0})).unwrap();
    assert!(s.folder_state("sample", &archive).messages.iter().any(|m| m.subject == "Open call: winter group show"));
    // No such step; none selected.
    assert!(s.run("quickStep.9", &json!({})).is_err());
    s.selection.clear();
    assert!(s.run("quickStep.1", &json!({})).is_err());
    // Checked as made and edited.
    for bad in [
        json!({"name": "", "actions": [{"do": "flag"}]}),
        json!({"name": "done", "actions": [{"do": "flag"}]}),
        json!({"name": "Nothing", "actions": []}),
        json!({"name": "No actions"}),
        json!({"name": "Two moves", "actions": [{"do": "delete"}, {"do": "moveToRole", "value": "archive"}]}),
        json!({"name": "Ghost", "actions": [{"do": "categorize", "value": "Nope"}]}),
        json!({"name": "Nowhere", "actions": [{"do": "moveTo", "value": "No/Such"}]}),
        json!({"name": "Odd", "actions": [{"do": "explode"}]}),
        json!({"name": "Many", "actions": vec![json!({"do": "flag"}); 11]}),
    ] {
        assert!(s.run("quickStep.create", &bad).is_err(), "{bad}");
    }
    s.run("quickStep.edit", &json!({"name": "Flag it", "newName": "Flag", "actions": [{"do": "flag"}]})).unwrap();
    assert!(s.run("quickStep.edit", &json!({"name": "Flag", "newName": "Done"})).is_err(), "taken");
    s.run("quickStep.delete", &json!({"name": "flag"})).unwrap();
    assert_eq!(s.prefs.quick_steps.len(), 1);
    let back: crate::session::Prefs = serde_json::from_value(serde_json::to_value(&s.prefs).unwrap()).unwrap();
    assert_eq!(back.quick_steps, s.prefs.quick_steps);
}
