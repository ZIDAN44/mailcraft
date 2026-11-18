//! Issue #145: an Exchange account's look for new mail between full syncs reads every folder the
//! server says changed, not the Inbox alone (mail an Inbox rule files elsewhere came only with
//! the next full sync, a quarter of an hour on); and its calendars and contacts, read a step at a
//! time, don't hold that look or Send / Receive up for the minutes they take.

use mailcraft_model::pim::{PIM_DOC, Pim};

use super::*;

/// How often these tests' accounts look for new mail (their full sync stays a quarter of an hour
/// away: what comes, the looks bring).
const LOOK: Duration = Duration::from_millis(150);

struct Mailbox {
    pool: crate::worker::Pool,
    data: Arc<Mutex<Data>>,
    store: Store,
    projects: String,
    _server: Server,
}

/// A mailbox with an Inbox, a Projects folder and an Archive, as `tweak` leaves it, its account
/// (`id`) online, its first full sync and calendars done. `shared`: calendars of others it opens.
fn mailbox(id: &str, shared: &[&str], tweak: impl FnOnce(&mut Data)) -> Mailbox {
    crate::worker::POLL_FOR.lock().unwrap().push((id.into(), LOOK));
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let projects = d.folder("Projects", None, None, "IPF.Note");
    d.folder("Archive", None, None, "IPF.Note");
    d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    d.folder("Contacts", None, Some("contacts"), "IPF.Contact");
    tweak(&mut d);
    let server = serve(d);
    let data = server.data.clone();
    let store = Store::open(crate::data_files::scratch::scratch(&format!("ews-watch-{id}"))).unwrap();
    // Its calendars read before: read after the mail, as at every full sync but the first.
    let pim = Pim { dav_checked: Some(crate::worker::now()), shared: shared.iter().map(|s| s.to_string()).collect(), ..Default::default() };
    store.save_account_doc(id, PIM_DOC, &pim).unwrap();
    let mut pool = crate::worker::Pool::new(store.clone(), Arc::new(crate::worker::MemoryCredentials::default()));
    pool.submit(Job::Start { account: account(id, &server.url), password: Some(Secret(PASSWORD.into())) });
    until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    Mailbox { pool, data, store, projects, _server: server }
}

fn mail(subject: &str) -> Vec<u8> {
    let id = subject.replace(' ', "-");
    format!("From: ana@contoso.example\r\nTo: riley@contoso.example\r\nSubject: {subject}\r\nMessage-ID: <{id}@contoso.example>\r\n\r\nHi\r\n")
        .into_bytes()
}

fn subjects(store: &Store, account: &str, folder: &str) -> Vec<String> {
    store.load_folder(account, folder).map(|f| f.messages.iter().map(|m| m.subject.clone()).collect()).unwrap_or_default()
}

/// The folder listings asked for: one at the start of each full sync.
fn listings(data: &Arc<Mutex<Data>>) -> usize {
    data.lock().unwrap().calls.iter().filter(|c| *c == "FindFolder").count()
}

#[test]
fn mail_a_rule_filed_comes_with_the_next_look() {
    let m = mailbox("watch-rule", &[], |_| {});
    until(&m.pool, "the calendars read", |e| matches!(e, Event::DavSynced { .. }));
    let full_syncs = listings(&m.data);
    // An Inbox rule moved it into Projects as it came.
    m.data.lock().unwrap().deliver(&m.projects, &mail("Filed by a rule"), false, "2026-11-18T08:30:00Z");
    until(&m.pool, "Projects read again", |e| matches!(e, Event::FolderChanged { folder, .. } if folder == "Projects"));
    eventually("the message in Projects", || subjects(&m.store, "watch-rule", "Projects").contains(&"Filed by a rule".to_string()));
    // By a look, not a full sync.
    assert_eq!(listings(&m.data), full_syncs);
    assert!(m.data.lock().unwrap().get_events_asked > 0);
}

