//! A JMAP server behind a single sign-on (OIDC), which takes bearer tokens only.

use mailcraft_engine::Session;
use mailcraft_model::dates::Clock;
use mailcraft_model::{OIDC, OidcSettings};
use serde_json::json;

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
    let found = oidc::describe(&oidc::description_url(&jmap.base)).unwrap();
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
    // It signed in with a password before: that goes once it's signed in through the browser.
    creds.set(&account, PASSWORD).unwrap();
    pool.submit(Job::SignIn { account: account.clone() });
    // The browser: the sign-in page sends it back to MailCraft with a code.
    let seen = until(&pool, |e| matches!(e, Event::OpenUrl { .. } | Event::Status { .. }));
    let Some(Event::OpenUrl { url }) = seen.last().cloned() else { panic!("{seen:?}") };
    assert!(url.starts_with(&format!("{}/authorize?", sso.base)), "{url}");
    assert!(url.contains("code_challenge_method=S256") && url.contains("scope=openid%20profile%20email&"), "{url}");
    std::thread::spawn(move || crate::http::get(&url, 10_000));
    let seen = until(&pool, |e| matches!(e, Event::SignedIn { .. } | Event::Status { .. }));
    assert!(matches!(seen.last(), Some(Event::SignedIn { user: Some(u), .. }) if u == USER), "{seen:?}");
    let kept = creds.get_secret(&account, SIGN_ON_SECRET).unwrap_or_default();
    assert!(kept.starts_with("rt"), "the refresh token is what's kept: {kept}");
    assert_eq!(creds.get(&account), None, "no password is kept");

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

/// Runs what `s` asks of the pool and gives it what comes back, as the app's window does, till
/// `done` (30 seconds at most). A page it opens is followed as the browser would, the user
/// signing in there.
fn drive(s: &mut Session, pool: &mut Pool, done: impl Fn(&Session) -> bool) -> Vec<Event> {
    let end = Instant::now() + Duration::from_secs(30);
    let mut seen = Vec::new();
    while !done(s) {
        assert!(Instant::now() < end, "never done: {seen:?}");
        for job in s.take_jobs() {
            pool.submit(job);
        }
        for url in std::mem::take(&mut s.open_urls) {
            std::thread::spawn(move || crate::http::get(&url, 10_000));
        }
        if let Some(e) = pool.wait(Duration::from_millis(50)) {
            seen.push(e.clone());
            s.apply(e);
        }
    }
    seen
}

