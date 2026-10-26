//! Issue #9: an Exchange account's own User-Agent, and the organisation directory looked up as
//! names are searched for, what it answers kept only briefly.

use super::*;

/// An EWS account in the sample session, with what adding it queued taken.
fn exchange(s: &mut Session, agent: Option<&str>) -> String {
    let mut p = json!({"email": "riley@contoso.example", "password": "pw", "username": "CONTOSO\\riley", "ews": "mail.contoso.example"});
    if let Some(a) = agent {
        p["userAgent"] = json!(a);
    }
    let id = s.run("account.add", &p).unwrap()["account"].as_str().unwrap().to_string();
    s.take_jobs();
    id
}

fn answer(s: &mut Session, account: &str, query: &str, people: &[(&str, &str)]) {
    let people = people.iter().map(|(n, a)| (n.to_string(), a.to_string())).collect();
    s.apply(Event::Directory { account: account.into(), query: query.into(), people, error: None, consent: false });
}

fn asked(s: &mut Session) -> Vec<String> {
    s.take_jobs()
        .into_iter()
        .filter_map(|j| match j {
            Job::SearchDirectory { query, .. } => Some(query),
            _ => None,
        })
        .collect()
}

#[test]
fn an_exchange_account_names_the_user_agent_it_is_given() {
    let mut s = sample_session("user-agent");
    assert!(
        s.run("account.add", &json!({"email": "x@contoso.example", "password": "pw", "ews": true, "userAgent": "Evil\r\nX-Injected: 1"}))
            .unwrap_err()
            .to_string()
            .contains("User-Agent")
    );
    let id = exchange(&mut s, Some("  Evolution/3.46.4  "));
    assert_eq!(s.account(&id).unwrap().config.user_agent.as_deref(), Some("Evolution/3.46.4"));
    let listed = s.run("account.list", &json!({})).unwrap();
    assert!(listed.as_array().unwrap().iter().any(|a| a["userAgent"] == "Evolution/3.46.4"), "{listed}");
    // Changed: the account connects again naming it; emptied: MailCraft's own.
    s.run("account.update", &json!({"account": id, "userAgent": "Evolution/3.50.0"})).unwrap();
    let jobs = s.take_jobs();
    assert!(jobs.iter().any(|j| matches!(j, Job::Start { account, .. } if account.user_agent.as_deref() == Some("Evolution/3.50.0"))), "{jobs:?}");
    s.run("account.update", &json!({"account": id, "userAgent": ""})).unwrap();
    assert_eq!(s.account(&id).unwrap().config.user_agent, None);
    assert!(s.run("account.update", &json!({"account": id, "userAgent": "é".repeat(10)})).is_err());
    assert!(s.run("account.update", &json!({"account": id, "userAgent": 7})).is_err());
    // A user name with a line break in it (a `\r` from JSON) is refused, not kept to fail.
    assert!(s.run("account.update", &json!({"account": id, "username": "CONTOSO\riley"})).unwrap_err().to_string().contains("user name"));
    assert!(s.run("account.add", &json!({"email": "q@contoso.example", "password": "pw", "ews": true, "username": "CONTOSO\tq"})).is_err());
    // Kept with the account, read back after a restart.
    s.run("account.update", &json!({"account": id, "userAgent": "Evolution/3.46.4"})).unwrap();
    let saved = std::fs::read_to_string(s.store.as_ref().unwrap().root().join("accounts.json")).unwrap();
    assert!(saved.contains("\"user_agent\": \"Evolution/3.46.4\"") || saved.contains("\"user_agent\":\"Evolution/3.46.4\""), "{saved}");
    // Only Exchange's requests name it: an IMAP account takes none.
    let imap = s
        .run(
            "account.add",
            &json!({"email": "zoe@mail.example", "password": "pw", "imapHost": "mail.example", "smtpHost": "mail.example", "userAgent": "X/1"}),
        )
        .unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(s.account(&imap).unwrap().config.user_agent, None);
    done(s);
}

