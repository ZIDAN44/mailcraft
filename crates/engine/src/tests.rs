//! Engine tests: commands against the sample mailbox and scripted network events.

use mailcraft_model::{Event, FailureKind, Flag, FolderRole, Job, Op, SyncStatus};
use mailcraft_store::Store;
use serde_json::{Value, json};

use crate::session::{ListFilter, Row, SearchScope, View};
use crate::{CmdError, Session, sample};

const NOW: i64 = 1_791_979_200;

/// What a test made in the temporary folder ([`scratch`], [`temp`]): removed when the test's
/// thread ends, whether the test passed or failed (its sessions have gone by then).
#[derive(Default)]
struct Scratch {
    folders: Vec<std::path::PathBuf>,
    /// The caches made in them: their read marks are written first (written after, on their
    /// own thread, they'd make the folder again).
    stores: Vec<Store>,
}

impl Drop for Scratch {
    fn drop(&mut self) {
        for store in self.stores.drain(..) {
            store.flush_marks();
        }
        for folder in &self.folders {
            // Leftovers in the temporary folder are harmless; a failed clean-up isn't a failed test.
            let _ = std::fs::remove_dir_all(folder);
        }
    }
}

thread_local!(static SCRATCH: std::cell::RefCell<Scratch> = std::cell::RefCell::default());

