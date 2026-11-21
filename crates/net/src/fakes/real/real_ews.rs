//! Against a real Exchange Web Services server on this computer (grommunio in Docker; never a
//! real account), run by hand: what the in-memory server can't prove about a real one's answers.
//!
//! ```text
//! MAILCRAFT_REAL_EWS="http://127.0.0.1:18080/EWS/Exchange.asmx user@domain password" \
//!   cargo test -p mailcraft-net real_ews -- --ignored --nocapture
//! ```
//!
//! The mailbox's calendar holds `Budget review`, `Stand-up` (Monday, Wednesday and Friday),
//! `Company holiday` (all day) and `Design sync`; its contacts, Ana Silva.

use std::time::{Duration, Instant};

use mailcraft_model::job::Secret;
use mailcraft_model::pim::Pim;
use mailcraft_model::{AccountConfig, Event, Job, SyncStatus};

use crate::worker::{MemoryCredentials, Pool};

/// `url user password` from the environment, or `None` (the test is skipped).
fn server() -> Option<(String, String, String)> {
    let v = std::env::var("MAILCRAFT_REAL_EWS").ok()?;
    let mut parts = v.split_whitespace().map(str::to_string);
    Some((parts.next()?, parts.next()?, parts.next()?))
}

/// The calendar and the contacts a real server holds are read, each whole.
#[test]
#[ignore = "needs a real Exchange server in MAILCRAFT_REAL_EWS"]
fn real_ews_calendar_and_contacts() {
    let Some((url, user, password)) = server() else {
        eprintln!("MAILCRAFT_REAL_EWS not set: skipped");
        return;
    };
    let c = crate::ews::Client::connect(&url, &user, &password, &user).unwrap();
    let sync = crate::ews_pim::sync(&c, &Pim::default(), 1_792_000_000).unwrap();
    for e in &sync.events {
        eprintln!(
            "event {:?} {} {} all_day={} rrule={:?} loc={:?}",
            e.event.summary, e.event.start, e.event.end, e.event.all_day, e.event.rrule, e.event.location
        );
    }
    for k in &sync.cards {
        eprintln!("card {:?} {:?}", k.card.display(), k.card.emails.iter().map(|e| e.value.as_str()).collect::<Vec<_>>());
    }
    eprintln!("errors {:?}", sync.errors);
    eprintln!("calendars {:?}", sync.calendars.as_ref().map(|c| c.iter().map(|c| c.name.as_str()).collect::<Vec<_>>()));
    let names: Vec<&str> = sync.events.iter().map(|e| e.event.summary.as_str()).collect();
    for want in ["Budget review", "Stand-up", "Company holiday", "Design sync"] {
        assert!(names.contains(&want), "{want} is missing: {names:?}");
    }
    let standup = sync.events.iter().find(|e| e.event.summary == "Stand-up").unwrap();
    assert_eq!(standup.event.rrule.as_deref(), Some("FREQ=WEEKLY;BYDAY=MO,WE,FR"));
    assert!(sync.events.iter().any(|e| e.event.summary == "Company holiday" && e.event.all_day));
    assert!(sync.cards.iter().any(|k| k.card.emails.iter().any(|e| e.value == "ana@ews.test")), "the contact is missing");
    assert!(sync.errors.is_empty(), "{:?}", sync.errors);
}

