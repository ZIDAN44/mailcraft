use super::*;
use crate::waiting::{events_until, eventually, until};

mod calendar_only;
mod check_every;
mod counts;
mod drafts;
mod edits;
mod offline_answers;
mod opening;
mod outbox_once;
mod pop_filed;
mod read_errors;
mod removed;
mod search_index;
mod unstarted;

/// An account `id` on an IMAP server of the tests' own at `port` on this computer, signing in as
/// `me@studio.example`.
pub(crate) fn imap_account(id: &str, port: u16) -> AccountConfig {
    AccountConfig {
        id: id.into(),
        email: "me@studio.example".into(),
        username: "me@studio.example".into(),
        imap: mailcraft_model::ServerConfig { host: "127.0.0.1".into(), port, security: mailcraft_model::Security::None },
        ..Default::default()
    }
}

/// An outgoing server on this computer where nothing listens (port 1).
pub(crate) fn no_smtp() -> mailcraft_model::ServerConfig {
    mailcraft_model::ServerConfig { host: "127.0.0.1".into(), port: 1, security: mailcraft_model::Security::None }
}

/// The POP account `p1` on a server of the tests' own at `port` on this computer, signing in as
/// `me@pop.example`.
pub(crate) fn pop_account(port: u16) -> AccountConfig {
    AccountConfig {
        id: "p1".into(),
        email: "me@pop.example".into(),
        username: "me@pop.example".into(),
        pop: Some(mailcraft_model::ServerConfig { host: "127.0.0.1".into(), port, security: mailcraft_model::Security::None }),
        ..Default::default()
    }
}

/// Sent Items asked for after each of several messages is read once.
#[test]
fn a_folder_waiting_to_be_read_isnt_queued_twice() {
    let mut q = VecDeque::new();
    let sent = Job::SyncFolder { account: "a".into(), folder: "Sent".into() };
    push_job(&mut q, sent.clone());
    push_job(&mut q, Job::SyncFolder { account: "a".into(), folder: "INBOX".into() });
    push_job(&mut q, sent.clone());
    push_job(&mut q, Job::GetVacation { account: "a".into() });
    push_job(&mut q, Job::GetVacation { account: "a".into() });
    assert_eq!(q.len(), 4, "only the folder read is merged");
    // A calendar sync waiting already: the second would send the first one's new items again
    // before its results were merged (found testing issue #30 against Radicale).
    push_job(&mut q, Job::SyncDav { account: "a".into() });
    push_job(&mut q, Job::SyncDav { account: "a".into() });
    push_job(&mut q, Job::SyncDav { account: "b".into() });
    assert_eq!(q.len(), 6);
}

#[test]
fn memory_credentials() {
    let m = MemoryCredentials::default();
    let a = AccountConfig { id: "a1".into(), ..Default::default() };
    assert_eq!(m.get(&a), None);
    m.set(&a, "pw").unwrap();
    assert_eq!(m.get(&a).as_deref(), Some("pw"));
    m.delete(&a);
    assert_eq!(m.get(&a), None);
}

/// Microsoft can take a token and then refuse the mailbox ("User is authenticated but not
/// connected"). The address the browser signed in as is tried; otherwise it's a refusal that
/// says what to do, not a sign-in to ask for again.
#[test]
fn a_mailbox_microsoft_wont_open_says_why() {
    let st = crate::fake::State::new();
    st.lock().unwrap().not_connected.push("zoe@hotmail.example".into());
    let port = crate::fake::serve_tcp(&st);
    let store = crate::fake::store("closed-mailbox");
    let pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    let mut acc = AccountConfig {
        id: "a1".into(),
        email: "zoe@hotmail.example".into(),
        imap: mailcraft_model::ServerConfig { host: "127.0.0.1".into(), port, security: mailcraft_model::Security::None },
        oauth: Some("microsoft".into()),
        ..Default::default()
    };
    let w = Worker::new(acc.clone(), Auth::shared("refresh"), store.clone(), pool.sink.clone(), SyncOptions::default());
    let e = w.imap_with_token("zoe@hotmail.example", crate::fake::GOOD_TOKEN).err().unwrap();
    assert!(e.to_string().starts_with(mailcraft_model::MICROSOFT_IMAP_CLOSED) && e.to_string().contains("IMAP"), "{e:?}");
    // The browser signed in as the mailbox's own address: that one opens it.
    acc.oauth_user = Some("me@studio.example".into());
    let w = Worker::new(acc.clone(), Auth::shared("refresh"), store.clone(), pool.sink.clone(), SyncOptions::default());
    assert!(w.imap_with_token("zoe@hotmail.example", crate::fake::GOOD_TOKEN).is_ok());
    // Signed in as another account that can't open it either: named in the refusal.
    acc.oauth_user = Some("someone@else.example".into());
    let w = Worker::new(acc, Auth::shared("refresh"), store, pool.sink.clone(), SyncOptions::default());
    let e = w.imap_with_token("zoe@hotmail.example", crate::fake::GOOD_TOKEN).err().unwrap();
    assert!(matches!(&e, NetError::MailboxClosed(m) if m.contains("signed in as someone@else.example")), "{e:?}");
    assert_eq!(e.failure(), mailcraft_model::FailureKind::MicrosoftImapClosed);
    // A token that's simply refused is still a sign-in to ask for again.
    let w = Worker::new(
        AccountConfig { oauth_user: None, ..w.acc.clone() },
        Auth::shared("refresh"),
        crate::fake::store("closed-mailbox-2"),
        pool.sink.clone(),
        SyncOptions::default(),
    );
    assert!(matches!(w.imap_with_token("zoe@hotmail.example", "expired"), Err(NetError::Auth(_))));
}

