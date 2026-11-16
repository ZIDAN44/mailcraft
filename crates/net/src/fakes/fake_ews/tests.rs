//! The fake EWS server's own tests: mail, sending and sign-in.

use std::time::{Duration, Instant};

use mailcraft_model::job::{Secret, Vacation};
use mailcraft_model::{AccountConfig, Event, Flag, Job, Op, SyncStatus};
use mailcraft_store::Store;

use super::*;
use crate::fakes::exchange_e2e::{Exchange, OnServer, an_exchange_account_end_to_end};
use crate::waiting::{events_until, eventually, until};

mod calendars;
mod directory;
mod elsewhere;
mod large_folder;
mod pim;

/// Gap #7: searching Exchange through EWS (`QueryString`, and a `Restriction` for the read
/// state) for mail past the window.
#[test]
fn ews_is_searched_past_the_window() {
    let mut d = Data::default();
    let inbox = d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let mut ids = Vec::new();
    for n in 1..=25 {
        ids.push(
            d.deliver(
                &inbox,
                &format!(
                    "From: Zoe Park <zoe@x.example>\r\nTo: me@x.example\r\nSubject: Message {n}\r\nMessage-ID: <s{n}@x>\r\n\r\nBody tag{n}.\r\n"
                )
                .into_bytes(),
                n > 5,
                &format!("2024-05-{n:02}T09:00:00Z"),
            ),
        );
    }
    let server = serve(d);
    let account = account("a1", &server.url);
    let data = server.data.clone();
    crate::fake::search_the_server_past_the_window("ews-search", account, PASSWORD, &|n, read| {
        let mut d = data.lock().unwrap();
        let id = ids[n as usize - 1].clone();
        if read {
            d.items.iter_mut().filter(|m| m.id == id).for_each(|m| m.read = true);
        } else {
            d.items.retain(|m| m.id != id);
        }
    });
}

fn mail(subject: &str, id: &str) -> Vec<u8> {
    format!("From: Zoe Park <zoe@contoso.example>\r\nTo: riley@contoso.example\r\nSubject: {subject}\r\nMessage-ID: <{id}@x>\r\n\r\nHello about {subject}.\r\n")
        .into_bytes()
}

/// The story every Exchange account is checked by, over EWS (with a calendar among the folders,
/// left out of the list).
#[test]
fn an_on_premises_exchange_account_works_through_ews() {
    let mut d = Data::default();
    d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    let server = serve(d);
    let account = AccountConfig { name: "Riley".into(), ..account("a1", &server.url) };
    let away = Vacation {
        enabled: true,
        subject: String::new(),
        text: "Back on Monday".into(),
        from: Some("2026-10-15".into()),
        until: Some("2026-10-20".into()),
        ..Default::default()
    };
    // Another reply for people outside the organisation, to the user's contacts only; then one
    // reply again.
    let split = Vacation { external: Some("Out of office".into()), outside: "known".into(), ..away.clone() };
    an_exchange_account_end_to_end("ews-e2e", account, "wrong", PASSWORD, &[away.clone(), split, away], &server);
}

impl Exchange for Server {
    const TOP: &'static str = "ROOT";
    const MOVES_KEEP_IDS: bool = false;
    const NO_SUCH_RECIPIENT: &'static str = "ErrorInvalidRecipients";
    fn folder(&self, name: &str, parent: Option<&str>, known: Option<&str>) -> String {
        self.data.lock().unwrap().folder(name, parent, known, "IPF.Note")
    }
    fn deliver(&self, folder: &str, raw: &[u8], read: bool, at: &str) -> String {
        self.data.lock().unwrap().deliver(folder, raw, read, at)
    }
    fn unreadable(&self, id: &str) {
        self.data.lock().unwrap().unreadable.push(id.into());
    }
    fn message(&self, id: &str) -> Option<OnServer> {
        let d = self.data.lock().unwrap();
        d.items.iter().find(|i| i.id == id).map(|i| OnServer { read: i.read, flagged: i.flagged, folder: i.folder.clone() })
    }
    fn holds(&self, folder: &str, words: &str) -> bool {
        self.data.lock().unwrap().items.iter().any(|i| i.folder == folder && String::from_utf8_lossy(&i.raw).contains(words))
    }
    fn count(&self, folder: &str) -> usize {
        self.data.lock().unwrap().items.iter().filter(|i| i.folder == folder).count()
    }
    fn parent(&self, name: &str) -> Option<String> {
        self.data.lock().unwrap().by_name(name).map(|f| f.parent.clone())
    }
    fn drafts(&self) -> usize {
        self.data.lock().unwrap().items.iter().filter(|i| i.draft).count()
    }
    fn last_sent(&self) -> Option<Vec<u8>> {
        self.data.lock().unwrap().sent.last().cloned()
    }
    fn replies_are(&self, v: &Vacation) {
        let d = self.data.lock().unwrap();
        if v.external.is_some() {
            assert_eq!((d.oof.outside.as_str(), d.oof.audience.as_str()), ("Out of office", "Known"));
        } else {
            assert_eq!(d.oof.state, "Scheduled");
        }
    }
    fn after_first_sync(&self) {
        assert!(self.data.lock().unwrap().handshakes <= 2, "NTLM signs in the connection once, not every request");
    }
}