/// Mail marked read or unread in another program, in a folder a rule files into, changes here
/// with the next look too (the reporter's: it waited for the full sync, as new mail did).
#[test]
fn a_mark_made_elsewhere_comes_with_the_next_look() {
    let m = mailbox("watch-mark", &[], |d| {
        let projects = d.by_name("Projects").unwrap().id.clone();
        d.deliver(&projects, &mail("Read elsewhere"), false, "2026-11-18T08:00:00Z");
    });
    until(&m.pool, "the calendars read", |e| matches!(e, Event::DavSynced { .. }));
    let seen = |store: &Store| -> Option<bool> {
        let f = store.load_folder("watch-mark", "Projects").ok()?;
        f.messages.iter().find(|s| s.subject == "Read elsewhere").map(|s| s.flags.seen)
    };
    assert_eq!(seen(&m.store), Some(false));
    let full_syncs = listings(&m.data);
    // Read in another program.
    m.data.lock().unwrap().items.iter_mut().filter(|i| i.folder == m.projects).for_each(|i| i.read = true);
    eventually("read here too", || seen(&m.store) == Some(true));
    assert_eq!(listings(&m.data), full_syncs, "by a look, not a full sync");
}

/// A subscription gone (it lapsed while the computer slept, the server restarted) is made again,
/// and what comes after it is told as before.
#[test]
fn a_subscription_gone_is_made_again() {
    let m = mailbox("watch-lapse", &[], |_| {});
    eventually("subscribed", || m.data.lock().unwrap().subscriptions.len() == 1);
    m.data.lock().unwrap().drop_subscriptions();
    eventually("subscribed again", || m.data.lock().unwrap().subscriptions.len() == 1);
    m.data.lock().unwrap().deliver(&m.projects, &mail("After the restart"), false, "2026-11-18T08:40:00Z");
    eventually("the message in Projects", || subjects(&m.store, "watch-lapse", "Projects").contains(&"After the restart".to_string()));
}

/// A server that won't subscribe to every folder at once (Exchange 2010 before its first service
/// pack) is subscribed to each by its id; one that tells its events a few at a time is asked for
/// the rest at once.
#[test]
fn a_server_that_says_less_at_a_time_is_followed() {
    let m = mailbox("watch-each", &[], |d| {
        d.subscribe_all_refused = true;
        d.events_per_answer = Some(1);
    });
    eventually("subscribed", || m.data.lock().unwrap().subscriptions.len() == 1);
    assert!(!m.data.lock().unwrap().subscriptions[0].folders.is_empty(), "to the folders by their ids");
    {
        let mut d = m.data.lock().unwrap();
        let (inbox, archive) = (d.by_name("Inbox").unwrap().id.clone(), d.by_name("Archive").unwrap().id.clone());
        let projects = m.projects.clone();
        d.deliver(&inbox, &mail("One"), false, "2026-11-18T08:50:00Z");
        d.deliver(&projects, &mail("Two"), false, "2026-11-18T08:50:00Z");
        d.deliver(&archive, &mail("Three"), false, "2026-11-18T08:50:00Z");
    }
    eventually("all three read", || {
        subjects(&m.store, "watch-each", "INBOX").contains(&"One".to_string())
            && subjects(&m.store, "watch-each", "Projects").contains(&"Two".to_string())
            && subjects(&m.store, "watch-each", "Archive").contains(&"Three".to_string())
    });
}

