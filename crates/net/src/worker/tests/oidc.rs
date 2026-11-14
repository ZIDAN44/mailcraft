//! A JMAP server behind a single sign-on (OIDC), which takes bearer tokens only.

use mailcraft_model::{OIDC, OidcSettings};

use super::*;
use crate::fakes::fake_jmap::{Data, PASSWORD, USER, serve};
use crate::fakes::fake_sso;
use crate::oauth::oidc;

/// The messages in the account's Inbox on this computer, once there are `n` (20 seconds at most).
fn inbox_holds(pool: &Pool, store: &Store, n: usize) -> Vec<Event> {
    let end = Instant::now() + Duration::from_secs(20);
    let mut seen = Vec::new();
    while store.load_folder("j1", "INBOX").map_or(0, |f| f.messages.len()) < n {
        assert!(Instant::now() < end, "the Inbox never had {n}: {seen:?}");
        seen.extend(pool.wait(Duration::from_millis(50)));
    }
    seen
}

/// The whole sign-in through a single sign-on: its service found from the mail server, the
/// browser sign-in with PKCE, the refresh token kept, mail synced over a bearer token, and a
/// token the server no longer takes renewed with the refresh token, with no password asked for.
#[test]
fn a_single_sign_on_signs_in_through_the_browser_then_syncs_over_its_tokens() {
    let message = |n: u32| format!("From: {USER}\r\nTo: {USER}\r\nSubject: m{n}\r\nMessage-ID: <o{n}@x>\r\n\r\nHello {n}.\r\n");
    let mut d = Data::default();
    d.tokens = Some(Vec::new());
    let inbox = d.mailbox("Inbox", None, Some("inbox"));
    d.deliver(&inbox, message(1).as_bytes(), &[], "2026-05-01T09:00:00Z");
    let jmap = serve(d);
    let mut sd = fake_sso::Data::default();
    sd.jmap = Some(jmap.data.clone());
    let sso = fake_sso::serve(sd);
    jmap.data.lock().unwrap().issuer = Some(sso.base.clone());
    // The service, found from the mail server's address (which sends on to it).
    let found = oidc::describe(&oidc::description_url_for_server(&format!("{}/jmap/session", jmap.base)).unwrap()).unwrap();
    assert_eq!(found.issuer, sso.base);

    let store = Store::memory();
    let creds = Arc::new(MemoryCredentials::default());
    let mut pool = Pool::new(store.clone(), creds.clone());
    let settings = OidcSettings { issuer: found.issuer, client_id: fake_sso::CLIENT_ID.into(), scope: oidc::DEFAULT_SCOPE.into() };
    let account = AccountConfig {
        id: "j1".into(),
        email: USER.into(),
        username: USER.into(),
        jmap: Some(jmap.base.clone()),
        oauth: Some(OIDC.into()),
        oidc: Some(settings),
        ..Default::default()
    };
    pool.submit(Job::SignIn { account: account.clone() });
    // The browser: the sign-in page sends it back to MailCraft with a code.
    let seen = until(&pool, |e| matches!(e, Event::OpenUrl { .. } | Event::Status { .. }));
    let Some(Event::OpenUrl { url }) = seen.last().cloned() else { panic!("{seen:?}") };
    assert!(url.starts_with(&format!("{}/authorize?", sso.base)), "{url}");
    assert!(url.contains("code_challenge_method=S256") && url.contains("scope=openid%20profile%20email&"), "{url}");
    std::thread::spawn(move || crate::http::get(&url, 10_000));
    let seen = until(&pool, |e| matches!(e, Event::SignedIn { .. } | Event::Status { .. }));
    assert!(matches!(seen.last(), Some(Event::SignedIn { user: Some(u), .. }) if u == USER), "{seen:?}");
    let kept = creds.get(&account).unwrap_or_default();
    assert!(kept.starts_with("rt"), "the refresh token is what's kept: {kept}");

    // The account starts: an access token from the refresh token, then its mail over it.
    pool.submit(Job::Start { account: account.clone(), password: None });
    let mut seen = inbox_holds(&pool, &store, 1);
    let refreshes = || sso.data.lock().unwrap().forms.iter().filter(|f| f.contains("grant_type=refresh_token")).count();
    let before = refreshes();
    assert!(before >= 1);
    // The server stops taking that token (it ran out early, or was revoked): a new one from the
    // refresh token, and the mail still comes.
    {
        let mut d = jmap.data.lock().unwrap();
        d.tokens = Some(Vec::new());
        d.deliver(&inbox, message(2).as_bytes(), &[], "2026-05-02T09:00:00Z");
    }
    pool.submit(Job::SyncFolder { account: "j1".into(), folder: "INBOX".into() });
    seen.extend(inbox_holds(&pool, &store, 2));
    assert!(seen.iter().all(|e| !matches!(e, Event::Status { status: SyncStatus::NeedsPassword { .. } | SyncStatus::Error { .. }, .. })), "{seen:?}");
    assert!(refreshes() > before, "the token wasn't renewed");
    let forms = sso.data.lock().unwrap().forms.clone();
    let refreshed: Vec<&String> = forms.iter().filter(|f| f.contains("grant_type=refresh_token")).collect();
    // A public client: its id with every request, and no secret.
    assert!(forms.iter().all(|f| f.contains("client_id=mailcraft-test") && !f.contains("client_secret")), "{forms:?}");
    assert!(refreshed.iter().all(|f| f.contains(&format!("refresh_token={kept}"))), "{forms:?}");
}

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