/// A mailbox with a dozen folders of mail to bring in has its calendar and contacts once the
/// Inbox is in, not after the last folder (issue #80: the calendar stayed empty meanwhile).
#[test]
#[ignore = "needs a real Exchange server in MAILCRAFT_REAL_EWS"]
fn real_ews_calendar_comes_with_the_inbox() {
    let Some((url, user, password)) = server() else {
        eprintln!("MAILCRAFT_REAL_EWS not set: skipped");
        return;
    };
    let store = crate::fake::store("real-ews-first");
    let mut pool = Pool::new(store, std::sync::Arc::new(MemoryCredentials::default()));
    let account = AccountConfig { id: "x1".into(), email: user.clone(), username: user, ews: Some(url), ..Default::default() };
    let began = Instant::now();
    pool.submit(Job::Start { account, password: Some(Secret(password)) });
    let (mut folders, mut calendar, mut online) = (Vec::new(), None, None);
    let end = Instant::now() + Duration::from_secs(900);
    while Instant::now() < end && (calendar.is_none() || online.is_none()) {
        match pool.wait(Duration::from_millis(100)) {
            Some(Event::Status { status: SyncStatus::Syncing { folder }, .. }) if !folders.contains(&folder) => {
                eprintln!("{:>6.1} s  {folder}", began.elapsed().as_secs_f32());
                folders.push(folder);
            }
            Some(Event::DavSynced { sync, .. }) => {
                eprintln!(
                    "{:>6.1} s  calendar: {} events, {} contacts, after {} folders",
                    began.elapsed().as_secs_f32(),
                    sync.events.len(),
                    sync.cards.len(),
                    folders.len()
                );
                calendar.get_or_insert((began.elapsed(), folders.len(), sync.events.len()));
            }
            Some(Event::Status { status: SyncStatus::Online { .. }, .. }) => {
                eprintln!("{:>6.1} s  online, {} folders", began.elapsed().as_secs_f32(), folders.len());
                online.get_or_insert(began.elapsed());
            }
            Some(Event::Notice { message, .. }) => eprintln!("notice: {message}"),
            _ => {}
        }
    }
    let (at, after, events) = calendar.expect("the calendar was never read");
    let online = online.expect("the mail never finished");
    assert!(events >= 4, "{events} events");
    assert!(after <= 1, "the calendar waited for {after} folders");
    assert!(at < online, "the calendar came at {at:?}, the mail was done at {online:?}");
}

