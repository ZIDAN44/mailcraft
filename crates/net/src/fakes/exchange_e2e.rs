//! An Exchange account's stories, told the same over EWS and over Graph: each fake's tests give
//! them the account (or a way to sync it) and a way to fill and read back their server.

use std::sync::Arc;

use mailcraft_model::job::{Secret, Vacation};
use mailcraft_model::{AccountConfig, Event, Flag, FolderRole, Job, Op, SyncStatus};
use mailcraft_store::Store;

use crate::waiting::{eventually, until};

/// A message as the server holds it.
pub(super) struct OnServer {
    pub read: bool,
    pub flagged: bool,
    /// Its folder's id.
    pub folder: String,
}

/// What the story needs of a fake Exchange server.
pub(super) trait Exchange {
    /// The id of the folder at the top of the mailbox.
    const TOP: &'static str;
    /// Whether a moved message keeps its id (Graph's immutable ids) rather than getting a new one
    /// (EWS).
    const MOVES_KEEP_IDS: bool;
    /// What the error says when a recipient is refused.
    const NO_SUCH_RECIPIENT: &'static str;
    /// A mail folder made (under the folder `parent`, at the top for none; `known` its
    /// well-known name): its id.
    fn folder(&self, name: &str, parent: Option<&str>, known: Option<&str>) -> String;
    /// `raw` delivered into the folder `folder`, read or not, received at `at`: its id.
    fn deliver(&self, folder: &str, raw: &[u8], read: bool, at: &str) -> String;
    /// The message `id`'s source refused from now on.
    fn unreadable(&self, id: &str);
    /// The message `id`, while it's there.
    fn message(&self, id: &str) -> Option<OnServer>;
    /// Whether a message whose source has `words` is in the folder `folder`.
    fn holds(&self, folder: &str, words: &str) -> bool;
    /// How many messages the folder `folder` has.
    fn count(&self, folder: &str) -> usize;
    /// The parent's id of the folder named `name`, while there's one so named.
    fn parent(&self, name: &str) -> Option<String>;
    /// How many drafts the mailbox has.
    fn drafts(&self) -> usize;
    /// The source of the last message sent.
    fn last_sent(&self) -> Option<Vec<u8>>;
    /// Checks the server keeps the automatic replies `v`, just set.
    fn replies_are(&self, v: &Vacation);
    /// Checks of this server's own once the account is first synced.
    fn after_first_sync(&self) {}
    /// Checks of this server's own at the end.
    fn at_the_end(&self) {}
}