/// An automatic reply goes as HTML with the text's own `<`, `>` and `&` escaped in it, so it
/// comes back as written: `Back <soon>` isn't taken for a tag, nor `R&D` for a reference.
#[test]
fn automatic_replies_with_markup_characters_come_back_as_written() {
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    let away = Vacation { enabled: true, text: "Back <soon>\nR&D on Monday".into(), ..Default::default() };
    c.set_vacation(&away).unwrap();
    assert_eq!(server.data.lock().unwrap().oof.inside, "Back &lt;soon&gt;<br>R&amp;D on Monday", "the HTML Exchange is given");
    let back = c.vacation().unwrap();
    assert_eq!(back.text, away.text);
}

/// What's read is what was written, whatever the reply has in it: one line with an `&` and no
/// `<` came back `R&amp;D`, and grew by an `amp;` each time the replies were saved.
#[test]
fn automatic_replies_come_back_as_typed_however_often_they_are_saved() {
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    for typed in crate::exchange::REPLIES_AS_TYPED {
        let away = Vacation { enabled: true, text: typed.into(), external: Some(format!("Outside: {typed}")), ..Default::default() };
        c.set_vacation(&away).unwrap();
        let once = c.vacation().unwrap();
        assert_eq!(once, away, "read after the first save");
        c.set_vacation(&once).unwrap();
        assert_eq!(c.vacation().unwrap(), away, "read after the second save");
    }
    // A reply another program set, as a page: read as HTML.
    server.data.lock().unwrap().oof.inside = "<html><body><div>Back &amp; forth</div><div>R&amp;D</div></body></html>".into();
    assert_eq!(c.vacation().unwrap().text, "Back & forth\nR&D");
}

/// A server (or a proxy before it) naming its sign-in schemes in lower case: its `ntlm` challenge
/// is answered as `NTLM`'s.
#[test]
fn a_server_naming_ntlm_in_lower_case_is_signed_in() {
    let mut d = Data { lower_case_schemes: true, ..Default::default() };
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    assert_eq!(c.mailboxes().unwrap().len(), 1);
    assert!(server.data.lock().unwrap().handshakes >= 1, "signed in with NTLM");
}

/// A proxy can drop an NTLM sign-in while the TCP connection stays open (SmarterMail behind IIS
/// ARR, PR #73): the next request is answered 401 on a connection signed in. It's signed in
/// again, once, and goes through; not taken for a refused password.
#[test]
fn ntlm_reauthenticates_after_a_401_on_an_existing_connection() {
    let mut d = Data { expire_ntlm_once: true, ..Default::default() };
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    assert_eq!(server.data.lock().unwrap().handshakes, 1, "initial sign-in");
    assert_eq!(c.mailboxes().unwrap().len(), 1, "request succeeds after NTLM reauthentication");
    assert_eq!(server.data.lock().unwrap().ntlm_reauthentications, 1, "one successful reauthentication on the existing connection");
    assert_eq!(c.mailboxes().unwrap().len(), 1);
    assert_eq!(server.data.lock().unwrap().ntlm_reauthentications, 1, "no repeated authentication expiry");
}