/// Issue #139 on a real server: a rule made here (its folder by its path, Junk Email) is on the
/// server and read back as made, its folder told by its path; turned off and renamed, then
/// deleted. Any rules already there are left as they are.
#[test]
#[ignore = "needs a real Exchange server in MAILCRAFT_REAL_EWS"]
fn real_ews_inbox_rules() {
    use mailcraft_model::organize::{Condition, Rule, RuleAction, ServerRule, ServerRuleOp, ServerRules};
    let Some((url, user, password)) = server() else {
        eprintln!("MAILCRAFT_REAL_EWS not set: skipped");
        return;
    };
    let store = crate::fake::store("real-ews-rules");
    let mut pool = Pool::new(store.clone(), std::sync::Arc::new(MemoryCredentials::default()));
    let account = AccountConfig { id: "x1".into(), email: user.clone(), username: user, ews: Some(url), ..Default::default() };
    pool.submit(Job::Start { account, password: Some(Secret(password)) });
    let wait = |pool: &mut Pool, what: &str, f: &dyn Fn(&Event) -> bool| -> Event {
        let end = Instant::now() + Duration::from_secs(300);
        while Instant::now() < end {
            if let Some(e) = pool.wait(Duration::from_millis(100))
                && f(&e)
            {
                return e;
            }
        }
        panic!("no {what}");
    };
    wait(&mut pool, "online", &|e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    let junk = store.load_folders("x1").unwrap().into_iter().find(|f| f.role == mailcraft_model::FolderRole::Junk).expect("Junk Email").path;
    let rules = |pool: &mut Pool, job: Job| -> (Option<ServerRules>, Option<String>) {
        pool.submit(job);
        match wait(pool, "the rules", &|e| matches!(e, Event::ServerRules { .. })) {
            Event::ServerRules { rules, error, .. } => (rules, error),
            other => panic!("{other:?}"),
        }
    };
    let (before, error) = rules(&mut pool, Job::GetServerRules { account: "x1".into() });
    eprintln!("before: {before:?} {error:?}");
    let before = before.expect("the server's rules");
    let new = Rule {
        name: "MailCraft test".into(),
        enabled: true,
        account: None,
        conditions: vec![Condition::Subject("mailcraft-rule-test".into())],
        match_all: true,
        actions: vec![RuleAction::MoveTo(junk.clone())],
        stop: true,
        sent: false,
    };
    let now = ServerRule {
        name: "MailCraft test".into(),
        priority: before.rules.len() as u32 + 1,
        enabled: true,
        rule: Some(new.clone()),
        ..Default::default()
    };
    let ops = vec![ServerRuleOp::Create { now }];
    let (after, error) = rules(&mut pool, Job::SetServerRules { account: "x1".into(), ops, drop_outlook_list: before.outlook_list });
    eprintln!("made: {after:?} {error:?}");
    assert_eq!(error, None);
    let made = after.unwrap().rules.into_iter().find(|r| r.name == "MailCraft test").expect("on the server");
    let back = made.rule.clone().expect("read back as MailCraft's own");
    assert_eq!((back.conditions, back.actions, back.stop), (new.conditions, new.actions, true), "its folder by its path");
    let now = ServerRule { name: "MailCraft test (off)".into(), enabled: false, ..made.clone() };
    let (after, error) =
        rules(&mut pool, Job::SetServerRules { account: "x1".into(), ops: vec![ServerRuleOp::Set { was: made, now }], drop_outlook_list: false });
    assert_eq!(error, None);
    let off = after.unwrap().rules.into_iter().find(|r| r.name == "MailCraft test (off)").expect("renamed");
    assert!(!off.enabled);
    let (after, error) =
        rules(&mut pool, Job::SetServerRules { account: "x1".into(), ops: vec![ServerRuleOp::Delete { was: off }], drop_outlook_list: false });
    assert_eq!(error, None);
    assert_eq!(after.unwrap().rules.len(), before.rules.len(), "gone again");
}

/// Free/busy times (issue #140) from a real availability service: an answer for each person
/// asked about, in order, whatever the server makes of them (what it can't give is said per
/// person, never a failure of the whole). Printed for a look.
#[test]
#[ignore = "needs a real Exchange server in MAILCRAFT_REAL_EWS"]
fn real_ews_free_busy() {
    let Some((url, user, password)) = server() else {
        eprintln!("MAILCRAFT_REAL_EWS not set: skipped");
        return;
    };
    let c = crate::ews::Client::connect(&url, &user, &password, &user).unwrap();
    let domain = user.split('@').nth(1).unwrap_or("ews.test");
    let people = vec![user.clone(), format!("ana@{domain}"), format!("nobody-here@{domain}")];
    let now = 1_793_000_000;
    let answers = c.availability(&people, now - 7 * 86_400, now + 60 * 86_400).unwrap();
    for a in &answers {
        eprintln!("{}: {} busy, {:?}", a.email, a.busy.len(), a.error);
    }
    assert_eq!(answers.iter().map(|a| a.email.as_str()).collect::<Vec<_>>(), people.iter().map(String::as_str).collect::<Vec<_>>());
    assert!(answers[2].error.is_some(), "nobody is there: {:?}", answers[2]);
}

/// New mail in a folder other than the Inbox (issue #145: an Inbox rule filed it there) comes
/// with the next look for mail, the server's subscription saying where, not a quarter of an hour
/// on with the full sync. The message is made in the Inbox's subfolder `Filed` (made when there
/// is none), then deleted again.
#[test]
#[ignore = "needs a real Exchange server in MAILCRAFT_REAL_EWS"]
fn real_ews_new_mail_in_any_folder() {
    let Some((url, user, password)) = server() else {
        eprintln!("MAILCRAFT_REAL_EWS not set: skipped");
        return;
    };
    let c = crate::ews::Client::connect(&url, &user, &password, &user).unwrap();
    let id_of = |doc: &crate::ews::El| doc.find("FolderId").and_then(|f| f.attr("Id")).map(str::to_string);
    let found = c
        .call(
            "<m:FindFolder Traversal=\"Shallow\"><m:FolderShape><t:BaseShape>Default</t:BaseShape></m:FolderShape>\
             <m:Restriction><t:IsEqualTo><t:FieldURI FieldURI=\"folder:DisplayName\"/><t:FieldURIOrConstant><t:Constant Value=\"Filed\"/>\
             </t:FieldURIOrConstant></t:IsEqualTo></m:Restriction><m:ParentFolderIds><t:DistinguishedFolderId Id=\"inbox\"/></m:ParentFolderIds></m:FindFolder>",
        )
        .ok()
        .and_then(|d| id_of(&d));
    let filed = found.unwrap_or_else(|| {
        let made = c
            .call(
                "<m:CreateFolder><m:ParentFolderId><t:DistinguishedFolderId Id=\"inbox\"/></m:ParentFolderId><m:Folders><t:Folder>\
                 <t:DisplayName>Filed</t:DisplayName></t:Folder></m:Folders></m:CreateFolder>",
            )
            .unwrap();
        id_of(&made).expect("a folder made")
    });
    crate::worker::POLL_FOR.lock().unwrap().push(("x2".into(), Duration::from_secs(2)));
    let store = crate::fake::store("real-ews-watch");
    let mut pool = Pool::new(store.clone(), std::sync::Arc::new(MemoryCredentials::default()));
    let account = AccountConfig { id: "x2".into(), email: user.clone(), username: user, ews: Some(url), ..Default::default() };
    pool.submit(Job::Start { account, password: Some(Secret(password)) });
    // Its mail and its calendars read (the first sync reads the calendars after the Inbox,
    // before the rest).
    let (mut online, mut calendars) = (false, false);
    let end = Instant::now() + Duration::from_secs(300);
    while !(online && calendars) {
        assert!(Instant::now() < end, "online {online}, calendars read {calendars}");
        match pool.wait(Duration::from_millis(100)) {
            Some(Event::Status { status: SyncStatus::Online { .. }, .. }) => online = true,
            Some(Event::DavSynced { .. }) => calendars = true,
            _ => {}
        }
    }
    let path = store.load_folders("x2").unwrap().into_iter().find(|f| f.path.ends_with("Filed")).expect("Filed listed").path;
    let made = c
        .call(&format!(
            "<m:CreateItem MessageDisposition=\"SaveOnly\"><m:SavedItemFolderId><t:FolderId Id=\"{}\"/></m:SavedItemFolderId><m:Items>\
             <t:Message><t:Subject>Filed by a rule</t:Subject><t:Body BodyType=\"Text\">x</t:Body></t:Message></m:Items></m:CreateItem>",
            crate::ews::esc(&filed)
        ))
        .unwrap();
    let item = made.find("ItemId").and_then(|i| i.attr("Id")).map(str::to_string).expect("made");
    let came = Instant::now();
    let in_filed = || store.load_folder("x2", &path).map(|f| f.messages.iter().any(|m| m.subject == "Filed by a rule")).unwrap_or(false);
    let end = Instant::now() + Duration::from_secs(60);
    while !in_filed() && Instant::now() < end {
        let _ = pool.wait(Duration::from_millis(200));
    }
    let _ = c.call(&format!(
        "<m:DeleteItem DeleteType=\"HardDelete\"><m:ItemIds><t:ItemId Id=\"{}\"/></m:ItemIds></m:DeleteItem>",
        crate::ews::esc(&item)
    ));
    assert!(in_filed(), "the message in {path} within a minute");
    eprintln!("came in {:.1} s", came.elapsed().as_secs_f32());
}

/// Issue #161: a task made here is made in the mailbox's Tasks folder and read back with its
/// days, status, priority, reminder, categories and notes; changed (done, its start and
/// reminder taken off) it's changed in place; deleted, it's gone. What it leaves is its own.
#[test]
#[ignore = "needs a real Exchange server in MAILCRAFT_REAL_EWS"]
fn real_ews_tasks() {
    use mailcraft_model::pim::{StoredTask, Task, TaskPriority, TaskStatus, Tombstone};
    let Some((url, user, password)) = server() else {
        eprintln!("MAILCRAFT_REAL_EWS not set: skipped");
        return;
    };
    let day = |d: u32| mailcraft_model::dates::days_from_civil(2026, 11, d) * 86_400;
    let c = crate::ews::Client::connect(&url, &user, &password, &user).unwrap();
    let title = format!("MailCraft task {}", std::process::id());
    let task = Task {
        uid: "t-real".into(),
        title: title.clone(),
        notes: "Reels 2 and 3\nwarmer".into(),
        start: Some(day(18)),
        due: Some(day(20)),
        status: TaskStatus::InProgress,
        percent: 50,
        priority: TaskPriority::High,
        categories: vec!["Clients".into()],
        reminder: Some(day(20) + 8 * 3600),
        modified: 5,
        ..Default::default()
    };
    let pim = Pim { tasks: vec![StoredTask { task, dirty: true, ..Default::default() }], ..Default::default() };
    let sync = crate::ews_pim::sync(&c, &pim, 1_792_000_000).unwrap();
    assert!(sync.errors.is_empty(), "{:?}", sync.errors);
    let [pushed] = sync.pushed_tasks.as_slice() else { panic!("{:?}", sync.pushed_tasks) };
    let read = sync.tasks.clone().expect("the Tasks folder");
    let mine = read.iter().find(|t| t.href == pushed.href).unwrap_or_else(|| panic!("{title} not read back: {read:?}"));
    eprintln!("read back: {:?}", mine.task);
    let t = &mine.task;
    assert_eq!((t.title.as_str(), t.notes.as_str()), (title.as_str(), "Reels 2 and 3\nwarmer"));
    assert_eq!((t.start, t.due), (Some(day(18)), Some(day(20))));
    assert_eq!((t.status, t.percent, t.priority), (TaskStatus::InProgress, 50, TaskPriority::High));
    assert_eq!((t.categories.clone(), t.reminder), (vec!["Clients".to_string()], Some(day(20) + 8 * 3600)));

    // Done, its start and reminder taken off: changed in place.
    let mut done = mine.clone();
    done.task.set_done(true, day(19));
    (done.task.start, done.task.reminder, done.dirty) = (None, None, true);
    let sync = crate::ews_pim::sync(&c, &Pim { tasks: vec![done], ..Default::default() }, 1_792_000_100).unwrap();
    assert!(sync.errors.is_empty(), "{:?}", sync.errors);
    let after = sync.tasks.unwrap().into_iter().find(|x| x.href == mine.href).expect("the same item");
    eprintln!("after: {:?}", after.task);
    assert!(after.task.is_done() && after.task.percent == 100);
    assert_eq!((after.task.start, after.task.reminder, after.task.due), (None, None, Some(day(20))));

    // Deleted: gone.
    let pim = Pim { deleted: vec![Tombstone { href: mine.href.clone(), ..Default::default() }], ..Default::default() };
    let sync = crate::ews_pim::sync(&c, &pim, 1_792_000_200).unwrap();
    assert_eq!(sync.deleted, std::slice::from_ref(&mine.href));
    assert!(sync.tasks.unwrap().iter().all(|x| x.href != mine.href));
}

/// Issue #191: what the list and a big message's text first ask for is all in the schema
/// (`message:InReplyTo` isn't, and on-premises Exchange refused it): a reply with a 1.5 MB file is
/// listed with what it answers, and opened its text comes first, then all of it, with no failure
/// said in its place. The message is made in the Inbox from its source, then deleted again.
#[test]
#[ignore = "needs a real Exchange server in MAILCRAFT_REAL_EWS"]
fn real_ews_big_reply_comes_as_its_text_first() {
    let Some((url, user, password)) = server() else {
        eprintln!("MAILCRAFT_REAL_EWS not set: skipped");
        return;
    };
    let c = crate::ews::Client::connect(&url, &user, &password, &user).unwrap();
    let tag = std::process::id();
    // 1.5 MB as Exchange counts it (the file's bytes, not their base64).
    let file = "QUJD".repeat(500_000);
    let raw = format!(
        "From: Sam <sam@ews.test>\r\nTo: {user}\r\nSubject: Rushes {tag}\r\nMessage-ID: <rushes{tag}@ews.test>\r\n\
         In-Reply-To: <asked{tag}@ews.test>\r\nReferences: <asked{tag}@ews.test>\r\nDate: Mon, 16 Nov 2026 09:00:00 +0000\r\n\
         MIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=\"B\"\r\n\r\n\
         --B\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nThe notes for the rushes.\r\n\
         --B\r\nContent-Type: video/quicktime; name=\"reel.mov\"\r\nContent-Disposition: attachment; filename=\"reel.mov\"\r\n\
         Content-Transfer-Encoding: base64\r\n\r\n{file}\r\n--B--\r\n"
    );
    let made = c
        .call(&format!(
            "<m:CreateItem MessageDisposition=\"SaveOnly\"><m:SavedItemFolderId><t:DistinguishedFolderId Id=\"inbox\"/></m:SavedItemFolderId>\
             <m:Items><t:Message><t:MimeContent>{}</t:MimeContent>\
             <t:ExtendedProperty><t:ExtendedFieldURI PropertyTag=\"0x0E07\" PropertyType=\"Integer\"/><t:Value>1</t:Value></t:ExtendedProperty>\
             </t:Message></m:Items></m:CreateItem>",
            mailcraft_model::base64::encode(raw.as_bytes())
        ))
        .unwrap();
    let item = made.find("ItemId").and_then(|i| i.attr("Id")).map(str::to_string).expect("made");
    let store = crate::fake::store("real-ews-text-first");
    crate::fakes::fake_dav::looked_for_today(&store, "x3");
    let mut pool = Pool::new(store.clone(), std::sync::Arc::new(MemoryCredentials::default()));
    pool.opts.prefetch = 0;
    let account = AccountConfig { id: "x3".into(), email: user.clone(), username: user, ews: Some(url), ..Default::default() };
    pool.submit(Job::Start { account, password: Some(Secret(password)) });
    let subject = format!("Rushes {tag}");
    let listed = || store.load_folder("x3", "INBOX").ok().and_then(|f| f.messages.into_iter().find(|m| m.subject == subject));
    let end = Instant::now() + Duration::from_secs(300);
    while listed().is_none() {
        assert!(Instant::now() < end, "listed within five minutes");
        if let Some(Event::Status { status: SyncStatus::Error { error, .. }, .. }) = pool.wait(Duration::from_millis(100)) {
            panic!("the account failed: {error}");
        }
    }
    let m = listed().unwrap();
    eprintln!("listed: {} bytes, files {}, answers {:?}, references {:?}", m.size, m.has_attachments, m.in_reply_to, m.references);
    pool.submit(Job::FetchBody { key: m.key.clone() });
    let (mut bodies, mut failed) = (Vec::new(), Vec::new());
    let end = Instant::now() + Duration::from_secs(120);
    while !bodies.last().is_some_and(|b: &Vec<u8>| !mailcraft_mime::is_text_first(b)) && failed.is_empty() {
        assert!(Instant::now() < end, "opened within two minutes");
        match pool.wait(Duration::from_millis(100)) {
            Some(Event::Body { key, raw }) if key == m.key => bodies.push(raw),
            Some(Event::BodyFailed { key, error }) if key == m.key => failed.push(error),
            _ => {}
        }
    }
    let _ = c.call(&format!(
        "<m:DeleteItem DeleteType=\"HardDelete\"><m:ItemIds><t:ItemId Id=\"{}\"/></m:ItemIds></m:DeleteItem>",
        crate::ews::esc(&item)
    ));
    assert!(failed.is_empty(), "{failed:?}");
    eprintln!("came as {:?}", bodies.iter().map(|b| (b.len(), mailcraft_mime::is_text_first(b))).collect::<Vec<_>>());
    assert!(m.has_attachments && m.size > 1 << 20, "{} bytes", m.size);
    assert_eq!(bodies.len(), 2, "its text, then all of it");
    assert!(mailcraft_mime::is_text_first(&bodies[0]));
    let first = mailcraft_mime::parse(&bodies[0]).unwrap();
    assert!(first.text.contains("The notes for the rushes."), "{}", first.text);
    assert_eq!(m.in_reply_to, format!("asked{tag}@ews.test"), "what it answers, listed");
}

/// Issue #197: a folder marked read on the server, mail never synced here too. Three unread
/// messages made in a folder of their own (`Unread`, made when there's none), marked read with
/// one change, then deleted again.
#[test]
#[ignore = "needs a real Exchange server in MAILCRAFT_REAL_EWS"]
fn real_ews_folder_marked_read() {
    let Some((url, user, password)) = server() else {
        eprintln!("MAILCRAFT_REAL_EWS not set: skipped");
        return;
    };
    let c = crate::ews::Client::connect(&url, &user, &password, &user).unwrap();
    let id_of = |doc: &crate::ews::El| doc.find("FolderId").and_then(|f| f.attr("Id")).map(str::to_string);
    let made = c
        .call(
            "<m:CreateFolder><m:ParentFolderId><t:DistinguishedFolderId Id=\"inbox\"/></m:ParentFolderId><m:Folders><t:Folder>\
             <t:DisplayName>Unread</t:DisplayName></t:Folder></m:Folders></m:CreateFolder>",
        )
        .ok()
        .and_then(|d| id_of(&d));
    let folder = made.unwrap_or_else(|| {
        let found = c
            .call(
                "<m:FindFolder Traversal=\"Shallow\"><m:FolderShape><t:BaseShape>Default</t:BaseShape></m:FolderShape>\
                 <m:Restriction><t:IsEqualTo><t:FieldURI FieldURI=\"folder:DisplayName\"/><t:FieldURIOrConstant><t:Constant Value=\"Unread\"/>\
                 </t:FieldURIOrConstant></t:IsEqualTo></m:Restriction><m:ParentFolderIds><t:DistinguishedFolderId Id=\"inbox\"/></m:ParentFolderIds></m:FindFolder>",
            )
            .unwrap();
        id_of(&found).expect("the Unread folder")
    });
    let unread = |c: &crate::ews::Client| -> usize {
        let doc = c
            .call(&format!(
                "<m:FindItem Traversal=\"Shallow\"><m:ItemShape><t:BaseShape>IdOnly</t:BaseShape></m:ItemShape>\
                 <m:Restriction><t:IsEqualTo><t:FieldURI FieldURI=\"message:IsRead\"/><t:FieldURIOrConstant><t:Constant Value=\"false\"/>\
                 </t:FieldURIOrConstant></t:IsEqualTo></m:Restriction><m:ParentFolderIds><t:FolderId Id=\"{}\"/></m:ParentFolderIds></m:FindItem>",
                crate::ews::esc(&folder)
            ))
            .unwrap();
        let mut all = Vec::new();
        doc.all("Message", &mut all);
        all.len()
    };
    for n in 0..3 {
        c.call(&format!(
            "<m:CreateItem MessageDisposition=\"SaveOnly\"><m:SavedItemFolderId><t:FolderId Id=\"{}\"/></m:SavedItemFolderId><m:Items>\
             <t:Message><t:Subject>Unread {n}</t:Subject><t:Body BodyType=\"Text\">x</t:Body><t:IsRead>false</t:IsRead></t:Message></m:Items></m:CreateItem>",
            crate::ews::esc(&folder)
        ))
        .unwrap();
    }
    assert_eq!(unread(&c), 3, "made unread");
    let store = crate::fake::store("real-ews-read");
    let mut pool = Pool::new(store.clone(), std::sync::Arc::new(MemoryCredentials::default()));
    let account = AccountConfig { id: "x5".into(), email: user.clone(), username: user, ews: Some(url), ..Default::default() };
    pool.submit(Job::Start { account, password: Some(Secret(password)) });
    let end = Instant::now() + Duration::from_secs(300);
    let path = loop {
        assert!(Instant::now() < end, "the folder listed");
        let _ = pool.wait(Duration::from_millis(200));
        if let Some(f) = store.load_folders("x5").ok().and_then(|l| l.into_iter().find(|f| f.path.ends_with("Unread"))) {
            break f.path;
        }
    };
    pool.submit(Job::Apply { account: "x5".into(), op: mailcraft_model::Op::MarkFolderRead { path } });
    let end = Instant::now() + Duration::from_secs(60);
    while unread(&c) > 0 && Instant::now() < end {
        let _ = pool.wait(Duration::from_millis(200));
    }
    let left = unread(&c);
    let _ = c.call(&format!(
        "<m:DeleteFolder DeleteType=\"HardDelete\"><m:FolderIds><t:FolderId Id=\"{}\"/></m:FolderIds></m:DeleteFolder>",
        crate::ews::esc(&folder)
    ));
    assert_eq!(left, 0, "all read on the server");
}

/// Issue #204: a weekly series of six made on the server, its second occurrence moved to the
/// Tuesday afternoon with a subject of its own and its third deleted, read as the series with
/// the third taken out and the moved one standing for the second; then deleted again.
#[test]
#[ignore = "needs a real Exchange server in MAILCRAFT_REAL_EWS"]
fn real_ews_changed_occurrences() {
    let Some((url, user, password)) = server() else {
        eprintln!("MAILCRAFT_REAL_EWS not set: skipped");
        return;
    };
    let c = crate::ews::Client::connect(&url, &user, &password, &user).unwrap();
    let id_of = |doc: &crate::ews::El| {
        let id = doc.find("ItemId")?;
        Some((id.attr("Id")?.to_string(), id.attr("ChangeKey").unwrap_or("").to_string()))
    };
    let made = c
        .call(
            "<m:CreateItem SendMeetingInvitations=\"SendToNone\"><m:SavedItemFolderId><t:DistinguishedFolderId Id=\"calendar\"/>\
             </m:SavedItemFolderId><m:Items><t:CalendarItem><t:Subject>Series 204</t:Subject>\
             <t:Start>2026-10-05T09:00:00Z</t:Start><t:End>2026-10-05T10:00:00Z</t:End>\
             <t:Recurrence><t:WeeklyRecurrence><t:Interval>1</t:Interval><t:DaysOfWeek>Monday</t:DaysOfWeek></t:WeeklyRecurrence>\
             <t:NumberedRecurrence><t:StartDate>2026-10-05Z</t:StartDate><t:NumberOfOccurrences>6</t:NumberOfOccurrences></t:NumberedRecurrence>\
             </t:Recurrence></t:CalendarItem></m:Items></m:CreateItem>",
        )
        .unwrap();
    let (series, _) = id_of(&made).expect("the series made");
    let occurrence = |n: usize| {
        let doc = c
            .call(&format!(
                "<m:GetItem><m:ItemShape><t:BaseShape>IdOnly</t:BaseShape></m:ItemShape><m:ItemIds>\
                 <t:OccurrenceItemId RecurringMasterId=\"{}\" InstanceIndex=\"{n}\"/></m:ItemIds></m:GetItem>",
                crate::ews::esc(&series)
            ))
            .unwrap();
        id_of(&doc).expect("the occurrence")
    };
    let (second, key) = occurrence(2);
    c.call_all(&format!(
        "<m:UpdateItem ConflictResolution=\"AlwaysOverwrite\" SendMeetingInvitationsOrCancellations=\"SendToNone\"><m:ItemChanges>\
         <t:ItemChange><t:ItemId Id=\"{}\" ChangeKey=\"{}\"/><t:Updates>\
         <t:SetItemField><t:FieldURI FieldURI=\"item:Subject\"/><t:CalendarItem><t:Subject>Series 204, Tuesday</t:Subject></t:CalendarItem></t:SetItemField>\
         <t:SetItemField><t:FieldURI FieldURI=\"calendar:Start\"/><t:CalendarItem><t:Start>2026-10-13T14:00:00Z</t:Start></t:CalendarItem></t:SetItemField>\
         <t:SetItemField><t:FieldURI FieldURI=\"calendar:End\"/><t:CalendarItem><t:End>2026-10-13T15:00:00Z</t:End></t:CalendarItem></t:SetItemField>\
         </t:Updates></t:ItemChange></m:ItemChanges></m:UpdateItem>",
        crate::ews::esc(&second),
        crate::ews::esc(&key)
    ))
    .unwrap();
    let (third, _) = occurrence(3);
    c.call_all(&format!(
        "<m:DeleteItem DeleteType=\"HardDelete\" SendMeetingCancellations=\"SendToNone\"><m:ItemIds><t:ItemId Id=\"{}\"/></m:ItemIds></m:DeleteItem>",
        crate::ews::esc(&third)
    ))
    .unwrap();

    let sync = crate::ews_pim::sync(&c, &Pim::default(), 1_792_000_000);
    let _ = c.call(&format!(
        "<m:DeleteItem DeleteType=\"HardDelete\" SendMeetingCancellations=\"SendToNone\"><m:ItemIds><t:ItemId Id=\"{}\"/></m:ItemIds></m:DeleteItem>",
        crate::ews::esc(&series)
    ));
    let sync = sync.unwrap();
    assert!(sync.errors.is_empty(), "{:?}", sync.errors);
    let t = |s: &str| mailcraft_jmap::utc_date(s).unwrap();
    let ours: Vec<_> = sync.events.iter().map(|e| &e.event).filter(|e| e.summary.starts_with("Series 204")).collect();
    for e in &ours {
        eprintln!("{:?} start {} recurrence-id {:?} exdates {:?} rule {:?} uid {}", e.summary, e.start, e.recurrence_id, e.exdates, e.rrule, e.uid);
    }
    let series = ours.iter().find(|e| e.recurrence_id.is_none()).expect("the series");
    let moved = ours.iter().find(|e| e.recurrence_id.is_some()).expect("the moved occurrence");
    assert_eq!(ours.len(), 2);
    assert_eq!(series.exdates, [t("2026-10-19T09:00:00Z")]);
    assert_eq!(
        (moved.recurrence_id, moved.start, moved.summary.as_str()),
        (Some(t("2026-10-12T09:00:00Z")), t("2026-10-13T14:00:00Z"), "Series 204, Tuesday")
    );
    assert_eq!(moved.uid, series.uid, "the series' own");
    let seen: Vec<i64> =
        mailcraft_model::pim::ical::occurrences(&ours, t("2026-10-05T00:00:00Z"), t("2026-11-20T00:00:00Z")).iter().map(|o| o.start).collect();
    assert_eq!(seen, ["2026-10-05T09:00:00Z", "2026-10-13T14:00:00Z", "2026-10-26T09:00:00Z", "2026-11-02T09:00:00Z", "2026-11-09T09:00:00Z"].map(t));
}
