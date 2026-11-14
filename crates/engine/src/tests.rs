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

/// The start of 2026-10-16, local, as the sample's clock has it.
fn mailcraft_engine_day(s: &Session) -> i64 {
    crate::parse_local("2026-10-16", &s.clock).map(|(t, _)| t).unwrap()
}

mod accounts;
mod calendar;
mod categories;
mod directory;
mod files;
mod mail;
mod organize;
mod safety;
mod selecting;
mod senders;
mod signature_code;
mod signatures;
mod smime;
mod source;
mod text_first;
mod writing;