/// A MailCraft data file (`.mailcraft`) comes in through the same job as a `.pst`.
#[test]
fn a_mailcraft_data_file_is_imported() {
    let t = std::env::temp_dir().join(format!("mailcraft-worker-datafile-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&t);
    let old = Store::open_packed(t.join("old"), t.join("Old Files")).unwrap();
    old.save_accounts(&[AccountConfig { id: "a1".into(), email: "zoe@studio.example".into(), ..Default::default() }]).unwrap();
    old.save_folders("a1", &[Folder { path: "INBOX".into(), role: FolderRole::Inbox, ..Default::default() }]).unwrap();
    let k = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid: 3 };
    let mut st = old.load_folder("a1", "INBOX").unwrap();
    st.messages.push(mailcraft_model::Summary { key: k, subject: "Kept".into(), ..Default::default() }.into());
    old.save_folder("a1", "INBOX", &st).unwrap();
    let mut pim = mailcraft_model::pim::Pim::default();
    pim.notes.push(mailcraft_model::pim::StoredNote::default());
    old.save_account_doc("a1", mailcraft_model::pim::PIM_DOC, &pim).unwrap();
    let file = old.data_file("a1").unwrap();
    drop(old);
    let store = Store::open_packed(t.join("app"), t.join("Files")).unwrap();
    let mut p = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    // Looked at first: what the file system says of it.
    let path = file.to_string_lossy().into_owned();
    p.submit(Job::InspectDataFile { path: path.clone() });
    p.submit(Job::InspectDataFile { path: t.join("gone.pst").to_string_lossy().into_owned() });
    let looked = |e: &Event| matches!(e, Event::DataFileInspected { .. });
    let seen = events_until(&p, "both files looked at", |seen| seen.iter().filter(|e| looked(e)).count() >= 2);
    let facts: Vec<_> = seen
        .into_iter()
        .filter_map(|e| match e {
            Event::DataFileInspected { facts, .. } => Some(facts),
            _ => None,
        })
        .collect();
    let size = std::fs::metadata(&file).unwrap().len();
    assert!(
        facts.iter().any(|f| f.path == path
            && f.kind == mailcraft_model::job::DataFileKind::MailCraft
            && f.size == size
            && f.modified > 0
            && f.error.is_none()),
        "{facts:?}"
    );
    assert!(
        facts.iter().any(|f| f.path.ends_with("gone.pst") && f.kind == mailcraft_model::job::DataFileKind::Pst && f.error.is_some()),
        "{facts:?}"
    );
    p.submit(Job::ImportPst { account: "p1".into(), path, resume: false });
    let seen = until(&p, "the import to end", |e| matches!(e, Event::Imported { done: true, .. }));
    assert!(seen.iter().any(|e| matches!(e, Event::Imported { messages: 1, done: true, error: None, .. })), "{seen:?}");
    assert!(
        seen.iter().any(|e| matches!(e, Event::Imported { report: Some(r), .. } if r.messages == 1 && r.folders == 1 && (r.contacts, r.notes) == (0, 1) && r.runs == 1 && r.skips.is_empty() && !r.data_file.is_empty())),
        "with what it came to"
    );
    assert!(seen.iter().any(|e| matches!(e, Event::Folders { folders, .. } if folders.len() == 1)));
    assert_eq!(store.load_folder("p1", "INBOX").unwrap().messages[0].key.account, "p1");
    let _ = std::fs::remove_dir_all(&t);
}

/// The keychain is read by the account's thread, not the caller's (the window's): with no
/// password there, the account asks for one and is as one never started.
#[test]
fn pool_without_password_asks_for_one() {
    let dir = std::env::temp_dir().join(format!("mailcraft-pool-{}", std::process::id()));
    let store = Store::open(&dir).unwrap();
    let mut p = Pool::new(store, Arc::new(MemoryCredentials::default()));
    let a = AccountConfig { id: "a1".into(), ..Default::default() };
    p.submit(Job::Start { account: a, password: None });
    let seen = until(&p, "the password asked for", |e| matches!(e, Event::Status { status: SyncStatus::NeedsPassword { .. }, .. }));
    assert!(seen.iter().all(|e| matches!(e, Event::Status { .. })), "{seen:?}");
    assert!(p.poll().is_empty());
    p.submit(Job::SyncAll { account: "a1".into() });
    assert!(matches!(p.poll().as_slice(), [Event::Notice { .. }]));
    let key = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid: 1 };
    p.submit(Job::FetchBody { key: key.clone() });
    assert!(matches!(p.poll().as_slice(), [Event::BodyFailed { .. }]));
    p.store.write_body(&key, b"Subject: cached\r\n\r\nx").unwrap();
    p.submit(Job::FetchBody { key });
    assert!(matches!(p.poll().as_slice(), [Event::Body { .. }]));
    let _ = std::fs::remove_dir_all(dir);
}