/// A scratch folder for the test, `mailcraft-engine-<name>-<pid>` in the temporary folder: what
/// a run before left there is removed now, and the folder with all in it once the test is over.
fn scratch(name: &str) -> std::path::PathBuf {
    let folder = std::env::temp_dir().join(format!("mailcraft-engine-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    SCRATCH.with(|s| s.borrow_mut().folders.push(folder.clone()));
    folder
}

/// A cache in a [`scratch`] folder.
fn temp(name: &str) -> Store {
    let store = Store::open(scratch(name)).unwrap();
    SCRATCH.with(|s| s.borrow_mut().stores.push(store.clone()));
    store
}

/// The sample mailbox in a [`scratch`] folder, and a session on it.
fn sample_session(name: &str) -> Session {
    let store = temp(name);
    assert!(sample::seed_if_empty(&store, NOW).unwrap());
    assert!(!sample::seed_if_empty(&store, NOW).unwrap());
    let mut s = Session::open(Some(store), mailcraft_model::dates::Clock::new(NOW, 0));
    assert!(s.take_jobs().is_empty(), "local accounts start no workers");
    s
}

/// A session on a [`temp`] cache with one account on a server, added with `add` (account.add's
/// params), and its folders (each a path and its role) as its server lists them on connecting.
fn server_account(name: &str, add: Value, folders: &[(&str, FolderRole)]) -> (Session, String) {
    let mut s = Session::open(Some(temp(name)), mailcraft_model::dates::Clock::new(NOW, 0));
    let id = s.run("account.add", &add).unwrap()["account"].as_str().unwrap().to_string();
    let folders = folders
        .iter()
        .map(|&(path, role)| mailcraft_model::Folder { account: id.clone(), path: path.into(), role, selectable: true, ..Default::default() })
        .collect();
    s.apply(Event::Folders { account: id.clone(), folders });
    (s, id)
}

/// The sample in `store` with its Projects made a folder of `n` unread messages, `Report <uid>`
/// a minute apart, and a session on it.
fn big_projects(store: &Store, n: u32) -> Session {
    big_projects_with(store, n, mailcraft_model::Flags::default(), false)
}

/// [`big_projects`] with each message's flags `flags`, and with `sources`, a short source for
/// each, written in batches of 500.
fn big_projects_with(store: &Store, n: u32, flags: mailcraft_model::Flags, sources: bool) -> Session {
    sample::seed_if_empty(store, NOW).unwrap();
    let mut st = store.load_folder("sample", "Projects").unwrap();
    st.validity = 7;
    st.messages = (1..=n)
        .map(|uid| mailcraft_model::Summary {
            key: mailcraft_model::MessageKey { account: "sample".into(), folder: "Projects".into(), validity: 7, uid },
            subject: format!("Report {uid}"),
            date: NOW - i64::from(uid) * 60,
            flags: flags.clone(),
            ..Default::default()
        })
        .map(Into::into)
        .collect();
    if sources {
        let bodies: Vec<(mailcraft_model::MessageKey, Vec<u8>)> =
            st.messages.iter().map(|m| (m.key.clone(), format!("Subject: Report {}\r\n\r\nThe figures.\r\n", m.key.uid).into_bytes())).collect();
        for chunk in bodies.chunks(500) {
            store.write_bodies(chunk.to_vec()).unwrap();
        }
    }
    store.save_folder("sample", "Projects", &st).unwrap();
    let mut s = Session::open(Some(store.clone()), mailcraft_model::dates::Clock::new(NOW, 0));
    s.run("folder.open", &json!({"account": "sample", "folder": "Projects"})).unwrap();
    assert_eq!(s.list.len(), n as usize);
    s
}

/// The key the next message to come into `account`'s `folder` gets: the validity of its list as
/// kept, and the uid past its highest.
fn next_key(s: &Session, account: &str, folder: &str) -> mailcraft_model::MessageKey {
    let st = s.store.as_ref().unwrap().load_folder(account, folder).unwrap();
    let uid = st.messages.iter().map(|m| m.key.uid).max().unwrap_or(0) + 1;
    mailcraft_model::MessageKey { account: account.into(), folder: folder.into(), validity: st.validity, uid }
}

/// A message come into its folder (its key's), as the network layer files one: `summary` at the
/// top of the folder's list kept (in place of one under the same key), `raw` kept as its source
/// (none: not downloaded yet), and the session told the folder changed. A folder with no list
/// kept yet takes the key's validity, as its first sync would give it.
fn file_in(s: &mut Session, summary: mailcraft_model::Summary, raw: Option<&[u8]>) -> mailcraft_model::MessageKey {
    let store = s.store.clone().unwrap();
    let key = summary.key.clone();
    let mut st = store.load_folder(&key.account, &key.folder).unwrap();
    if st.validity == 0 && st.messages.is_empty() {
        st.validity = key.validity;
    }
    st.messages.retain(|m| m.key != key);
    st.messages.insert(0, summary.into());
    store.save_folder(&key.account, &key.folder, &st).unwrap();
    if let Some(raw) = raw {
        store.write_body(&key, raw).unwrap();
    }
    s.apply(Event::FolderChanged { account: key.account.clone(), folder: key.folder.clone() });
    key
}

/// [`file_in`] whole: its summary read from `raw` (dated `NOW` when it says no date), and `raw` kept.
fn file_whole(s: &mut Session, key: mailcraft_model::MessageKey, raw: &[u8]) -> mailcraft_model::MessageKey {
    let summary = mailcraft_mime::summary_from_full(raw, key, mailcraft_model::Flags::default(), 0, NOW);
    file_in(s, summary, Some(raw))
}

/// A message to Riley from `from` carrying a calendar file (`invite.ics`, of type `mime`): an
/// invitation, or an answer to one, as it comes.
fn calendar_mail(from: (&str, &str), subject: &str, mime: &str, ics: Vec<u8>, message_id: &str) -> Vec<u8> {
    let d = mailcraft_mime::Draft {
        to: vec![mailcraft_model::EmailAddress::new("Riley", "riley@studio.example")],
        subject: subject.into(),
        body: "See the calendar.".into(),
        attachments: vec![mailcraft_mime::OutAttachment { name: "invite.ics".into(), mime: mime.into(), data: ics, ..Default::default() }],
        ..Default::default()
    };
    mailcraft_mime::build(&d, &mailcraft_model::EmailAddress::new(from.0, from.1), NOW, message_id).unwrap()
}

/// How many waits the store has logged on this thread. A wait for the read marks, or for the
/// lock every change of a list is made under, is logged (by `mailcraft_store::marking`, on the
/// thread that waited) once it took over 50 ms or was given up. While a test holds the marks, a
/// wait can't end sooner, so this count not moving shows the window's thread waited for none of
/// them, without timing it on a busy machine.
fn waits_logged() -> usize {
    use std::cell::Cell;
    thread_local!(static WAITED: Cell<usize> = const { Cell::new(0) });
    struct Waits;
    impl log::Log for Waits {
        fn enabled(&self, m: &log::Metadata<'_>) -> bool {
            m.level() <= log::Level::Warn && m.target() == "mailcraft_store::marking"
        }
        fn log(&self, r: &log::Record<'_>) {
            if self.enabled(r.metadata()) {
                WAITED.with(|w| w.set(w.get() + 1));
            }
        }
        fn flush(&self) {}
    }
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        assert!(log::set_logger(&Waits).is_ok(), "the only logger of the engine's tests");
        log::set_max_level(log::LevelFilter::Warn);
    });
    WAITED.with(Cell::get)
}