/// An Exchange account through the worker: signing in (a refused secret, then the right one),
/// the folder list and first sync, opening, reading, flagging and moving a message, making,
/// renaming and deleting a folder, the automatic replies (each of `replies` set in turn, then a
/// change refused), emptying Deleted Items and deleting for good, sending (with a Bcc, and to
/// someone refused) and a draft replacing its earlier copy. `wrong` is a secret the server
/// refuses, `secret` the one it takes.
pub(super) fn an_exchange_account_end_to_end<S: Exchange>(
    name: &str,
    account: AccountConfig,
    wrong: &str,
    secret: &str,
    replies: &[Vacation],
    server: &S,
) {
    let me = account.email.clone();
    let mail = |subject: &str, id: &str| {
        format!("From: Zoe Park <zoe@contoso.example>\r\nTo: {me}\r\nSubject: {subject}\r\nMessage-ID: <{id}@x>\r\n\r\nHello about {subject}.\r\n")
            .into_bytes()
    };
    let inbox = server.folder("Inbox", None, Some("inbox"));
    server.folder("Drafts", None, Some("drafts"));
    let sent = server.folder("Sent Items", None, Some("sentitems"));
    let trash = server.folder("Deleted Items", None, Some("deleteditems"));
    server.folder("Outbox", None, Some("outbox"));
    let projects = server.folder("Projects", None, None);
    server.folder("2024", Some(&projects), None);
    let first = server.deliver(&inbox, &mail("Storyboard", "m1"), true, "2024-05-06T09:00:00Z");
    let second = server.deliver(&inbox, &mail("Sound mix", "m2"), false, "2024-05-06T10:00:00Z");
    server.deliver(&projects, &mail("Plan", "m3"), true, "2024-05-01T10:00:00Z");

    let store = crate::fakes::fake::store(name);
    let mut pool = crate::worker::Pool::new(store.clone(), Arc::new(crate::worker::MemoryCredentials::default()));
    assert!(!account.is_local() && account.is_api());

    // A secret the server refuses means signing in again.
    pool.submit(Job::Start { account: account.clone(), password: Some(Secret(wrong.into())) });
    until(&pool, "sign in again", |e| matches!(e, Event::Status { status: SyncStatus::NeedsPassword { .. }, .. }));

    pool.submit(Job::Start { account: account.clone(), password: Some(Secret(secret.into())) });
    let seen = until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    let folders = seen
        .iter()
        .rev()
        .find_map(|e| match e {
            Event::Folders { folders, .. } => Some(folders.clone()),
            _ => None,
        })
        .unwrap();
    let mut paths: Vec<&str> = folders.iter().map(|f| f.path.as_str()).collect();
    paths.sort_unstable();
    assert_eq!(
        paths,
        ["Deleted Items", "Drafts", "INBOX", "Projects", "Projects/2024", "Sent Items"],
        "pages followed, children listed; no Outbox, no calendar"
    );
    let role = |p: &str| folders.iter().find(|f| f.path == p).unwrap().role;
    assert_eq!((role("Sent Items"), role("Deleted Items")), (FolderRole::Sent, FolderRole::Trash));
    let inbox_folder = folders.iter().find(|f| f.path == "INBOX").unwrap();
    assert_eq!((inbox_folder.total, inbox_folder.unread), (2, 1));
    let st = store.load_folder("a1", "INBOX").unwrap();
    let subjects: Vec<&str> = st.messages.iter().map(|m| m.subject.as_str()).collect();
    assert_eq!(subjects, ["Sound mix", "Storyboard"], "newest first");
    assert!(st.messages[1].flags.seen && !st.messages[0].flags.seen);
    assert_eq!(st.messages[0].from.email, "zoe@contoso.example");
    assert!(store.has_body(&st.messages[0].key), "the newest sources are downloaded");
    server.after_first_sync();

    // Open a message whose source isn't cached.
    let key = st.messages[0].key.clone();
    store.remove_body(&key);
    pool.submit(Job::FetchBody { key: key.clone() });
    until(&pool, "body", |e| matches!(e, Event::Body { key: k, raw } if *k == key && raw.starts_with(b"From:")));

    // Read, flag, move: Graph keeps the message's id, Exchange's own web service gives it a new one.
    pool.submit(Job::Apply { account: "a1".into(), op: Op::SetFlag { keys: vec![key.clone()], flag: Flag::Seen, on: true } });
    eventually("read on the server", || server.message(&second).is_some_and(|m| m.read));
    pool.submit(Job::Apply { account: "a1".into(), op: Op::SetFlag { keys: vec![key.clone()], flag: Flag::Flagged, on: true } });
    eventually("flagged on the server", || server.message(&second).is_some_and(|m| m.flagged));
    pool.submit(Job::Apply { account: "a1".into(), op: Op::Move { keys: vec![key.clone()], to: "Projects".into() } });
    eventually("moved on the server", || {
        server.holds(&projects, "Sound mix") && (!S::MOVES_KEEP_IDS || server.message(&second).is_some_and(|m| m.folder == projects))
    });
    until(&pool, "the move shows in Projects", |e| matches!(e, Event::FolderChanged { folder, .. } if folder == "Projects"));
    let in_projects = || {
        store
            .load_folder("a1", "Projects")
            .unwrap()
            .messages
            .iter()
            .any(|m| m.subject == "Sound mix" && m.flags.flagged && (!S::MOVES_KEEP_IDS || m.key.uid == key.uid))
    };
    if S::MOVES_KEEP_IDS {
        assert!(in_projects(), "in Projects here as the move is said, flagged, its number kept");
    } else {
        eventually("in Projects here, flagged", in_projects);
    }

    // Folders: create inside Projects, rename and move it to the top, delete it.
    pool.submit(Job::Apply { account: "a1".into(), op: Op::CreateFolder { path: "Projects/Caf&AOk-".into() } });
    eventually("created", || server.parent("Café").is_some_and(|p| p == projects));
    until(&pool, "folder list", |e| matches!(e, Event::Folders { folders, .. } if folders.iter().any(|f| f.path == "Projects/Caf&AOk-")));
    pool.submit(Job::Apply { account: "a1".into(), op: Op::RenameFolder { from: "Projects/Caf&AOk-".into(), to: "Clients".into() } });
    eventually("renamed", || server.parent("Clients").is_some_and(|p| p == S::TOP));
    until(&pool, "renamed in the list", |e| matches!(e, Event::Folders { folders, .. } if folders.iter().any(|f| f.path == "Clients")));
    pool.submit(Job::Apply { account: "a1".into(), op: Op::DeleteFolder { path: "Clients".into() } });
    eventually("deleted", || server.parent("Clients").is_none());

    // Automatic replies.
    pool.submit(Job::GetVacation { account: "a1".into() });
    until(&pool, "replies read", |e| matches!(e, Event::Vacation { vacation: Some(v), error: None, .. } if !v.enabled));
    for v in replies {
        pool.submit(Job::SetVacation { account: "a1".into(), vacation: v.clone() });
        until(&pool, "replies set", |e| matches!(e, Event::Vacation { vacation: Some(got), error: None, .. } if got == v));
        server.replies_are(v);
    }
    let last = replies.last().cloned().unwrap();
    pool.submit(Job::SetVacation { account: "a1".into(), vacation: Vacation { text: "refuse".into(), ..last.clone() } });
    until(&pool, "a refused change", |e| matches!(e, Event::Vacation { vacation: Some(v), error: Some(_), .. } if *v == last));

    // Empty Deleted Items; delete for good.
    for i in 0..3 {
        server.deliver(&trash, &mail(&format!("Old {i}"), &format!("t{i}")), true, "2024-01-01T00:00:00Z");
    }
    pool.submit(Job::Apply { account: "a1".into(), op: Op::EmptyFolder { path: "Deleted Items".into() } });
    eventually("emptied", || server.count(&trash) == 0);
    let gone = store.load_folder("a1", "INBOX").unwrap().messages.iter().find(|m| m.subject == "Storyboard").unwrap().key.clone();
    pool.submit(Job::Apply { account: "a1".into(), op: Op::Expunge { keys: vec![gone] } });
    eventually("deleted for good", || server.message(&first).is_none());

    // Send: the Bcc goes in the MIME for Exchange; Sent Items has the copy.
    let raw = format!("From: {me}\r\nTo: zoe@contoso.example\r\nSubject: e2e\r\nMessage-ID: <e2e@x>\r\n\r\nhi\r\n").into_bytes();
    let send = |id: u64, to: Vec<String>| Job::Send {
        account: "a1".into(),
        id,
        raw: raw.clone(),
        from: me.clone(),
        recipients: to,
        sent_folder: Some("Sent Items".into()),
    };
    pool.submit(send(7, vec!["zoe@contoso.example".into(), "boss@contoso.example".into()]));
    until(&pool, "sent", |e| matches!(e, Event::Sent { id: 7, .. }));
    until(&pool, "the copy in Sent", |e| matches!(e, Event::FolderChanged { folder, .. } if folder == "Sent Items"));
    let out = String::from_utf8(server.last_sent().unwrap()).unwrap();
    assert!(out.starts_with("Bcc: boss@contoso.example\r\nFrom:"), "{out}");
    assert!(server.count(&sent) > 0);
    pool.submit(send(8, vec!["nobody@contoso.example".into()]));
    until(&pool, "refused", |e| matches!(e, Event::SendFailed { id: 8, error, .. } if error.contains(S::NO_SUCH_RECIPIENT)));

    // A draft replaces its earlier copy.
    pool.submit(Job::SaveDraft { account: "a1".into(), raw: mail("Draft one", "d1"), folder: "Drafts".into(), replaces: None });
    until(&pool, "draft", |e| matches!(e, Event::FolderChanged { folder, .. } if folder == "Drafts"));
    let draft = store.load_folder("a1", "Drafts").unwrap().messages.iter().find(|m| m.subject == "Draft one").unwrap().key.clone();
    assert!(store.load_folder("a1", "Drafts").unwrap().messages.iter().any(|m| m.flags.draft));
    pool.submit(Job::SaveDraft { account: "a1".into(), raw: mail("Draft two", "d2"), folder: "Drafts".into(), replaces: Some(draft) });
    eventually("one draft left", || server.drafts() == 1);
    server.at_the_end();
    pool.submit(Job::Remove { account: "a1".into() });
}

