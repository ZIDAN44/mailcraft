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

/// Issue #9 (follow-up): a server that names each mailbox by its alias (`alice.moreno`) gives the
/// full name in the contact; MailCraft asks for it and shows it, for anyone it has never had mail
/// from. A contact with only a given name and surname, or none at all, falls back in turn.
#[test]
fn people_found_in_the_directory_have_their_full_names() {
    let mut d = Data { directory_aliases: true, ..Default::default() };
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    people(&mut d);
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    assert_eq!(
        c.search_directory("ali").unwrap(),
        [("Alice Moreno".to_string(), "alice.moreno@contoso.example".to_string()), ("Alicia Keyes".into(), "alicia@contoso.example".into())]
    );
    assert_eq!(c.search_directory("ålesund").unwrap(), [("Ålesund Office".to_string(), "alesund@contoso.example".to_string())]);

    let name = |contact: &str| {
        let doc = crate::ews::parse(&format!(
            "<Resolution><Mailbox><Name>j.doe</Name><EmailAddress>j.doe@contoso.example</EmailAddress></Mailbox>{contact}</Resolution>"
        ));
        let r = doc.find("Resolution").unwrap_or(&doc);
        crate::ews::directory_name(r)
    };
    assert_eq!(name("<Contact><DisplayName> Jane Doe </DisplayName></Contact>"), "Jane Doe");
    assert_eq!(name("<Contact><DisplayName/><GivenName>Jane</GivenName><Surname>Doe</Surname></Contact>"), "Jane Doe");
    assert_eq!(name("<Contact><Surname>Doe</Surname></Contact>"), "Doe");
    assert_eq!(name("<Contact><DisplayName>Ja\u{7}ne</DisplayName></Contact>"), "Jane");
    assert_eq!(name(""), "j.doe", "no contact: the mailbox's name");
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
    let dir = crate::data_files::scratch::scratch("ews-directory");
    let store = Store::open(&dir).unwrap();
    let mut pool = crate::worker::Pool::new(store.clone(), Arc::new(crate::worker::MemoryCredentials::default()));
    let account = AccountConfig { user_agent: Some(EVOLUTION.into()), ..account("a1", &server.url) };
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
    assert_eq!(
        found.iter().map(|p| (p.name.clone(), p.email.clone())).collect::<Vec<_>>(),
        [("Dana Whitfield".to_string(), "dana@contoso.example".to_string())]
    );
    assert!(data.lock().unwrap().agents.iter().all(|a| a == EVOLUTION));

    // The server refusing the search (its User-Agent rule changed): why, and no one.
    data.lock().unwrap().only_agent = Some("Something/1.0".into());
    let (found, error) = search(&mut pool, "bob");
    assert!(found.is_empty());
    assert!(error.as_deref().is_some_and(|e| e.contains("403")), "{error:?}");
    drop(pool);
}