/// The directory is asked as a name is typed (three letters or more), once at a time; a list of
/// people stays five minutes, "no one" half a minute, so someone added on the server is found
/// soon without a restart; Refresh forgets it all and reads the address books again.
#[test]
fn the_directory_is_asked_again_once_its_answer_is_old() {
    let mut s = sample_session("directory");
    assert!(s.run("contact.searchDirectory", &json!({"query": "ali"})).is_err(), "no Exchange account: no directory");
    let id = exchange(&mut s, None);
    s.clock.now = 1_000_000;

    // Two letters: not by itself; three: asked, once while it's being answered.
    assert!(s.address_book("directory", "al").is_empty());
    assert!(asked(&mut s).is_empty());
    s.address_book("directory", "Ali");
    s.address_book("all", "ali");
    s.suggest_contacts("ali", 8);
    assert_eq!(asked(&mut s), ["ali"]);
    assert!(s.directory_pending("ali"));
    answer(&mut s, &id, "ali", &[("Alicia Keyes", "alicia@contoso.example"), ("Alice Moreno", "alice.moreno@contoso.example")]);
    assert!(!s.directory_pending("ali"));
    let found: Vec<String> = s.address_book("directory", "ali").into_iter().map(|a| a.email).collect();
    assert_eq!(found, ["alice.moreno@contoso.example", "alicia@contoso.example"], "by name");
    assert!(s.address_book("all", "ali").iter().any(|a| a.email == "alicia@contoso.example"), "everyone includes the directory's");
    assert!(s.suggest_contacts("ali", 8).iter().any(|c| c.address.email == "alice.moreno@contoso.example"), "suggested while writing");

    // Typed on, while the longer search is asked: the shorter one's answer, narrowed.
    let narrowed: Vec<String> = s.address_book("directory", "alice").into_iter().map(|a| a.email).collect();
    assert_eq!(narrowed, ["alice.moreno@contoso.example"]);
    assert_eq!(asked(&mut s), ["alice"]);

    // Fresh for five minutes, then asked again.
    s.clock.now += 299;
    s.address_book("directory", "ali");
    assert!(asked(&mut s).is_empty());
    s.clock.now += 2;
    s.address_book("directory", "ali");
    assert_eq!(asked(&mut s), ["ali"], "five minutes on");
    answer(&mut s, &id, "ali", &[("Alicia Keyes", "alicia@contoso.example")]);

    // No one: asked again half a minute later (the new employee of the issue).
    s.address_book("directory", "dana");
    assert_eq!(asked(&mut s), ["dana"]);
    answer(&mut s, &id, "dana", &[]);
    assert!(s.address_book("directory", "dana").is_empty());
    assert!(asked(&mut s).is_empty());
    s.clock.now += 31;
    s.address_book("directory", "dana");
    assert_eq!(asked(&mut s), ["dana"]);
    answer(&mut s, &id, "dana", &[("Dana Whitfield", "dana@contoso.example")]);
    assert_eq!(s.address_book("directory", "dana").len(), 1);

    // Go asks now, whatever was answered, and for a short search too.
    let r = s.run("contact.searchDirectory", &json!({"query": "da"})).unwrap();
    assert_eq!(r["pending"], true);
    assert_eq!(asked(&mut s), ["da"]);
    s.run("contact.searchDirectory", &json!({"query": "dana"})).unwrap();
    assert_eq!(asked(&mut s), ["dana"]);

    // A failure says why, and keeps who was found before.
    s.apply(Event::Directory {
        account: id.clone(),
        query: "dana".into(),
        people: Vec::new(),
        error: Some("Exchange answered HTTP 403".into()),
        consent: false,
    });
    assert_eq!(s.address_book("directory", "dana").len(), 1);
    assert!(s.directory_errors("dana").iter().any(|e| e.contains("riley@contoso.example") && e.contains("403")));

    // Refresh: everything forgotten, the address books read again, the next search asked.
    let rev = s.directory.rev;
    s.run("contact.refreshDirectory", &json!({})).unwrap();
    assert!(s.take_jobs().iter().any(|j| matches!(j, Job::SyncDav { account } if *account == id)));
    assert!(s.directory.rev != rev && s.directory.answers.is_empty());
    // An answer to a search asked before the refresh isn't kept.
    answer(&mut s, &id, "zed", &[("Zed", "zed@contoso.example")]);
    assert!(s.directory.answers.is_empty());
    s.address_book("directory", "dana");
    assert_eq!(asked(&mut s), ["dana"]);

    // Junk from the server: addresses that aren't one, left out.
    answer(&mut s, &id, "dana", &[("Bad", "not an address"), ("Two", "a@x.example, b@x.example"), ("Dana", "dana@contoso.example")]);
    assert_eq!(s.address_book("directory", "dana").len(), 1);

    // Many searches: the oldest answers make room, nothing grows without end.
    for i in 0..500 {
        s.address_book("directory", &format!("name{i}"));
        answer(&mut s, &id, &format!("name{i}"), &[]);
        s.clock.now += 1;
    }
    assert!(s.directory.answers.len() <= 200, "{}", s.directory.answers.len());
    done(s);
}

