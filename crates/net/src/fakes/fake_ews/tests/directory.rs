//! Issue #9 against the fake EWS server: a server that lets in only the clients it knows, and
//! the organisation's directory asked each time.

use super::*;

const EVOLUTION: &str = "Evolution/3.46.4";

fn people(d: &mut Data) {
    for (n, a) in [
        ("Alice Moreno", "alice.moreno@contoso.example"),
        ("Alicia Keyes", "alicia@contoso.example"),
        ("Bob Stone", "bob@contoso.example"),
        ("Ålesund Office", "alesund@contoso.example"),
    ] {
        d.directory.push((n.into(), a.into()));
    }
}

/// A server that answers 403 to clients it doesn't know (the tester's, with curl: it lets in
/// `Evolution/3.46.4`): refused as MailCraft, let in naming the User-Agent the account gives,
/// on every request, Autodiscover's too.
#[test]
fn a_server_that_lets_in_only_known_clients() {
    let mut d = Data { only_agent: Some(EVOLUTION.into()), ..Default::default() };
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let server = serve(d);
    let user = format!("{DOMAIN}\\{USER}");
    match crate::ews::Client::connect(&server.url, &user, PASSWORD, "riley@contoso.example") {
        Err(crate::NetError::Protocol(m)) => assert!(m.contains("HTTP 403"), "{m}"),
        Err(e) => panic!("refused, but as {e:?}"),
        Ok(_) => panic!("the server lets in MailCraft"),
    }
    assert!(server.data.lock().unwrap().agents.iter().all(|a| a.starts_with("MailCraft/")));

    let c = crate::ews::Client::connect_as(&server.url, &user, PASSWORD, "riley@contoso.example", Some(EVOLUTION)).unwrap();
    assert_eq!(c.mailboxes().unwrap().len(), 1);
    assert_eq!(crate::ews::autodiscover_as("riley@contoso.example", &user, PASSWORD, Some(&server.url), Some(EVOLUTION)).unwrap(), server.url);
    let d = server.data.lock().unwrap();
    let after: Vec<&String> = d.agents.iter().skip_while(|a| a.starts_with("MailCraft/")).collect();
    assert!(!after.is_empty() && after.iter().all(|a| *a == EVOLUTION), "{after:?}");

    // What can't go in a header is never sent: MailCraft's own name instead.
    for bad in ["", "   ", "Evil\r\nX-Injected: 1", "tab\tbed", "émoji 📧"] {
        assert_eq!(mailcraft_model::user_agent(bad), None, "{bad:?}");
    }
    assert_eq!(mailcraft_model::user_agent(&"x".repeat(257)), None);
    assert_eq!(mailcraft_model::user_agent("  Evolution/3.46.4 (Linux)  ").as_deref(), Some("Evolution/3.46.4 (Linux)"));
}

/// The directory: several found (Exchange's warning), one, none (an empty list, not an error),
/// an Exchange-only address left out, a search with markup and controls sent safely.
#[test]
fn the_directory_answers_whoever_matches() {
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    people(&mut d);
    d.directory.push(("Legacy Only".into(), "/o=Contoso/cn=Recipients/cn=legacy".into()));
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    let names = |q: &str| c.search_directory(q).unwrap().into_iter().map(|(_, a)| a).collect::<Vec<_>>();
    assert_eq!(names("ali"), ["alice.moreno@contoso.example", "alicia@contoso.example"]);
    assert_eq!(names("moreno"), ["alice.moreno@contoso.example"]);
    assert_eq!(names("BOB"), ["bob@contoso.example"]);
    assert_eq!(names("ålesund"), ["alesund@contoso.example"]);
    assert!(names("zed").is_empty());
    assert!(names("legacy").is_empty(), "an Exchange address can't be written to");
    assert!(names("  ").is_empty());
    assert!(names("<a>&\u{0}\u{7}").is_empty());
    assert!(c.search_directory(&"a".repeat(100_000)).unwrap().is_empty());
}

