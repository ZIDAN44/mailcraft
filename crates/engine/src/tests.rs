//! Engine tests: commands against the sample mailbox and scripted network events.

use mailcraft_model::{Event, Flag, FolderRole, Job, Op, SyncStatus};
use mailcraft_store::Store;
use serde_json::{Value, json};

use crate::session::{ListFilter, Row, SearchScope, View};
use crate::{CmdError, Session, sample};

const NOW: i64 = 1_791_979_200;

fn temp(name: &str) -> Store {
    let dir = std::env::temp_dir().join(format!("mailcraft-engine-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    Store::open(dir).unwrap()
}

fn sample_session(name: &str) -> Session {
    let store = temp(name);
    assert!(sample::seed_if_empty(&store, NOW).unwrap());
    assert!(!sample::seed_if_empty(&store, NOW).unwrap());
    let mut s = Session::open(Some(store), mailcraft_model::dates::Clock::new(NOW, 0));
    assert!(s.take_jobs().is_empty(), "local accounts start no workers");
    s
}

fn done(s: Session) {
    if let Some(st) = &s.store {
        let _ = std::fs::remove_dir_all(st.root());
    }
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
mod directory;
mod files;
mod invites;
mod mail;
mod organize;
mod pim;
mod safety;
mod signature_code;
mod smime;
mod tentative;
mod text_first;
mod web_calendars;
mod writing;