/// A message whose source the server won't give doesn't stop the folder: the others download,
/// and the folder syncs. `sync` lists the account's (`a1`) folders and syncs its Inbox into the
/// store.
pub(super) fn one_unreadable_message_holds_up_nothing(name: &str, server: &impl Exchange, sync: impl FnOnce(&Store)) {
    let mail =
        |s: &str| format!("From: Zoe <zoe@studio.example>\r\nTo: me@x.example\r\nSubject: {s}\r\nMessage-ID: <{s}@x>\r\n\r\nHello.\r\n").into_bytes();
    let inbox = server.folder("Inbox", None, Some("inbox"));
    server.deliver(&inbox, &mail("one"), false, "2026-10-01T09:00:00Z");
    let bad = server.deliver(&inbox, &mail("two"), false, "2026-10-01T10:00:00Z");
    server.deliver(&inbox, &mail("three"), false, "2026-10-01T11:00:00Z");
    server.unreadable(&bad);
    let store = crate::fakes::fake::store(name);
    sync(&store);
    let fs = store.load_folder("a1", "INBOX").unwrap();
    assert_eq!(fs.messages.len(), 3);
    let with_body: Vec<&str> = fs.messages.iter().filter(|m| store.has_body(&m.key)).map(|m| m.subject.as_str()).collect();
    assert_eq!(with_body, ["three", "one"], "the others downloaded");
}