/// Calendars that take a while to read (two of colleagues', slow): new mail is looked at between
/// them, not after them all.
#[test]
fn new_mail_doesnt_wait_for_the_calendars() {
    let mut m = mailbox("watch-pim", &["ana@contoso.example", "ben@contoso.example"], |d| d.pim_slow = Some(Duration::from_millis(400)));
    until(&m.pool, "the calendars read", |e| matches!(e, Event::DavSynced { .. }));
    // The next full sync's calendars: begun at once by Send / Receive, mail coming meanwhile.
    m.pool.submit(Job::SyncAll { account: "watch-pim".into() });
    until(&m.pool, "online again", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    let inbox = m.data.lock().unwrap().by_name("Inbox").unwrap().id.clone();
    m.data.lock().unwrap().deliver(&inbox, &mail("While the calendars are read"), false, "2026-11-18T09:00:00Z");
    let seen = until(&m.pool, "the calendars read again", |e| matches!(e, Event::DavSynced { .. }));
    assert!(
        subjects(&m.store, "watch-pim", "INBOX").contains(&"While the calendars are read".to_string()),
        "read before the calendars were done: {seen:?}"
    );
}

/// Send / Receive while the calendars are read: the mail read then (the status saying so), the
/// calendars going on after, not read all over again.
#[test]
fn send_receive_doesnt_wait_for_the_calendars() {
    let mut m = mailbox("watch-f9", &["ana@contoso.example", "ben@contoso.example", "cleo@contoso.example"], |d| {
        d.pim_slow = Some(Duration::from_millis(400));
    });
    until(&m.pool, "the calendars read", |e| matches!(e, Event::DavSynced { .. }));
    m.pool.submit(Job::SyncAll { account: "watch-f9".into() });
    until(&m.pool, "online again", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    // The calendars are being read: Send / Receive now.
    m.data.lock().unwrap().deliver(&m.projects, &mail("Pressed F9"), false, "2026-11-18T09:10:00Z");
    m.pool.submit(Job::SyncAll { account: "watch-f9".into() });
    let seen = until(&m.pool, "the calendars read again", |e| matches!(e, Event::DavSynced { .. }));
    // Every folder read (said as it went, then done) before the calendars were.
    let syncing = seen.iter().position(|e| matches!(e, Event::Status { status: SyncStatus::Syncing { .. }, .. }));
    let online = seen.iter().position(|e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    assert!(syncing.is_some() && online.is_some() && syncing < online, "the mail's full sync, while the calendars were read: {seen:?}");
    assert!(subjects(&m.store, "watch-f9", "Projects").contains(&"Pressed F9".to_string()));
    // The calendars not read all over again after (a pass takes two seconds here).
    std::thread::sleep(Duration::from_secs(3));
    let after: Vec<Event> = std::iter::from_fn(|| m.pool.wait(Duration::ZERO)).collect();
    assert!(!after.iter().any(|e| matches!(e, Event::DavSynced { .. })), "{after:?}");
}

/// What a broken or hostile answer to `GetEvents` holds is taken as far as it makes sense.
#[test]
fn a_broken_events_answer_is_read_safely() {
    let ok = "<ResponseMessages><x ResponseClass=\"Success\"><ResponseCode>NoError</ResponseCode>";
    let events =
        |inner: &str| crate::ews::watch_events(&parse(&format!("<Body>{ok}<Notification>{inner}</Notification></x></ResponseMessages></Body>")));
    // Gromox's answer (its response message named `x`, no watermarks).
    let got = events(
        "<MoreEvents>false</MoreEvents><CreatedEvent><ItemId Id=\"I1\"/><ParentFolderId Id=\"F1\"/></CreatedEvent>\
         <ModifiedEvent><FolderId Id=\"F1\"/><ParentFolderId Id=\"ROOT\"/></ModifiedEvent>\
         <MovedEvent><ItemId Id=\"I2\"/><ParentFolderId Id=\"F2\"/><OldParentFolderId Id=\"F1\"/></MovedEvent>",
    )
    .unwrap();
    assert_eq!(got, crate::ews::WatchEvents { folders: vec!["F1".into(), "F2".into()], more: false, watermark: None });
    // An empty id, a huge one, and more than are kept.
    let many: String = (0..2000).map(|n| format!("<CreatedEvent><ParentFolderId Id=\"F{n}\"/></CreatedEvent>")).collect();
    let got = events(&format!("<MoreEvents>true</MoreEvents><CreatedEvent><ParentFolderId Id=\"\"/></CreatedEvent>{many}")).unwrap();
    assert_eq!((got.folders.len(), got.more), (crate::ews::WATCH_CHANGED_MAX, true));
    assert!(events(&format!("<CreatedEvent><ParentFolderId Id=\"{}\"/></CreatedEvent>", "x".repeat(5000))).unwrap().folders.is_empty());
    // Refused: the subscription gone.
    let gone = parse(
        "<Body><ResponseMessages><x ResponseClass=\"Error\"><ResponseCode>ErrorSubscriptionNotFound</ResponseCode></x></ResponseMessages></Body>",
    );
    assert!(crate::ews::watch_events(&gone).is_err());
    assert!(crate::ews::watch_events(&parse("")).is_err());
}
