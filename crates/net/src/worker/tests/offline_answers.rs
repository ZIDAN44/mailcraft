//! What the pool answers itself for an account no thread can take a job for (working offline,
//! not connected), and what it says when the keychain won't keep a password.

use mailcraft_model::job::{Secret, Vacation};
use mailcraft_model::{AccountConfig, Flags, MessageKey, ServerConfig, SyncStatus};

use super::*;

/// A keychain that keeps nothing.
struct Refusing;

impl Credentials for Refusing {
    fn get(&self, _: &AccountConfig) -> Option<String> {
        None
    }
    fn set(&self, _: &AccountConfig, _: &str) -> std::result::Result<(), String> {
        Err("locked".into())
    }
    fn delete(&self, _: &AccountConfig) {}
    fn get_smtp(&self, _: &AccountConfig) -> Option<String> {
        None
    }
    fn set_smtp(&self, _: &AccountConfig, _: &str) -> std::result::Result<(), String> {
        Err("locked".into())
    }
    fn delete_smtp(&self, _: &AccountConfig) {}
}

fn notice(message: &str) -> Event {
    Event::Notice { account: "a1".into(), message: message.into() }
}

/// What submitting `job` said at once.
fn said(pool: &mut Pool, job: Job) -> Vec<Event> {
    pool.submit(job);
    pool.poll()
}

