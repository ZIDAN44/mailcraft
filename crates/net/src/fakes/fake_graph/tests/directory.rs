//! Issue #9 online, against the fake Graph server: Exchange Online's directory searched with a token of its
//! own, asked for in a sign-in of its own, never with the mail's.

use super::*;

fn people(d: &mut Data) {
    for (n, a) in [("Alice Moreno", "alice.moreno@contoso.example"), ("Alicia Keyes", "alicia@contoso.example"), ("Bob Stone", "bob@contoso.example")]
    {
        d.directory.push((n.into(), a.into()));
    }
}

/// HTTP answers from a token endpoint, one per connection, in order.
fn token_answers(answers: &[(&str, &str)]) -> (String, std::thread::JoinHandle<Vec<String>>) {
    let replies = answers
        .iter()
        .map(|(status, json)| {
            let mut r = format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: application/json\r\n\r\n", json.len()).into_bytes();
            r.extend_from_slice(json.as_bytes());
            r
        })
        .collect();
    crate::http::tests::serve(replies)
}

/// The search: `/users` with the header Microsoft's directory wants for `$search`; refused
/// there, `/me/people` with the directory's people asked for; nothing found is no one; a search
/// with quotes, backslashes and controls sent safely; refused everywhere is a missing permission.
#[test]
fn the_directory_is_searched_by_users_then_people() {
    let mut d = Data { directory_token: Some("dir".into()), ..Default::default() };
    people(&mut d);
    let server = serve(d);
    let find = |q: &str| crate::graph::search_directory(&server.base, "dir", q).unwrap().into_iter().map(|(_, a)| a).collect::<Vec<_>>();
    assert_eq!(find("ali"), ["alice.moreno@contoso.example", "alicia@contoso.example"]);
    assert_eq!(find("moreno"), ["alice.moreno@contoso.example"]);
    assert!(find("zed").is_empty());
    assert!(find("  ").is_empty());
    assert_eq!(find("b\"o\\b\u{0}"), ["bob@contoso.example"], "quotes, backslashes and controls dropped");
    assert!(crate::graph::search_directory(&server.base, "dir", &"a".repeat(10_000)).unwrap().is_empty());
    {
        let d = server.data.lock().unwrap();
        let users: Vec<&String> = d.calls.iter().filter(|c| c.starts_with("GET /users?")).collect();
        assert!(users.iter().all(|c| c.contains("$count=true") && c.contains("$select=displayName,mail")), "{users:?}");
    }
    // The mail's token is refused there, as a token without the permission is.
    let e = crate::graph::search_directory(&server.base, TOKEN, "ali").unwrap_err();
    assert!(crate::graph::needs_consent(&e), "{e:?}");

    // A tenant that won't list its users: the relevant people, the directory's with them.
    server.data.lock().unwrap().users_forbidden = true;
    assert_eq!(find("bob"), ["bob@contoso.example"]);
    assert!(server.data.lock().unwrap().calls.iter().any(|c| c.starts_with("GET /me/people?")));
    // Never plain http away from this computer.
    assert!(crate::graph::search_directory("http://graph.example/v1.0", "dir", "ali").is_err());
}