/// Issue #9: someone added to the directory while MailCraft runs is found by the next search,
/// through the account's worker, with the account's own User-Agent; a search the server can't
/// answer says why.
#[test]
fn someone_new_in_the_directory_is_found_without_a_restart() {
    let mut d = Data { only_agent: Some(EVOLUTION.into()), ..Default::default() };
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    people(&mut d);
    let server = serve(d);
    let data = server.data.clone();
    let dir = std::env::temp_dir().join(format!("mailcraft-ews-directory-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open(&dir).unwrap();
    let mut pool = crate::worker::Pool::new(store.clone(), Arc::new(crate::worker::MemoryCredentials::default()));
    let account = AccountConfig {
        id: "a1".into(),
        email: "riley@contoso.example".into(),
        username: format!("{DOMAIN}\\{USER}"),
        ews: Some(server.url.clone()),
        user_agent: Some(EVOLUTION.into()),
        ..Default::default()
    };
    pool.submit(Job::Start { account: account.clone(), password: Some(Secret(PASSWORD.into())) });
    until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    let search = |pool: &mut crate::worker::Pool, q: &str| {
        pool.submit(Job::SearchDirectory { account: "a1".into(), query: q.into() });
        let seen = until(pool, "the directory's answer", |e| matches!(e, Event::Directory { query, .. } if query == q));
        seen.into_iter()
            .rev()
            .find_map(|e| match e {
                Event::Directory { people, error, .. } => Some((people, error)),
                _ => None,
            })
            .unwrap()
    };
    let (found, error) = search(&mut pool, "dana");
    assert!(found.is_empty() && error.is_none(), "{found:?} {error:?}");
    data.lock().unwrap().directory.push(("Dana Whitfield".into(), "dana@contoso.example".into()));
    let (found, _) = search(&mut pool, "dana");
    assert_eq!(found, [("Dana Whitfield".to_string(), "dana@contoso.example".to_string())]);
    assert!(data.lock().unwrap().agents.iter().all(|a| a == EVOLUTION));

    // The server refusing the search (its User-Agent rule changed): why, and no one.
    data.lock().unwrap().only_agent = Some("Something/1.0".into());
    let (found, error) = search(&mut pool, "bob");
    assert!(found.is_empty());
    assert!(error.as_deref().is_some_and(|e| e.contains("403")), "{error:?}");
    drop(pool);
    let _ = std::fs::remove_dir_all(store.root());
}

/// Issue #9 through a real front end that refuses unknown clients (HAProxy in Docker, as the
/// tester's server: `http-request deny deny_status 403 unless { req.hdr(user-agent) -m beg
/// Evolution/ }`) before the fake server `serve_ews_for_the_app` runs: refused as MailCraft,
/// synced naming Evolution, and someone new in the directory found while it runs.
/// Run: `MAILCRAFT_EWS_FRONT=http://127.0.0.1:8086/EWS/Exchange.asmx cargo test -p mailcraft-net
/// through_a_front_end -- --ignored --nocapture`.
#[test]
#[ignore]
fn through_a_front_end_that_checks_the_user_agent() {
    let Ok(front) = std::env::var("MAILCRAFT_EWS_FRONT") else { return };
    let dir = std::env::temp_dir().join(format!("mailcraft-ews-front-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open(&dir).unwrap();
    let mut pool = crate::worker::Pool::new(store.clone(), Arc::new(crate::worker::MemoryCredentials::default()));
    let mut account = AccountConfig {
        id: "front".into(),
        email: "riley@contoso.example".into(),
        username: format!("{DOMAIN}\\{USER}"),
        ews: Some(front),
        ..Default::default()
    };
    pool.submit(Job::Start { account: account.clone(), password: Some(Secret(PASSWORD.into())) });
    let seen = until(&pool, "refused", |e| matches!(e, Event::Status { status: SyncStatus::Error { .. }, .. }));
    let why = seen.iter().rev().find_map(|e| match e {
        Event::Status { status: SyncStatus::Error { error }, .. } => Some(error.clone()),
        _ => None,
    });
    println!("as MailCraft: {why:?}");
    assert!(why.as_deref().is_some_and(|m| m.contains("403")), "{why:?}");

    account.user_agent = Some(EVOLUTION.into());
    pool.submit(Job::Start { account, password: Some(Secret(PASSWORD.into())) });
    until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    println!("as {EVOLUTION}: online");
    let search = |pool: &mut crate::worker::Pool, q: &str| {
        pool.submit(Job::SearchDirectory { account: "front".into(), query: q.into() });
        let seen = until(pool, "the directory's answer", |e| matches!(e, Event::Directory { query, .. } if query == q));
        seen.into_iter()
            .rev()
            .find_map(|e| match e {
                Event::Directory { people, error, .. } => Some((people, error)),
                _ => None,
            })
            .unwrap()
    };
    let (ali, error) = search(&mut pool, "ali");
    println!("ali: {ali:?} {error:?}");
    assert_eq!(ali.len(), 2);
    // Dana starts 45 seconds after the server: asked until she's there.
    let began = Instant::now();
    loop {
        let (dana, error) = search(&mut pool, "dana");
        assert!(error.is_none(), "{error:?}");
        if !dana.is_empty() {
            println!("dana found after {:?}: {dana:?}", began.elapsed());
            break;
        }
        assert!(began.elapsed() < Duration::from_secs(120), "never found");
        std::thread::sleep(Duration::from_secs(5));
    }
    drop(pool);
    let _ = std::fs::remove_dir_all(store.root());
}