/// A rejected NTLM Type 3 response must fail instead of retrying forever.
#[test]
fn ntlm_rejects_bad_credentials_without_reauthentication_loop() {
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let server = serve(d);
    let result = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), "incorrect-password", "riley@contoso.example");
    assert!(matches!(result, Err(crate::NetError::Auth(_))), "bad credentials must be rejected");
    assert_eq!(server.data.lock().unwrap().handshakes, 0, "no successful NTLM handshake");
}

/// A folder flagged hidden is not offered as mail, nor is anything nested in it at any
/// depth; its siblings and their subfolders still are.
#[test]
fn exchanges_own_folders_are_left_out_of_the_list() {
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let projects = d.folder("Projects", None, None, "IPF.Note");
    let working = d.folder("Working Set", None, None, "IPF.Note");
    let inner = d.folder("Inner", Some(&working), None, "IPF.Note");
    d.folder("Deeper", Some(&inner), None, "IPF.Note");
    d.folder("2024", Some(&projects), None, "IPF.Note");
    d.flagged_hidden.push(working);
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    let mut names: Vec<String> = c.mailboxes().unwrap().into_iter().map(|m| m.name).collect();
    names.sort();
    assert_eq!(names, ["2024", "Inbox", "Projects"]);
}

/// A shared mailbox's folders are told from its other folders as the account's own are: over
/// several pages, its calendar and contacts aren't mail, a folder flagged hidden stays out with
/// what's in it, its Outbox too; a class of mail of its own is mail. Each sits under its parent,
/// those at its top under the folder named for the mailbox.
#[test]
fn a_shared_mailbox_lists_its_mail_folders_as_the_account_does() {
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let owner = "sales@contoso.example";
    let inbox = d.shared_mailbox(SharedBox { address: owner.into(), name: "Sales".into(), automap: true, legacy: false, granted: true });
    let root = d.folders.iter().find(|f| f.owner == owner && f.known.as_deref() == Some("msgfolderroot")).map(|f| f.id.clone()).unwrap();
    d.shared_folder(owner, "Quotes", &inbox);
    let projects = d.shared_folder(owner, "Projects", &root);
    d.shared_folder(owner, "2025", &projects);
    let working = d.shared_folder(owner, "Working Set", &root);
    d.shared_folder(owner, "Inner", &working);
    d.flagged_hidden.push(working);
    let calendar = d.shared_folder(owner, "Calendar", &root);
    let contacts = d.shared_folder(owner, "Contacts", &root);
    let faxes = d.shared_folder(owner, "Faxes", &root);
    for f in d.folders.iter_mut() {
        f.class = if f.id == calendar {
            "IPF.Appointment".into()
        } else if f.id == contacts {
            "IPF.Contact".into()
        } else if f.id == faxes {
            "IPF.Note.Fax".into()
        } else {
            f.class.clone()
        };
    }
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    let mut st = crate::jmap::State::default();
    let mut paths: Vec<String> = crate::ews::list_folders(&c, &mut st, "a1").unwrap().into_iter().map(|f| f.path).collect();
    paths.sort();
    assert_eq!(paths, ["INBOX", "Sales", "Sales/Faxes", "Sales/Inbox", "Sales/Inbox/Quotes", "Sales/Projects", "Sales/Projects/2025"]);
    assert!(server.data.lock().unwrap().calls.iter().filter(|c| *c == "FindFolder").count() >= 4, "its folders came in pages");
}

/// Folder edits queued back to back (create, rename, delete, and a move into a new
/// folder): each must see the folder ids the previous one produced, within one replay.
#[test]
fn folder_edits_queued_together_each_find_their_folder() {
    let mut d = Data::default();
    let inbox = d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.deliver(&inbox, &mail("Storyboard", "m1"), false, "2024-05-06T09:00:00Z");
    let server = serve(d);
    let data = server.data.clone();
    let store = crate::fake::store("ews-folder-burst");
    let mut pool = crate::worker::Pool::new(store.clone(), Arc::new(crate::worker::MemoryCredentials::default()));
    let account = account("a1", &server.url);
    pool.submit(Job::Start { account, password: Some(Secret(PASSWORD.into())) });
    until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    let key = store.load_folder("a1", "INBOX").unwrap().messages[0].key.clone();
    // All submitted at once, before the first reaches the server.
    for op in [
        Op::CreateFolder { path: "Drafts 2".into() },
        Op::RenameFolder { from: "Drafts 2".into(), to: "Clients".into() },
        Op::Move { keys: vec![key], to: "Clients".into() },
        Op::CreateFolder { path: "Old".into() },
        Op::DeleteFolder { path: "Old".into() },
    ] {
        pool.submit(Job::Apply { account: "a1".into(), op });
    }
    // The changes waiting read as the worker writes them: on Windows a read while it replaces or
    // removes the file can be refused for a moment, which is not yet, not a failure.
    eventually("all made on the server", || {
        let d = data.lock().unwrap();
        let clients = d.by_name("Clients").map(|f| f.id.clone());
        clients.is_some_and(|c| d.items.iter().any(|i| i.folder == c))
            && d.by_name("Drafts 2").is_none()
            && d.by_name("Old").is_none()
            && store.load_ops("a1").is_ok_and(|ops| ops.is_empty())
    });
    drop(pool);
}

