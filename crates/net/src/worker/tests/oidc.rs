//! A JMAP server behind a single sign-on (OIDC), which takes bearer tokens only.

use super::*;
use crate::fakes::fake_jmap::{Data, PASSWORD, USER, serve};

/// The password, right as it is, gets "sign in through the browser" rather than a request for
/// the password again, which only looped.
#[test]
fn a_password_to_a_server_that_takes_none_says_to_sign_in_through_the_browser() {
    let mut d = Data::default();
    d.tokens = Some(Vec::new());
    d.mailbox("Inbox", None, Some("inbox"));
    let server = serve(d);
    let mut pool = Pool::new(Store::memory(), Arc::new(MemoryCredentials::default()));
    let account = AccountConfig { id: "j1".into(), email: USER.into(), username: USER.into(), jmap: Some(server.base.clone()), ..Default::default() };
    pool.submit(Job::Start { account, password: Some(Secret(PASSWORD.into())) });
    let seen = until(&pool, |e| matches!(e, Event::Status { status: SyncStatus::Error { .. } | SyncStatus::NeedsPassword { .. }, .. }));
    let said = seen.iter().rev().find_map(|e| match e {
        Event::Status { status: SyncStatus::Error { error } | SyncStatus::NeedsPassword { error }, .. } => Some(error.as_str()),
        _ => None,
    });
    assert_eq!(said, Some(mailcraft_model::NO_PASSWORDS), "{seen:?}");
    assert!(seen.iter().all(|e| !matches!(e, Event::Status { status: SyncStatus::NeedsPassword { .. }, .. })), "{seen:?}");
}