/// Through the worker: with no permission yet the account says so (no token asked for); once
/// allowed, the directory's own refresh token gets a token for the directory's permissions alone,
/// its replacement kept, the mail's token and refreshes untouched; the permission taken away is
/// asked for again.
#[test]
fn exchange_online_searches_with_a_token_of_its_own() {
    let (oauth_base, token_requests) = token_answers(&[
        ("200 OK", r#"{"access_token":"mail-1","expires_in":3600}"#),
        ("200 OK", r#"{"access_token":"dir-1","refresh_token":"dr2","expires_in":3600}"#),
    ]);
    let mut p = crate::oauth::tests::local(&format!("{oauth_base}/token"));
    p.id = "test-graph-directory".into();
    crate::oauth::TEST_PROVIDERS.lock().unwrap().push(p);

    let mut d = Data { token: Some("mail-1".into()), directory_token: Some("dir-1".into()), ..Default::default() };
    let inbox = d.folder("Inbox", None, Some("inbox"));
    d.deliver(&inbox, &mail("Hello", "m1"), true, "2024-05-06T09:00:00Z");
    people(&mut d);
    let server = serve(d);
    let store = crate::fake::tests_store("graph-directory");
    let creds = Arc::new(crate::worker::MemoryCredentials::default());
    let mut pool = crate::worker::Pool::new(store.clone(), creds.clone());
    let account = AccountConfig {
        id: "a1".into(),
        email: "me@contoso.example".into(),
        graph: Some(server.base.clone()),
        oauth: Some("test-graph-directory".into()),
        ..Default::default()
    };
    pool.submit(Job::Start { account: account.clone(), password: Some(Secret("r1".into())) });
    until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    let search = |pool: &mut crate::worker::Pool, q: &str| {
        pool.submit(Job::SearchDirectory { account: "a1".into(), query: q.into() });
        let seen = until(pool, "the directory's answer", |e| matches!(e, Event::Directory { query, .. } if query == q));
        seen.into_iter()
            .rev()
            .find_map(|e| match e {
                Event::Directory { people, error, consent, .. } => Some((people, error, consent)),
                _ => None,
            })
            .unwrap()
    };

    let (found, error, consent) = search(&mut pool, "ali");
    assert!(found.is_empty() && error.is_some() && consent, "no permission yet: {error:?}");

    crate::worker::Credentials::set_secret(creds.as_ref(), &account, "directory", "dr1").unwrap();
    let (found, error, consent) = search(&mut pool, "ali");
    assert_eq!((found.len(), error, consent), (2, None, false));
    assert_eq!(crate::worker::Credentials::get_secret(creds.as_ref(), &account, "directory").as_deref(), Some("dr2"), "the new one kept");
    // Asked again while the token lasts: no new token.
    assert_eq!(search(&mut pool, "bob").0.len(), 1);
    // The mail goes on with its own token.
    server.data.lock().unwrap().deliver(&inbox, &mail("After", "m2"), false, "2024-05-06T10:00:00Z");
    pool.submit(Job::SyncFolder { account: "a1".into(), folder: "INBOX".into() });
    eventually("mail synced with its own token", || store.load_folder("a1", "INBOX").is_ok_and(|f| f.messages.iter().any(|m| m.subject == "After")));

    // The permission taken away: said, to be asked for again.
    server.data.lock().unwrap().directory_token = Some("dir-2".into());
    let (_, error, consent) = search(&mut pool, "ali");
    assert!(consent, "{error:?}");

    pool.submit(Job::Remove { account: "a1".into() });
    let seen = token_requests.join().unwrap();
    let [mail_refresh, directory_refresh] = seen.as_slice() else { panic!("{seen:?}") };
    assert!(mail_refresh.contains("refresh_token=r1") && !mail_refresh.contains("User.ReadBasic.All"), "{mail_refresh}");
    assert!(directory_refresh.contains("refresh_token=dr1"), "{directory_refresh}");
    for scope in ["User.ReadBasic.All", "People.Read", "offline_access"] {
        assert!(directory_refresh.contains(scope), "{scope}: {directory_refresh}");
    }
    assert!(!directory_refresh.contains("Mail."), "{directory_refresh}");
    eventually("its directory sign-in removed with it", || crate::worker::Credentials::get_secret(creds.as_ref(), &account, "directory").is_none());
    let _ = std::fs::remove_dir_all(store.root());
}

/// Allowing it: the browser asked for the directory's permissions alone (no mail ones), the
/// lasting sign-in kept apart; a refusal said, nothing kept; an account that isn't Exchange
/// Online refused at once.
#[test]
fn the_directory_is_allowed_in_a_sign_in_of_its_own() {
    let (oauth_base, _token_requests) = token_answers(&[("200 OK", r#"{"access_token":"dir-1","refresh_token":"dr1","expires_in":3600}"#)]);
    let mut p = crate::oauth::tests::local(&format!("{oauth_base}/token"));
    p.id = "test-graph-allow".into();
    crate::oauth::TEST_PROVIDERS.lock().unwrap().push(p);
    let store = crate::fake::tests_store("graph-allow");
    let creds = Arc::new(crate::worker::MemoryCredentials::default());
    let mut pool = crate::worker::Pool::new(store.clone(), creds.clone());
    let account = AccountConfig {
        id: "a1".into(),
        email: "me@contoso.example".into(),
        graph: Some("http://127.0.0.1:9/v1.0".into()),
        oauth: Some("test-graph-allow".into()),
        ..Default::default()
    };
    // The "browser" answers the page MailCraft opens: allowed, then refused.
    for (allow, kept) in [(true, true), (false, false)] {
        crate::worker::Credentials::delete_secret(creds.as_ref(), &account, "directory");
        pool.submit(Job::AllowDirectory { account: account.clone() });
        let seen = until(&pool, "the page to open", |e| matches!(e, Event::OpenUrl { .. }));
        let Some(Event::OpenUrl { url }) = seen.last().cloned() else { panic!() };
        let scope = crate::oauth::unpct(url.split("scope=").nth(1).and_then(|s| s.split('&').next()).unwrap_or_default());
        assert!(scope.contains("User.ReadBasic.All") && scope.contains("People.Read") && !scope.contains("Mail."), "{scope}");
        let get = |k: &str| crate::oauth::unpct(url.split(&format!("{k}=")).nth(1).and_then(|s| s.split('&').next()).unwrap_or_default());
        let answer = if allow { format!("code=the-code&state={}", get("state")) } else { format!("error=access_denied&state={}", get("state")) };
        let _ = crate::http::get(&format!("{}?{answer}", get("redirect_uri")), 10_000);
        let seen = until(&pool, "allowed or not", |e| matches!(e, Event::DirectoryAllowed { .. }));
        let Some(Event::DirectoryAllowed { error, .. }) = seen.last().cloned() else { panic!() };
        assert_eq!(error.is_none(), allow, "{error:?}");
        assert_eq!(crate::worker::Credentials::get_secret(creds.as_ref(), &account, "directory").is_some(), kept);
    }
    let imap = AccountConfig { id: "a2".into(), email: "x@mail.example".into(), ..Default::default() };
    pool.submit(Job::AllowDirectory { account: imap });
    let seen = until(&pool, "refused", |e| matches!(e, Event::DirectoryAllowed { account, .. } if account == "a2"));
    assert!(matches!(seen.last(), Some(Event::DirectoryAllowed { error: Some(_), .. })));
    let _ = std::fs::remove_dir_all(store.root());
}