/// A keychain that notes which thread asks it, and has no password till one is kept.
#[derive(Default)]
struct Asked(std::sync::Mutex<Vec<String>>, MemoryCredentials);

impl Asked {
    fn note(&self, what: &str) {
        let thread = std::thread::current().name().unwrap_or("?").to_string();
        if let Ok(mut l) = self.0.lock() {
            l.push(format!("{what} on {thread}"));
        }
    }
}

impl Credentials for Asked {
    fn get(&self, account: &AccountConfig) -> Option<String> {
        self.note("get");
        self.1.get(account)
    }
    fn set(&self, account: &AccountConfig, password: &str) -> std::result::Result<(), String> {
        self.note("set");
        self.1.set(account, password)
    }
    fn delete(&self, account: &AccountConfig) {
        self.1.delete(account);
    }
    fn get_smtp(&self, _: &AccountConfig) -> Option<String> {
        None
    }
    fn set_smtp(&self, _: &AccountConfig, _: &str) -> std::result::Result<(), String> {
        Ok(())
    }
    fn delete_smtp(&self, _: &AccountConfig) {}
}

/// The keychain is asked on the account's own thread, never the caller's: on macOS it may ask
/// the user first (after an update), and the caller is the window, which would stop drawing
/// till that's answered. Read at a start, kept when a password is given.
#[test]
fn the_keychain_is_asked_on_the_accounts_thread() {
    let dir = std::env::temp_dir().join(format!("mailcraft-pool-keychain-{}", std::process::id()));
    let store = Store::open(&dir).unwrap();
    let asked = Arc::new(Asked::default());
    let mut p = Pool::new(store, asked.clone());
    let a = AccountConfig { id: "a1".into(), ..Default::default() };
    p.submit(Job::Start { account: a.clone(), password: None });
    until(&p, "the password asked for", |e| matches!(e, Event::Status { status: SyncStatus::NeedsPassword { .. }, .. }));
    p.submit(Job::Start { account: a, password: Some(Secret("pw".into())) });
    eventually("the password looked for and kept", || asked.0.lock().unwrap().len() >= 2);
    assert_eq!(*asked.0.lock().unwrap(), ["get on mail-a1", "set on mail-a1"]);
    assert_eq!(asked.1.get(&AccountConfig { id: "a1".into(), ..Default::default() }).as_deref(), Some("pw"));
    p.submit(Job::Remove { account: "a1".into() });
    let _ = std::fs::remove_dir_all(dir);
}

/// An account of `n` messages in its Inbox, each with its source: `word{uid}` and `common`.
fn indexed_mail(n: u32) -> Store {
    let s = Store::memory();
    fill(&s, n);
    s
}

/// `n` messages in `s`'s account `a1`, as [`indexed_mail`] has them.
fn fill(s: &Store, n: u32) {
    let messages = (1..=n)
        .map(|uid| mailcraft_model::Summary {
            key: MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 7, uid },
            message_id: format!("m{uid}@x"),
            size: 40,
            date: i64::from(uid),
            ..Default::default()
        })
        .map(Into::into)
        .collect();
    let inbox = Folder { account: "a1".into(), path: "INBOX".into(), role: FolderRole::Inbox, selectable: true, ..Default::default() };
    s.save_folders("a1", &[inbox]).unwrap();
    s.save_folder("a1", "INBOX", &mailcraft_store::FolderState { validity: 7, uid_next: n + 1, messages, ..Default::default() }).unwrap();
    for uid in 1..=n {
        let key = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 7, uid };
        s.write_body(&key, format!("Subject: m\r\nMessage-ID: <m{uid}@x>\r\n\r\nword{uid} common\r\n").as_bytes()).unwrap();
    }
}