/// The seed the engine's tests start [`Xorshift`] from.
pub(crate) const XORSHIFT_SEED: u64 = 0x9E37_79B9_7F4A_7C15;

/// The xorshift64 steps (13, 7, 17) from a seed (not 0): the same numbers every run, for tests
/// that want many different values (bytes that don't compress, docs in no order), not randomness.
pub(crate) struct Xorshift(pub(crate) u64);

impl Iterator for Xorshift {
    type Item = u64;

    fn next(&mut self) -> Option<u64> {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        Some(self.0)
    }
}

/// [`Xorshift`] gives the sequence its tests were written with (worked out apart, in Python).
#[test]
fn xorshift_gives_the_same_numbers() {
    let first: Vec<u64> = Xorshift(XORSHIFT_SEED).take(3).collect();
    assert_eq!(first, [0xdc1b_77ae_0bf3_4dad, 0x64f0_eeb9_026e_6076, 0x7b07_ce91_e590_6136]);
    assert_eq!(Xorshift(XORSHIFT_SEED).nth(99_999), Some(0xdeaf_6465_dc02_951c));
}

fn key_of(s: &Session, subject: &str) -> Value {
    let m = s.list.iter().find(|m| m.subject == subject).unwrap_or_else(|| panic!("no {subject}"));
    json!({"key": m.key.to_id()})
}

/// The calendar carried by the newest message in a folder (as `METHOD`, and the text).
fn sent_calendar(s: &Session, folder: &str, subject: &str) -> (String, String) {
    let st = s.store.as_ref().unwrap().load_folder("sample", folder).unwrap();
    let m = st
        .messages
        .iter()
        .find(|m| m.subject == subject)
        .unwrap_or_else(|| panic!("no {subject}: {:?}", st.messages.iter().map(|m| &m.subject).collect::<Vec<_>>()));
    let cal = s.message_calendar(&m.key).unwrap();
    let raw = String::from_utf8_lossy(&s.store.as_ref().unwrap().read_body(&m.key).unwrap()).to_string();
    (cal.method.unwrap_or_default(), raw)
}

/// `--version` of both programs: the release build's commit, shortened, and date; a build
/// without them, the version alone.
#[test]
fn the_version_line_names_the_program_and_its_build() {
    assert_eq!(
        crate::build_line("mailcraft-cli", "1.2.3", Some("0123456789abcdef"), Some("2026-11-15")),
        "mailcraft-cli 1.2.3 (012345678, 2026-11-15)"
    );
    assert_eq!(crate::build_line("mailcraft", "1.2.3", Some("abc"), Some("2026-11-15")), "mailcraft 1.2.3 (abc, 2026-11-15)");
    assert_eq!(crate::build_line("mailcraft", "1.2.3", Some(""), Some("2026-11-15")), "mailcraft 1.2.3");
    assert_eq!(crate::build_line("mailcraft", "1.2.3", Some("abc"), None), "mailcraft 1.2.3");
    assert!(crate::version_line("mailcraft").starts_with(&format!("mailcraft {}", env!("CARGO_PKG_VERSION"))));
}

mod accounts;
mod calendar;
mod directory;
mod mail;
mod opening;
mod organize;
mod safety;
mod smime;
mod writing;