/// Working offline: the account is remembered, not started, and each job for its mail is
/// answered here, as the account being offline, without a word of a connection.
#[test]
fn an_account_offline_is_answered_by_the_pool() {
    let store = crate::fake::store("offline-answers");
    let mut pool = Pool::new(store.clone(), Arc::new(Refusing));
    let account = AccountConfig {
        id: "a1".into(),
        email: "me@example.com".into(),
        imap: ServerConfig { host: "127.0.0.1".into(), port: 1, ..Default::default() },
        ..Default::default()
    };
    assert!(!account.is_local());
    let offline = "the account is offline";
    assert_eq!(said(&mut pool, Job::WorkOffline { on: true }), []);
    // Started while offline: its password can't be kept, and it waits.
    assert_eq!(
        said(&mut pool, Job::Start { account: account.clone(), password: Some(Secret("pw".into())) }),
        [notice("the password could not be saved in the keychain: locked"), Event::Status { account: "a1".into(), status: SyncStatus::Offline }]
    );
    assert_eq!(
        said(&mut pool, Job::SmtpPassword { account: account.clone(), password: Secret("pw".into()) }),
        [notice("the outgoing server's password could not be saved in the keychain: locked")]
    );
    assert_eq!(
        said(&mut pool, Job::DavPassword { account: account.clone(), password: Secret("pw".into()) }),
        [notice("the calendar server's password could not be saved in the keychain: there's no keychain to keep it in")]
    );
    assert_eq!(said(&mut pool, Job::DavPassword { account: account.clone(), password: Secret(String::new()) }), []);

    let key = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid: 7 };
    assert_eq!(
        said(&mut pool, Job::FetchBody { key: key.clone() }),
        [Event::BodyFailed { key: key.clone(), error: "not downloaded yet, and the account is offline".into() }]
    );
    assert_eq!(
        said(&mut pool, Job::SearchServer { account: "a1".into(), folders: vec!["INBOX".into()], query: "x".into(), search: 3 }),
        [Event::ServerSearched { account: "a1".into(), query: "x".into(), keys: Vec::new(), error: Some(offline.into()), done: true }]
    );
    assert_eq!(
        said(&mut pool, Job::SearchDirectory { account: "a1".into(), query: "ana".into() }),
        [Event::Directory { account: "a1".into(), query: "ana".into(), people: Vec::new(), error: Some(offline.into()), consent: false }]
    );
    assert_eq!(
        said(&mut pool, Job::FindIdentities { account: "a1".into() }),
        [Event::IdentitiesFound { account: "a1".into(), found: Vec::new(), error: Some(offline.into()) }]
    );
    let away = Event::Vacation { account: "a1".into(), vacation: None, error: Some(offline.into()) };
    assert_eq!(said(&mut pool, Job::GetVacation { account: "a1".into() }), std::slice::from_ref(&away));
    assert_eq!(said(&mut pool, Job::SetVacation { account: "a1".into(), vacation: Vacation::default() }), [away]);
    assert_eq!(
        said(&mut pool, Job::FileCopies { account: "a1".into(), folder: "Archive".into(), mail: vec![(key.clone(), Flags::default())], moved: true }),
        [Event::CopiesFiled { account: "a1".into(), folder: "Archive".into(), done: Vec::new(), failed: vec![(key, offline.into())], moved: true }]
    );
    let path = "mail/note.eml";
    assert_eq!(
        said(&mut pool, Job::FileMessages { account: "a1".into(), folder: "INBOX".into(), paths: vec![path.into()] }),
        [Event::MessagesFiled {
            account: "a1".into(),
            folder: "INBOX".into(),
            added: 0,
            failed: vec![(imports::filing::file_name(path), offline.into())]
        }]
    );
    // Nothing connects, by choice: not a word.
    for job in [
        Job::SyncAll { account: "a1".into() },
        Job::SyncFolder { account: "a1".into(), folder: "INBOX".into() },
        Job::FlushOutbox { account: "a1".into() },
        Job::SyncDav { account: "a1".into() },
    ] {
        assert_eq!(said(&mut pool, job.clone()), [], "{job:?}");
    }
    // What only its thread does: it isn't there.
    assert_eq!(said(&mut pool, Job::MoreMail { account: "a1".into(), folder: "INBOX".into(), more: 5 }), [notice("the account is not connected")]);

    // A message sent waits in the Outbox; a draft is kept here.
    let raw = b"From: me@example.com\r\nTo: ana@example.com\r\nSubject: Hi\r\nMessage-ID: <s@x>\r\n\r\nhi\r\n".to_vec();
    let send = Job::Send {
        account: "a1".into(),
        id: 4,
        raw: raw.clone(),
        from: "me@example.com".into(),
        recipients: vec!["ana@example.com".into()],
        sent_folder: Some("Sent".into()),
    };
    assert_eq!(said(&mut pool, send), [Event::Queued { account: "a1".into(), id: 4, error: "offline".into(), kind: Default::default() }]);
    let queued = store.outbox("a1");
    assert_eq!(queued.iter().map(|i| (i.id, i.error.as_deref())).collect::<Vec<_>>(), [(4, Some("offline"))]);
    assert_eq!(store.outbox_raw("a1", 4), Some(raw));
    let draft = b"From: me@example.com\r\nSubject: Draft\r\nMessage-ID: <d@x>\r\n\r\nlater\r\n".to_vec();
    assert_eq!(
        said(&mut pool, Job::SaveDraft { account: "a1".into(), raw: draft.clone(), folder: "Drafts".into(), replaces: None }),
        [
            Event::DraftSaved { account: "a1".into(), message_id: mailcraft_mime::written_message_id(&draft), error: None },
            notice("the draft is kept on this computer; it goes to Drafts when you're back online"),
        ]
    );
    assert_eq!(kept_raws(&store, "a1").unwrap(), [draft]);

    // Removed: an import into it said thrown away, and the account forgotten.
    let removed = said(&mut pool, Job::Remove { account: "a1".into() });
    assert!(
        matches!(&removed[..], [Event::Imported { account, done: true, report: Some(r), .. }] if account == "a1" && r.stopped == Some(mailcraft_model::job::StopHow::Discard)),
        "{removed:?}"
    );
    assert!(pool.configs.is_empty() && pool.workers.is_empty());
    pool.submit(Job::WorkOffline { on: false });
    assert!(pool.workers.is_empty(), "nothing left to start");
}