/// Flag a message while a slow full sync runs, then shut the pool down: the flag is in the
/// cache and ops.json straight away, and the next session delivers it to the server.
#[test]
fn a_flag_set_mid_sync_is_not_lost() {
    let mut d = Data::default();
    let inbox = d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.folder("Projects", None, None, "IPF.Note");
    d.folder("Archive 2024", None, None, "IPF.Note");
    let id = d.deliver(&inbox, &mail("Storyboard", "m1"), false, "2024-05-06T09:00:00Z");
    let server = serve(d);
    let data = server.data.clone();
    let dir = crate::data_files::scratch::scratch("ews-changes");
    let store = Store::open(&dir).unwrap();
    let account = AccountConfig { name: "Riley".into(), ..account("a1", &server.url) };
    let start = |store: &Store| {
        let mut pool = crate::worker::Pool::new(store.clone(), Arc::new(crate::worker::MemoryCredentials::default()));
        pool.submit(Job::Start { account: account.clone(), password: Some(Secret(PASSWORD.into())) });
        pool
    };
    let mut pool = start(&store);
    until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    let key = store.load_folder("a1", "INBOX").unwrap().messages[0].key.clone();

    // Hold the server's listing of a folder's mail, start a full sync, flag the message during
    // it.
    data.lock().unwrap().hold = true;
    pool.submit(Job::SyncAll { account: "a1".into() });
    eventually("the sync held on a folder", || data.lock().unwrap().held > 0);
    let op = Op::SetFlag { keys: vec![key.clone()], flag: Flag::Flagged, on: true };
    pool.submit(Job::Apply { account: "a1".into(), op: op.clone() });
    let flagged = || store.load_folder("a1", "INBOX").unwrap().messages.iter().any(|m| m.key == key && m.flags.flagged);
    // Made on the pool's thread for changes (not the window's), at once: not behind the sync,
    // which is held until the test lets it go. (A read of the changes as they're written can be
    // refused for a moment on Windows: not yet.)
    let seen = events_until(&pool, "the flag made here", |_| flagged() && store.load_ops("a1").is_ok_and(|ops| !ops.is_empty()));
    let online = |e: &Event| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. });
    assert!(!seen.iter().any(online), "made here only once the sync was done: {seen:?}");
    assert_eq!(store.load_ops("a1").unwrap(), [op], "ops.json written at once");
    // Shut down mid-sync, the listing held answered; a fresh pool must push the flag and keep
    // showing it.
    drop(pool);
    data.lock().unwrap().hold = false;
    eventually("the held listing answered", || data.lock().unwrap().held == 0);
    let pool = start(&store);
    until(&pool, "online again", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    let on_server = || data.lock().unwrap().items.iter().any(|i| i.id == id && i.flagged);
    events_until(&pool, "the flag on the server", |_| on_server() && store.load_ops("a1").is_ok_and(|ops| ops.is_empty()));
    assert!(flagged(), "the later syncs kept the flag locally");
    drop(pool);
}

