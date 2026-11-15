//! An OAuth account's token refused by Graph: renewed from the refresh token, and the work done
//! again. Only the refresh token refused asks to sign in again: not a renewal the sign-in
//! service failed, nor a renewed token refused after the server took it.

use mailcraft_store::Store;

use super::*;

/// What a test of renewals works with: an account signed in with the refresh token `r1` at a
/// token endpoint, online on a fake Graph server that takes `t1`, its Inbox holding "Before".
struct Renewing {
    pool: crate::worker::Pool,
    server: Server,
    store: Store,
    inbox: String,
    token_requests: std::thread::JoinHandle<Vec<String>>,
}

/// [`Renewing`], the sign-in service (its provider `id`) giving `answers` in order, the first
/// of them `t1`.
fn renewing(id: &str, answers: &[(&str, &str)]) -> Renewing {
    let (oauth_base, token_requests) = token_answers(answers);
    let mut p = crate::oauth::tests::local(&format!("{oauth_base}/token"));
    p.id = id.into();
    crate::oauth::TEST_PROVIDERS.lock().unwrap().push(p);
    let mut d = Data { token: Some("t1".into()), ..Default::default() };
    let inbox = d.folder("Inbox", None, Some("inbox"));
    d.deliver(&inbox, &mail("Before", "m1"), true, "2024-05-06T09:00:00Z");
    let server = serve(d);
    let store = crate::fake::store(id);
    let mut pool = crate::worker::Pool::new(store.clone(), Arc::new(crate::worker::MemoryCredentials::default()));
    let account = AccountConfig { oauth: Some(id.into()), ..account("a1", &server.base) };
    pool.submit(Job::Start { account, password: Some(Secret("r1".into())) });
    until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    Renewing { pool, server, store, inbox, token_requests }
}

impl Renewing {
    /// Whether the Inbox has the message `subject` (and its source, when `whole`).
    fn has(&self, subject: &str, whole: bool) -> bool {
        let f = self.store.load_folder("a1", "INBOX");
        f.is_ok_and(|f| f.messages.iter().any(|m| m.subject == subject && (!whole || self.store.has_body(&m.key))))
    }

    /// Wait for `done`, failing if the account asks to be signed in again meanwhile.
    fn without_signing_in(&self, what: &str, done: impl Fn(&Renewing) -> bool) {
        let asked = |e: &Event| matches!(e, Event::Status { status: SyncStatus::NeedsPassword { .. }, .. });
        let seen = crate::waiting::events_until(&self.pool, what, |seen| seen.iter().any(asked) || done(self));
        if let Some(Event::Status { status: SyncStatus::NeedsPassword { error }, .. }) = seen.into_iter().find(asked) {
            panic!("asked to sign in again ({what}): {error}");
        }
    }

    /// The server refuses the token it took: the next sync asks for another.
    fn refuse(&mut self, next: &str) {
        self.server.data.lock().unwrap().token = Some(next.into());
        self.sync();
    }

    fn sync(&mut self) {
        self.pool.submit(Job::SyncFolder { account: "a1".into(), folder: "INBOX".into() });
    }

    /// Refused again, and the refresh token refused too: now it's signing in again. Every token
    /// request was a renewal from `r1`, `count` of them.
    fn end_signing_in_again(mut self, count: usize) {
        self.refuse("t-last");
        until(&self.pool, "sign in again", |e| matches!(e, Event::Status { status: SyncStatus::NeedsPassword { .. }, .. }));
        let seen = self.token_requests.join().unwrap();
        assert_eq!(seen.len(), count, "{seen:?}");
        assert!(seen.iter().all(|r| r.contains("grant_type=refresh_token&refresh_token=r1")), "{seen:?}");
        self.pool.submit(Job::Remove { account: "a1".into() });
    }
}

