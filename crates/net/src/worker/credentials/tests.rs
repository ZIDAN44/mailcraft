use std::sync::Arc;

use mailcraft_store::Store;

use super::super::{Auth, Pool, SyncOptions, Worker};
use super::*;

/// Found in review: the password just given was kept for signing in only when the account's lock
/// on it could be had whole, unlike everywhere else it's taken: after a thread panicked holding
/// it, the account signed in with no password, and nothing said why.
#[test]
fn a_password_given_is_kept_after_a_thread_panicked_holding_the_lock() {
    let store = Store::memory();
    let pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    let acc = AccountConfig { id: "a1".into(), ..Default::default() };
    let mut w = Worker::new(acc, Auth::shared(""), store, pool.sink.clone(), SyncOptions::default());
    w.creds = Some(Arc::new(MemoryCredentials::default()));
    w.keychain = Some(Some("pw".into()));
    let held = w.auth.clone();
    let _ = std::thread::spawn(move || {
        let _held = held.lock();
        panic!("a thread panicking with the lock held");
    })
    .join();
    assert!(w.auth.is_poisoned());
    assert!(w.settle_password());
    assert_eq!(w.auth.lock().unwrap_or_else(std::sync::PoisonError::into_inner).secret, "pw");
}