/// Mail sent and automatic replies asked for while a big mailbox's first sync is still going
/// go at once: they're done with the sync held until they are.
#[test]
fn sending_and_automatic_replies_dont_wait_for_a_long_sync() {
    let mut d = Data { hold: true, ..Default::default() };
    let inbox = d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.folder("Sent Items", None, Some("sentitems"), "IPF.Note");
    d.folder("Projects", None, None, "IPF.Note");
    d.folder("Archive 2024", None, None, "IPF.Note");
    d.deliver(&inbox, &mail("Storyboard", "m1"), false, "2024-05-06T09:00:00Z");
    let server = serve(d);
    let data = server.data.clone();
    let dir = crate::data_files::scratch::scratch("ews-busy");
    let store = Store::open(&dir).unwrap();
    let mut pool = crate::worker::Pool::new(store, Arc::new(crate::worker::MemoryCredentials::default()));
    let account = AccountConfig { name: "Riley".into(), ..account("a1", &server.url) };
    pool.submit(Job::Start { account, password: Some(Secret(PASSWORD.into())) });
    // The folders are listed; listing their messages is held from here.
    until(&pool, "folders", |e| matches!(e, Event::Folders { .. }));
    eventually("the sync held on a folder", || data.lock().unwrap().held > 0);
    let raw = b"From: riley@contoso.example\r\nTo: zoe@contoso.example\r\nSubject: Quick answer\r\nMessage-ID: <q1@x>\r\n\r\nYes.\r\n".to_vec();
    pool.submit(Job::Send {
        account: "a1".into(),
        id: 7,
        raw,
        from: "riley@contoso.example".into(),
        recipients: vec!["zoe@contoso.example".into()],
        sent_folder: Some("Sent Items".into()),
    });
    pool.submit(Job::GetVacation { account: "a1".into() });
    let sent = |e: &Event| matches!(e, Event::Sent { id: 7, .. });
    let replies = |e: &Event| matches!(e, Event::Vacation { vacation: Some(_), .. });
    let seen =
        events_until(&pool, "the message sent and the replies read, with the sync held", |seen| seen.iter().any(sent) && seen.iter().any(replies));
    let failed = seen.iter().find_map(|e| if let Event::SendFailed { error, .. } = e { Some(error) } else { None });
    assert!(failed.is_none(), "send failed: {failed:?}");
    assert_eq!(data.lock().unwrap().held, 1, "the sync still held");
    assert_eq!(data.lock().unwrap().sent.len(), 1, "sent once");
    // Several at once, and the replies changed meanwhile: each goes once, in order.
    let last_sends = Instant::now();
    for (id, subject) in [(8u64, "Second"), (9, "Third"), (10, "Fourth")] {
        pool.submit(Job::Send {
            account: "a1".into(),
            id,
            raw: format!("From: riley@contoso.example\r\nTo: zoe@contoso.example\r\nSubject: {subject}\r\nMessage-ID: <{id}@x>\r\n\r\nx\r\n")
                .into_bytes(),
            from: "riley@contoso.example".into(),
            recipients: vec!["zoe@contoso.example".into()],
            sent_folder: Some("Sent Items".into()),
        });
    }
    let away = mailcraft_model::job::Vacation { enabled: true, text: "Away".into(), ..Default::default() };
    pool.submit(Job::SetVacation { account: "a1".into(), vacation: away.clone() });
    let done = |seen: &[Event]| -> Vec<u64> { seen.iter().filter_map(|e| if let Event::Sent { id, .. } = e { Some(*id) } else { None }).collect() };
    let set = |e: &Event| matches!(e, Event::Vacation { vacation: Some(v), error: None, .. } if *v == away);
    let seen = events_until(&pool, "three sent and the replies set, with the sync held", |seen| done(seen).len() >= 3 && seen.iter().any(set));
    let failed: Vec<&Event> = seen.iter().filter(|e| matches!(e, Event::SendFailed { .. } | Event::Vacation { error: Some(_), .. })).collect();
    assert!(failed.is_empty(), "{failed:?}");
    assert_eq!(done(&seen), [8, 9, 10]);
    assert_eq!(data.lock().unwrap().held, 1, "the sync still held");
    let subjects: Vec<String> = data.lock().unwrap().sent.iter().map(|r| mailcraft_mime::parse(r).map(|p| p.subject).unwrap_or_default()).collect();
    assert_eq!(subjects, ["Quick answer", "Second", "Third", "Fourth"]);
    // Let go, the sync carries on, and finishes (Sent Items read again once however many went).
    data.lock().unwrap().hold = false;
    until(&pool, "the sync to finish", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    assert_eq!(data.lock().unwrap().sent.len(), 4, "not sent again");
    // Removing the account stops both connections.
    pool.submit(Job::Remove { account: "a1".into() });
    pool.submit(Job::GetVacation { account: "a1".into() });
    until(&pool, "offline answer", |e| matches!(e, Event::Vacation { error: Some(_), .. }));
    // Its threads end before the test's folder goes: the main one still wrote in it after the
    // test's thread ended, and the folder was left behind.
    pool.shutdown_and_wait();
    // And they end with the account, not with the later look at Sent Items each of those sends
    // asked for: that held the main one until 15 s after the send, never sooner.
    let took = last_sends.elapsed();
    assert!(took < Duration::from_secs(10), "the account's threads ended {took:?} after its last sends");
}

#[test]
fn basic_sign_in_when_that_is_what_the_server_offers() {
    let mut d = Data { basic: true, ..Default::default() };
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    assert_eq!(c.mailboxes().unwrap().len(), 1);
    assert!(matches!(crate::ews::Client::connect(&server.url, "x", "y", "e"), Err(crate::NetError::Auth(_))));
}

/// A message whose source the server won't give doesn't stop the folder, over EWS.
#[test]
fn one_unreadable_message_holds_up_nothing() {
    let server = serve(Data::default());
    crate::fakes::exchange_e2e::one_unreadable_message_holds_up_nothing("ews-unreadable", &server, |store| {
        let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
        let mut st = crate::jmap::State::default();
        crate::ews::list_folders(&c, &mut st, "a1").unwrap();
        crate::ews::sync_folder(&c, &mut st, store, "a1", "INBOX", &crate::sync::SyncOptions::default()).unwrap();
    });
}

/// Issue #1: a mailbox made in Dutch showed "Inbox", "Sent Items"… where Outlook shows the
/// names Exchange gave them ("Postvak IN", "Verzonden items"); they're kept now, and the
/// folders are still the account's Inbox, Sent Items… A folder of the user's own is as it was.
#[test]
fn special_folders_keep_the_names_exchange_gives_them() {
    let mut d = Data::default();
    d.folder("Postvak IN", None, Some("inbox"), "IPF.Note");
    d.folder("Verzonden items", None, Some("sentitems"), "IPF.Note");
    d.folder("Verwijderde items", None, Some("deleteditems"), "IPF.Note");
    d.folder("Afgehandeld", None, None, "IPF.Note");
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    let folders = crate::ews::list_folders(&c, &mut crate::jmap::State::default(), "a1").unwrap();
    let named = |role: mailcraft_model::FolderRole| folders.iter().find(|f| f.role == role).map(|f| (f.path.clone(), f.display_name())).unwrap();
    assert_eq!(named(mailcraft_model::FolderRole::Inbox), ("INBOX".to_string(), "Postvak IN".to_string()));
    assert_eq!(named(mailcraft_model::FolderRole::Sent).1, "Verzonden items");
    assert_eq!(named(mailcraft_model::FolderRole::Trash).1, "Verwijderde items");
    let own = folders.iter().find(|f| f.path == "Afgehandeld").unwrap();
    assert_eq!((own.name.as_deref(), own.display_name().as_str()), (None, "Afgehandeld"));
}

/// For looking at the app against an Exchange server whose mailbox was made in Dutch, with
/// a shared mailbox mapped to it, on 127.0.0.1:8085 without TLS (`CONTOSO\riley`, password
/// `pw`): add the account with `"ews": "http://127.0.0.1:8085/EWS/Exchange.asmx"`.
/// Run: `cargo test -p mailcraft-net serve_ews_for_the_app -- --ignored`.
#[test]
#[ignore = "a server for looking at the app, run by hand: it never ends"]
fn serve_ews_for_the_app() {
    // `MAILCRAFT_FAKE_OPEN`: signed in by a front in front of it (gap 5's Kerberos one).
    let mut d = Data { open: std::env::var_os("MAILCRAFT_FAKE_OPEN").is_some(), ..Default::default() };
    let inbox = d.folder("Postvak IN", None, Some("inbox"), "IPF.Note");
    d.folder("Concepten", None, Some("drafts"), "IPF.Note");
    d.folder("Verzonden items", None, Some("sentitems"), "IPF.Note");
    d.folder("Verwijderde items", None, Some("deleteditems"), "IPF.Note");
    d.folder("Ongewenste e-mail", None, Some("junkemail"), "IPF.Note");
    d.folder("Postvak UIT", None, Some("outbox"), "IPF.Note");
    d.folder("Afgehandeld", None, None, "IPF.Note");
    d.deliver(&inbox, &mail("Weekly report", "o1"), false, "2026-10-19T08:00:00Z");
    d.deliver(&inbox, &mail("Re: Weekly report", "o2"), false, "2026-10-19T09:00:00Z");
    // `MAILCRAFT_FAKE_COUNT`: that many older ones in Afgehandeld ("Message 1" the oldest,
    // each body `needle-<n>`), for more mail than is kept (gap #7).
    let count: u32 = std::env::var("MAILCRAFT_FAKE_COUNT").ok().and_then(|n| n.parse().ok()).unwrap_or(0);
    let done = d.by_name("Afgehandeld").map(|f| f.id.clone()).unwrap_or_default();
    for n in 1..=count.min(20_000) {
        let raw = format!(
            "From: Zoe Park <zoe@contoso.example>\r\nTo: riley@contoso.example\r\nSubject: Message {n}\r\nMessage-ID: <old{n}@x>\r\n\r\nOld body needle-{n}.\r\n"
        );
        let at = format!("2025-01-{:02}T{:02}:{:02}:00Z", 1 + n / 1440, n / 60 % 24, n % 60);
        d.deliver(&done, raw.as_bytes(), n % 7 != 0, &at);
    }
    let shared = d.shared_mailbox(SharedBox {
        address: "test@contoso.example".into(),
        name: "Test Grommunio".into(),
        automap: true,
        legacy: false,
        granted: true,
    });
    d.deliver(&shared, &mail("For the team", "s1"), false, "2026-10-19T10:00:00Z");
    // A Teams meeting today, its join link in the HTML body as Teams writes it, and the shared
    // mailbox's calendar with an event of its own.
    let agenda = d.folder("Agenda", None, Some("calendar"), "IPF.Appointment");
    d.folder("Contactpersonen", None, Some("contacts"), "IPF.Contact");
    let teams = parse(
        "<t:CalendarItem><t:Subject>Weekly sync</t:Subject><t:Body BodyType=\"HTML\">&lt;html&gt;&lt;body&gt;&lt;p&gt;Agenda: the release.&lt;/p&gt;             &lt;div&gt;________________________________________________________________________________&lt;/div&gt;             &lt;div&gt;&lt;span style=\"font-size:24px\"&gt;Microsoft Teams &lt;a href=\"https://aka.ms/JoinTeamsMeeting\"&gt;Need help?&lt;/a&gt;&lt;/span&gt;&lt;/div&gt;             &lt;div&gt;&lt;a href=\"https://teams.microsoft.com/l/meetup-join/19%3ameeting_NjA3%40thread.v2/0?context=%7b%22Tid%22%3a%22c1%22%7d\"&gt;             &lt;span style=\"font-size:21px\"&gt;Join the meeting now&lt;/span&gt;&lt;/a&gt;&lt;/div&gt;&lt;div&gt;Meeting ID: 366 254 181 924&lt;/div&gt;             &lt;div&gt;Passcode: Ab3cD9&lt;/div&gt;&lt;/body&gt;&lt;/html&gt;</t:Body>             <t:Start>2026-10-20T13:00:00Z</t:Start><t:End>2026-10-20T13:30:00Z</t:End><t:IsAllDayEvent>false</t:IsAllDayEvent>             <t:Location>Microsoft Teams Meeting</t:Location><t:UID>weekly-sync-1</t:UID></t:CalendarItem>",
    );
    d.pim.push(PimItem::new("E1", agenda.clone(), teams.kids[0].clone()));
    let shared_cal = d.id("S");
    d.folders.push(Fo {
        id: shared_cal.clone(),
        name: "Agenda".into(),
        parent: String::new(),
        class: "IPF.Appointment".into(),
        known: Some("calendar".into()),
        owner: "test@contoso.example".into(),
    });
    let team = parse(
        "<t:CalendarItem><t:Subject>Team planning</t:Subject><t:Start>2026-10-20T09:00:00Z</t:Start><t:End>2026-10-20T10:00:00Z</t:End>             <t:IsAllDayEvent>false</t:IsAllDayEvent><t:UID>team-plan-1</t:UID></t:CalendarItem>",
    );
    d.pim.push(PimItem::new("T1", shared_cal, team.kids[0].clone()));
    // Issue #1: a Google Calendar invitation, its body as Exchange gives it as text (each
    // link written `label<url>`), long links and all.
    let tok = "AKUaPmZ3cDk0bWtnZ2I5NjBhYmRlZjEyMzQ1Njc4OTBhYmNkZWYxMjM0NTY3ODkwYWJjZGVm";
    let eid = "NnQ5MWVxZzdhY2JkMWU0ZXJnaGtnOHFwdmEgc3R1ZGlvQGNvbnRvc28uZXhhbXBsZQ";
    let mut invite = String::from(
        "Join with Google Meet – You have been invited by ana@studio.example to attend an event named Design review on Wednesday 21 Oct 2026 14:00 – 15:00 (Eastern European Time).\n\n\
         <https://meet.google.com/abc-defg-hij>\nJoin with Google Meet\n\nMeeting link\nmeet.google.com/abc-defg-hij<https://meet.google.com/abc-defg-hij?hs=224>\n\
         Join by phone\n(US) +1 555-0100<tel:+1-555-0100%3B618021784%23>\nPIN: 618 021 784#\n\nMore phone numbers<https://tel.meet/abc-defg-hij?pin=6180217840000&hs=1>\n\n\n\
         When\nWednesday 21 Oct 2026 14:00 – 15:00 (Eastern European Time)\nGuests\nAna <mailto:ana@studio.example>\nRiley <mailto:riley@contoso.example>\nView all guest info<https://calendar.google.com/calendar/event?action=VIEW&eid=",
    );
    invite.push_str(&format!("{eid}&tok={tok}&ctz=Europe%2FAmsterdam&hl=en&es=0>\n\nReply for riley@contoso.example\n"));
    for (label, rst) in [("Yes", 1), ("No", 2), ("Maybe", 3)] {
        invite.push_str(&format!(
            "{label}\n\n<https://calendar.google.com/calendar/event?action=RESPOND&eid={eid}&rst={rst}&tok={tok}&ctz=Europe%2FAmsterdam&hl=en&es=0>\n"
        ));
    }
    invite.push_str(&format!(
        "More options\n\n<https://calendar.google.com/calendar/event?action=VIEW&eid={eid}&tok={tok}&ctz=Europe%2FAmsterdam&hl=en&es=0>\n\n\n\n\
         Sent by Google Calendar<https://calendar.google.com/calendar/>\n\nYou are receiving this email because you are an attendee on the event.\n\n\
         Forwarding this invitation could allow any recipient to send a response to the organizer, be added to the guest list, invite others regardless of their own invitation status, or modify your RSVP. Learn more<https://support.google.com/calendar/answer/37135#forwarding>\n"
    ));
    let google = parse(&format!(
        "<t:CalendarItem><t:Subject>Invitation: Design review @ Wed 21 Oct 2026 14:00 - 15:00 (EET)</t:Subject><t:Body BodyType=\"Text\">{}</t:Body>\
         <t:Start>2026-10-21T12:00:00Z</t:Start><t:End>2026-10-21T13:00:00Z</t:End><t:IsAllDayEvent>false</t:IsAllDayEvent>\
         <t:Location>https://meet.google.com/abc-defg-hij</t:Location><t:UID>google-invite-1</t:UID></t:CalendarItem>",
        esc(&invite)
    ));
    d.pim.push(PimItem::new("G1", agenda, google.kids[0].clone()));
    // Issue #9: the organisation's directory, where someone new (Dana Whitfield) starts 45
    // seconds after the server does, for a search that finds her without a restart.
    for (n, a) in [("Alice Moreno", "alice.moreno@contoso.example"), ("Alicia Keyes", "alicia@contoso.example"), ("Bob Stone", "bob@contoso.example")]
    {
        d.directory.push((n.into(), a.into()));
    }
    let server = serve_at(d, "127.0.0.1:8085");
    std::thread::sleep(std::time::Duration::from_secs(45));
    server.data.lock().unwrap().directory.push(("Dana Whitfield".into(), "dana@contoso.example".into()));
    loop {
        std::thread::park();
    }
}