const T1: (&str, &str) = ("200 OK", r#"{"access_token":"t1","expires_in":3600}"#);
const T2: (&str, &str) = ("200 OK", r#"{"access_token":"t2","expires_in":3600}"#);
const T3: (&str, &str) = ("200 OK", r#"{"access_token":"t3","expires_in":3600}"#);
const REFUSED: (&str, &str) = ("400 Bad Request", r#"{"error":"invalid_grant"}"#);

/// An OAuth token Graph refuses (it ran out while the computer slept, say) is renewed from the
/// refresh token and the work done again; only a refused refresh asks to sign in again.
#[test]
fn a_refused_token_is_renewed_once_before_asking_to_sign_in() {
    // t1 at sign-in, t2 on the first renewal, and the refresh token refused on the second.
    let mut r = renewing("test-graph-renew", &[T1, T2, REFUSED]);
    // The server stops taking t1: the next sync gets t2 and goes on, without asking anything.
    r.server.data.lock().unwrap().deliver(&r.inbox, &mail("After", "m2"), false, "2024-05-06T10:00:00Z");
    r.refuse("t2");
    r.without_signing_in("synced with the renewed token", |r| r.has("After", false));
    r.end_signing_in_again(3);
}

/// Test hang: the token a renewal brought, taken by the server (signing in, the folders and the
/// Inbox listed) and refused before that sync was done (it ran out, or was revoked), was taken
/// for a fresh token the server won't have: the account asked to be signed in again, the
/// refresh token never tried. Under load the test above did that (the worker slow between the
/// Inbox written and its new message downloaded), and waited for good for the renewal it
/// expected. Once taken, a token refused later is renewed again.
#[test]
fn a_renewed_token_refused_after_it_worked_is_renewed_again() {
    let mut r = renewing("test-graph-renew-again", &[T1, T2, T3, REFUSED]);
    {
        let mut d = r.server.data.lock().unwrap();
        d.deliver(&r.inbox, &mail("After", "m2"), false, "2024-05-06T10:00:00Z");
        // t2 runs out as the Inbox is counted: written, its new message not downloaded yet.
        d.token_after = Some(("?$select=totalItemCount".into(), "t3".into()));
    }
    r.refuse("t2");
    r.without_signing_in("downloaded with the token renewed again", |r| r.has("After", true));
    r.end_signing_in_again(4);
}

/// Test hang, and what it brought to light: a renewal the sign-in service failed (busy, down or
/// throttling: HTTP 503, 500, 429, or `temporarily_unavailable` with a 400), or whose connection
/// dropped with no answer, was taken for the refresh token refused, and the account asked to be
/// signed in again. It's offline instead, and tried again later.
#[test]
fn a_renewal_the_sign_in_service_fails_is_tried_again() {
    let failures = [
        ("", ""),
        ("503 Service Unavailable", r#"{"error":"temporarily_unavailable"}"#),
        ("500 Internal Server Error", r#"{"error":"server_error"}"#),
        ("429 Too Many Requests", r#"{"error":"too_many_requests"}"#),
        ("400 Bad Request", r#"{"error":"temporarily_unavailable","error_description":"busy"}"#),
    ];
    let mut r = renewing("test-graph-renew-fails", &[&[T1][..], &failures[..], &[T2, REFUSED][..]].concat());
    r.server.data.lock().unwrap().deliver(&r.inbox, &mail("After", "m2"), false, "2024-05-06T10:00:00Z");
    r.server.data.lock().unwrap().token = Some("t2".into());
    for (status, _) in failures {
        r.sync();
        let what = format!("offline after a renewal answered {status:?}");
        until(&r.pool, &what, |e| match e {
            Event::Status { status: SyncStatus::NeedsPassword { error }, .. } => panic!("asked to sign in again ({what}): {error}"),
            e => matches!(e, Event::Status { status: SyncStatus::Error { .. }, .. }),
        });
    }
    // The sign-in service back: renewed, and synced.
    r.sync();
    r.without_signing_in("synced once renewed", |r| r.has("After", true));
    r.end_signing_in_again(8);
}