/// End to end, as the app does it: the engine's session asks, the pool goes to this computer's
/// fake servers. The server, asked how it's signed in to, answers `401 Bearer`, and its address
/// names its service; the account added signs in through it in the browser (PKCE), with no
/// password; its mail comes over a bearer token; and when the server stops taking that token,
/// Send / Receive gets a new one with the refresh token.
#[test]
fn an_account_is_added_to_sign_in_through_its_servers_single_sign_on() {
    let message = |n: u32| format!("From: {USER}\r\nTo: {USER}\r\nSubject: m{n}\r\nMessage-ID: <e{n}@x>\r\n\r\nHello {n}.\r\n");
    let mut d = Data::default();
    d.tokens = Some(Vec::new());
    let inbox = d.mailbox("Inbox", None, Some("inbox"));
    d.deliver(&inbox, message(1).as_bytes(), &[], "2026-05-01T09:00:00Z");
    let jmap = serve(d);
    let mut sd = fake_sso::Data::default();
    sd.jmap = Some(jmap.data.clone());
    let sso = fake_sso::serve(sd);
    jmap.data.lock().unwrap().issuer = Some(sso.base.clone());
    let store = Store::memory();
    let creds = Arc::new(MemoryCredentials::default());
    let mut pool = Pool::new(store.clone(), creds.clone());
    let mut s = Session::open(Some(store.clone()), Clock::new(1_777_626_000, 0));
    s.oauth_available = vec![OIDC.into()];

    // The add dialog asks the server how it's signed in to, once its address is typed.
    let server = jmap.base.clone();
    s.run("account.probeJmap", &json!({"url": server})).unwrap();
    drive(&mut s, &mut pool, |s| s.jmap_probes.get(&server).is_some_and(Option::is_some));
    let probe = s.jmap_probes.get(&server).cloned().flatten().unwrap();
    assert!(probe.takes_no_password(), "{probe:?}");
    assert_eq!((probe.service(), probe.scope.as_str()), (Some(sso.base.as_str()), "openid profile email"));
    // MailCraft was given no client id for it: the one its administrator gave is typed.
    assert!(probe.needs_client_id() && probe.signs_in_with(fake_sso::CLIENT_ID), "{probe:?}");

    let added = s.run("account.add", &json!({"email": USER, "jmap": server, "oauth": OIDC, "clientId": fake_sso::CLIENT_ID})).unwrap();
    let id = added["account"].as_str().unwrap().to_string();
    let config = s.account(&id).map(|a| a.config.clone()).unwrap();
    let settings = OidcSettings { issuer: sso.base.clone(), client_id: fake_sso::CLIENT_ID.into(), scope: "openid profile email".into() };
    assert_eq!((config.oauth.as_deref(), config.oidc.as_ref()), (Some(OIDC), Some(&settings)));

    // Signed in in the browser; then its mail comes over a bearer token.
    let holds = |n: usize| store.load_folder(&id, "INBOX").map_or(0, |f| f.messages.len()) >= n;
    let failed =
        |seen: &[Event]| seen.iter().any(|e| matches!(e, Event::Status { status: SyncStatus::NeedsPassword { .. } | SyncStatus::Error { .. }, .. }));
    let seen = drive(&mut s, &mut pool, |_| holds(1));
    assert!(!failed(&seen), "{seen:?}");
    let opened: Vec<&String> = seen
        .iter()
        .filter_map(|e| match e {
            Event::OpenUrl { url } => Some(url),
            _ => None,
        })
        .collect();
    assert!(
        matches!(opened.as_slice(), [url] if url.starts_with(&format!("{}/authorize?", sso.base)) && url.contains("code_challenge_method=S256")),
        "{opened:?}"
    );
    assert!(seen.iter().any(|e| matches!(e, Event::SignedIn { user: Some(u), .. } if u == USER)), "{seen:?}");
    let kept = creds.get_secret(&config, SIGN_ON_SECRET).unwrap_or_default();
    assert!(kept.starts_with("rt") && creds.get(&config).is_none(), "the refresh token alone is kept: {kept}");

    // The server stops taking its token (it ran out early, or was revoked): Send / Receive gets a
    // new one with the refresh token, and the mail still comes, with no password asked for.
    let refreshes = || sso.data.lock().unwrap().forms.iter().filter(|f| f.contains("grant_type=refresh_token")).count();
    let before = refreshes();
    {
        let mut d = jmap.data.lock().unwrap();
        d.tokens = Some(Vec::new());
        d.deliver(&inbox, message(2).as_bytes(), &[], "2026-05-02T09:00:00Z");
    }
    s.run("mail.sync", &json!({})).unwrap();
    let seen = drive(&mut s, &mut pool, |_| holds(2));
    assert!(!failed(&seen), "{seen:?}");
    assert!(refreshes() > before, "the token wasn't renewed");
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

/// An account moved from its password to its server's single sign-on, whose sign-in in the
/// browser was given up: the password still in the keychain is never sent to the service as a
/// refresh token, nor is one given for it. It asks to be signed in.
#[test]
fn a_password_is_never_sent_to_a_single_sign_on_as_its_token() {
    let mut d = Data::default();
    d.tokens = Some(Vec::new());
    d.mailbox("Inbox", None, Some("inbox"));
    let jmap = serve(d);
    let mut sd = fake_sso::Data::default();
    sd.jmap = Some(jmap.data.clone());
    let sso = fake_sso::serve(sd);
    let creds = Arc::new(MemoryCredentials::default());
    let mut pool = Pool::new(Store::memory(), creds.clone());
    let settings = OidcSettings { issuer: sso.base.clone(), client_id: fake_sso::CLIENT_ID.into(), scope: oidc::DEFAULT_SCOPE.into() };
    let account = AccountConfig {
        id: "j1".into(),
        email: USER.into(),
        username: USER.into(),
        jmap: Some(jmap.base.clone()),
        oauth: Some(OIDC.into()),
        oidc: Some(settings),
        ..Default::default()
    };
    creds.set(&account, PASSWORD).unwrap();
    for password in [None, Some(Secret(PASSWORD.into()))] {
        pool.submit(Job::Start { account: account.clone(), password });
        let seen = until(&pool, |e| matches!(e, Event::Status { status: SyncStatus::NeedsPassword { .. } | SyncStatus::Error { .. }, .. }));
        assert!(matches!(seen.last(), Some(Event::Status { status: SyncStatus::NeedsPassword { .. }, .. })), "{seen:?}");
    }
    assert_eq!(sso.data.lock().unwrap().forms, Vec::<String>::new(), "nothing was sent to the service");
    assert_eq!(creds.get_secret(&account, SIGN_ON_SECRET), None);
}

/// Back to a password: the single sign-on's refresh token goes, the password is kept; and an
/// account removed takes its sign-in with it.
#[test]
fn a_single_sign_ons_token_goes_with_the_password_back_and_the_account() {
    let mut d = Data::default();
    d.mailbox("Inbox", None, Some("inbox"));
    let jmap = serve(d);
    let creds = Arc::new(MemoryCredentials::default());
    let mut pool = Pool::new(Store::memory(), creds.clone());
    let account = AccountConfig { id: "j1".into(), email: USER.into(), username: USER.into(), jmap: Some(jmap.base.clone()), ..Default::default() };
    creds.set_secret(&account, SIGN_ON_SECRET, "rt-old").unwrap();
    pool.submit(Job::Start { account: account.clone(), password: Some(Secret(PASSWORD.into())) });
    let seen = until(&pool, |e| matches!(e, Event::Status { status: SyncStatus::Online { .. } | SyncStatus::NeedsPassword { .. }, .. }));
    assert!(matches!(seen.last(), Some(Event::Status { status: SyncStatus::Online { .. }, .. })), "{seen:?}");
    assert_eq!(creds.get(&account).as_deref(), Some(PASSWORD));
    assert_eq!(creds.get_secret(&account, SIGN_ON_SECRET), None, "the refresh token went");

    creds.set_secret(&account, SIGN_ON_SECRET, "rt-old").unwrap();
    pool.submit(Job::Remove { account: account.id.clone() });
    assert_eq!(creds.get_secret(&account, SIGN_ON_SECRET), None, "removed with its account");
    assert_eq!(creds.get(&account), None);
}