/// Issue #9, online: an Exchange Online account searches its directory too, once the user allows it. An
/// answer saying it needs permission keeps it waiting (no error shown for it, the dialog asks
/// instead); allowing it opens the browser for that account, and once allowed its searches are
/// asked again; a refusal is said.
#[test]
fn exchange_online_asks_for_permission_to_search_its_directory() {
    let mut s = sample_session("directory-graph");
    s.oauth_available = vec!["microsoft".into()];
    let id = s.run("account.add", &json!({"email": "me@contoso.example", "graph": true})).unwrap()["account"].as_str().unwrap().to_string();
    s.take_jobs();
    assert!(s.directory_accounts().contains(&id));
    assert!(s.run("contact.allowDirectory", &json!({})).is_err(), "nobody waiting yet");
    s.address_book("directory", "ali");
    assert_eq!(asked(&mut s), ["ali"]);
    s.apply(Event::Directory {
        account: id.clone(),
        query: "ali".into(),
        people: Vec::new(),
        error: Some("searching your organisation's directory needs your permission first".into()),
        consent: true,
    });
    assert!(s.directory.consent.contains(&id));
    assert!(s.directory_errors("ali").is_empty(), "asked for, not shown as an error");
    s.run("contact.allowDirectory", &json!({})).unwrap();
    let jobs = s.take_jobs();
    assert!(matches!(jobs.as_slice(), [Job::AllowDirectory { account }] if account.id == id), "{jobs:?}");

    // Refused in the browser: said, still waiting.
    s.apply(Event::DirectoryAllowed { account: id.clone(), error: Some("Microsoft sign-in was cancelled (access_denied)".into()) });
    assert!(s.directory.consent.contains(&id));
    // Allowed: the account's answers forgotten and asked again.
    let rev = s.directory.rev;
    s.apply(Event::DirectoryAllowed { account: id.clone(), error: None });
    assert!(!s.directory.consent.contains(&id) && s.directory.rev != rev);
    s.address_book("directory", "ali");
    assert_eq!(asked(&mut s), ["ali"]);
    answer(&mut s, &id, "ali", &[("Alicia Keyes", "alicia@contoso.example")]);
    assert_eq!(s.address_book("directory", "ali").len(), 1);

    // Only an Exchange Online account signed in with Microsoft asks.
    let imap = s
        .run("account.add", &json!({"email": "zoe@mail.example", "password": "pw", "imapHost": "mail.example", "smtpHost": "mail.example"}))
        .unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(s.run("contact.allowDirectory", &json!({"account": imap})).is_err());
    // A personal Microsoft account has no organisation directory to search.
    let personal = s.run("account.add", &json!({"email": "zoe@outlook.com", "graph": true})).unwrap()["account"].as_str().unwrap().to_string();
    assert!(!s.directory_accounts().contains(&personal));
    done(s);
}