/// Issue #9 through a real front end that refuses unknown clients (HAProxy in Docker, as the
/// tester's server: `http-request deny deny_status 403 unless { req.hdr(user-agent) -m beg
/// Evolution/ }`) before the fake server `serve_ews_for_the_app` runs: refused as MailCraft,
/// synced naming Evolution, and someone new in the directory found while it runs.
/// Run: `MAILCRAFT_EWS_FRONT=http://127.0.0.1:8086/EWS/Exchange.asmx cargo test -p mailcraft-net
/// through_a_front_end -- --ignored --nocapture`.
#[test]
#[ignore = "needs MAILCRAFT_EWS_FRONT, a front end in Docker: see the doc"]
fn through_a_front_end_that_checks_the_user_agent() {
    let Ok(front) = std::env::var("MAILCRAFT_EWS_FRONT") else { panic!("set MAILCRAFT_EWS_FRONT to run it (see its doc)") };
    let dir = crate::data_files::scratch::scratch("ews-front");
    let store = Store::open(&dir).unwrap();
    let mut pool = crate::worker::Pool::new(store.clone(), Arc::new(crate::worker::MemoryCredentials::default()));
    let mut account = AccountConfig { id: "front".into(), ..account("a1", &front) };
    pool.submit(Job::Start { account: account.clone(), password: Some(Secret(PASSWORD.into())) });
    let seen = until(&pool, "refused", |e| matches!(e, Event::Status { status: SyncStatus::Error { .. }, .. }));
    let why = seen.iter().rev().find_map(|e| match e {
        Event::Status { status: SyncStatus::Error { error, .. }, .. } => Some(error.clone()),
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
}

/// Gap 5 through a real Kerberos front (MIT krb5 and Apache's mod_auth_gssapi in Docker,
/// `GssapiAllowedMech krb5`, Negotiate only) before the fake server, `MAILCRAFT_FAKE_OPEN`, and
/// the realm's KDC at `MAILCRAFT_KDC`: signed in with Kerberos and synced through the worker; a
/// wrong password refused as one; an unknown user too.
/// Run: `MAILCRAFT_EWS_KERBEROS=http://localhost:8087/EWS/Exchange.asmx MAILCRAFT_KDC=127.0.0.1:88
/// cargo test -p mailcraft-net through_a_kerberos -- --ignored --nocapture`.
#[test]
#[ignore = "needs MAILCRAFT_EWS_KERBEROS and MAILCRAFT_KDC, a Kerberos front in Docker: see the doc"]
fn through_a_kerberos_front() {
    let Ok(front) = std::env::var("MAILCRAFT_EWS_KERBEROS") else { panic!("set MAILCRAFT_EWS_KERBEROS to run it (see its doc)") };
    let c = crate::ews::Client::connect(&front, "riley@contoso.test", PASSWORD, "riley@contoso.test").unwrap();
    println!("signed in with Kerberos: {} folders", c.mailboxes().unwrap().len());
    // As Exchange users type it, the domain's NetBIOS name before the user: the realm from the
    // address.
    let c = crate::ews::Client::connect(&front, "CONTOSO\\riley", PASSWORD, "riley@contoso.test").unwrap();
    println!("CONTOSO\\riley signed in with Kerberos: {} folders", c.mailboxes().unwrap().len());
    for (user, password) in [("riley@contoso.test", "wrong"), ("nobody@contoso.test", PASSWORD)] {
        match crate::ews::Client::connect(&front, user, password, "riley@contoso.test") {
            Err(crate::NetError::Auth(m)) => println!("{user} / {password}: refused: {m}"),
            other => panic!("{user} / {password}: {:?}", other.map(|_| ())),
        }
    }
    let dir = crate::data_files::scratch::scratch("ews-kerberos");
    let store = Store::open(&dir).unwrap();
    let mut pool = crate::worker::Pool::new(store.clone(), Arc::new(crate::worker::MemoryCredentials::default()));
    let account = AccountConfig {
        id: "krb".into(),
        email: "riley@contoso.test".into(),
        username: "riley@contoso.test".into(),
        ews: Some(front),
        ..Default::default()
    };
    pool.submit(Job::Start { account, password: Some(Secret(PASSWORD.into())) });
    until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    let inbox = store.load_folders("krb").unwrap().into_iter().find(|f| f.role == mailcraft_model::FolderRole::Inbox).unwrap();
    let mail = store.load_folder("krb", &inbox.path).unwrap().messages;
    println!("synced through Kerberos: {} in {}", mail.len(), inbox.path);
    assert!(!mail.is_empty());
    drop(pool);
}

/// Issue #9 (follow-up) against a real EWS server, grommunio (Docker, `gromox-container`), before
/// a plain-HTTP front: someone with no display name, only a given name and surname, is named by
/// them, not by their alias as the server's mailbox name has it; someone with one, by it.
/// Run: `MAILCRAFT_GROMMUNIO=http://127.0.0.1:18090/EWS/Exchange.asmx MAILCRAFT_GROMMUNIO_USER=riley@studio.test
/// MAILCRAFT_GROMMUNIO_PASSWORD=… cargo test -p mailcraft-net against_grommunio -- --ignored --nocapture`.
#[test]
#[ignore = "needs MAILCRAFT_GROMMUNIO and its user and password, grommunio in Docker: see the doc"]
fn against_grommunio() {
    let (Ok(url), Ok(user), Ok(password)) =
        (std::env::var("MAILCRAFT_GROMMUNIO"), std::env::var("MAILCRAFT_GROMMUNIO_USER"), std::env::var("MAILCRAFT_GROMMUNIO_PASSWORD"))
    else {
        panic!("set MAILCRAFT_GROMMUNIO, MAILCRAFT_GROMMUNIO_USER and MAILCRAFT_GROMMUNIO_PASSWORD to run it (see its doc)");
    };
    let dir = crate::data_files::scratch::scratch("grommunio");
    let mut pool = crate::worker::Pool::new(Store::open(&dir).unwrap(), Arc::new(crate::worker::MemoryCredentials::default()));
    let account = AccountConfig { id: "grom".into(), email: user.clone(), username: user, ews: Some(url), ..Default::default() };
    pool.submit(Job::Start { account, password: Some(Secret(password)) });
    until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    let mut search = |q: &str| {
        pool.submit(Job::SearchDirectory { account: "grom".into(), query: q.into() });
        let seen = until(&pool, "the directory's answer", |e| matches!(e, Event::Directory { query, .. } if query == q));
        seen.into_iter()
            .rev()
            .find_map(|e| match e {
                Event::Directory { people, error, .. } => Some((people, error)),
                _ => None,
            })
            .unwrap()
    };
    let (dana, error) = search("dana");
    println!("dana: {dana:?} {error:?}");
    assert_eq!(
        dana.iter().map(|p| (p.name.clone(), p.email.clone())).collect::<Vec<_>>(),
        [("Dana Whitfield".to_string(), "dana.noname@studio.test".to_string())]
    );
    let (john, error) = search("john");
    println!("john: {john:?} {error:?}");
    assert_eq!(
        john.iter().map(|p| (p.name.clone(), p.email.clone())).collect::<Vec<_>>(),
        [("John Doe".to_string(), "john.doe@studio.test".to_string())]
    );
}

/// Issue #222: a colleague's calendar opened here is named as the directory names them, not by
/// their address; one the directory doesn't know keeps the address. Each is asked once on the
/// connection, not at every sync.
#[test]
fn a_colleagues_calendar_has_their_name() {
    use mailcraft_model::pim::{Pim, shared_calendar_id};
    let mut d = Data::default();
    people(&mut d);
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    d.folder("Contacts", None, Some("contacts"), "IPF.Contact");
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    let pim = Pim { shared: vec!["bob@contoso.example".into(), "zed@contoso.example".into()], ..Default::default() };
    let names = |c: &crate::ews::Client| {
        let sync = crate::ews_pim::sync(c, &pim, 1_792_000_000).unwrap();
        let calendars = sync.calendars.unwrap_or_default();
        let name = |who: &str| calendars.iter().find(|x| x.id == shared_calendar_id(who)).map(|x| x.name.clone());
        (name("bob@contoso.example"), name("zed@contoso.example"))
    };
    assert_eq!(names(&c), (Some("Bob Stone".into()), Some("zed@contoso.example".into())));
    let asked = server.data.lock().unwrap().calls.iter().filter(|op| *op == "ResolveNames").count();
    assert_eq!(names(&c), (Some("Bob Stone".into()), Some("zed@contoso.example".into())));
    assert_eq!(server.data.lock().unwrap().calls.iter().filter(|op| *op == "ResolveNames").count(), asked, "not asked again");
}

/// Issue #213: the directory's people come with their job title and department, as the full
/// contact data gives them; one without says none.
#[test]
fn the_directory_gives_titles_and_departments() {
    let mut d = Data::default();
    people(&mut d);
    d.directory_titles.push(("alice.moreno@contoso.example".into(), "Colourist".into(), "Post-production".into()));
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    let found = c.search_directory_full("ali").unwrap();
    let of = |a: &str| found.iter().find(|p| p.email == a).map(|p| (p.name.as_str(), p.title.as_str(), p.department.as_str()));
    assert_eq!(of("alice.moreno@contoso.example"), Some(("Alice Moreno", "Colourist", "Post-production")));
    assert_eq!(of("alicia@contoso.example"), Some(("Alicia Keyes", "", "")));
}

/// Issue #223: a person's photo, as the organisation has it (`GetUserPhoto`): one with none is
/// HTTP 404 and no photo, not a failure; what isn't an address isn't asked.
#[test]
fn a_persons_photo_comes_from_the_server() {
    let photo = b"\x89PNG\r\n\x1a\nAlice".to_vec();
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.photos.push(("alice.moreno@contoso.example".into(), photo.clone()));
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    assert_eq!(c.user_photo("Alice.Moreno@contoso.example").unwrap(), Some(photo.clone()));
    assert_eq!(c.user_photo("bob@contoso.example").unwrap(), None, "none: HTTP 404");
    assert_eq!(c.user_photo("not an address").unwrap(), None);
    assert_eq!(server.data.lock().unwrap().calls.iter().filter(|c| *c == "GetUserPhoto").count(), 2, "an address only");
}
