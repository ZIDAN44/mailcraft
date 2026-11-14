//! A JMAP server behind a single sign-on (OpenID Connect): how it's signed in to, asked before
//! its account is added, and an account that signs in through it, or stops to.

use mailcraft_model::{AccountConfig, JmapProbe, OIDC, OidcSettings};

use super::*;

const SERVER: &str = "https://jmap.example.com/jmap/session";

fn session(name: &str) -> Session {
    let mut s = Session::open(Some(temp(name)), mailcraft_model::dates::Clock::new(NOW, 0));
    s.oauth_available = vec![OIDC.into()];
    s
}

/// What the server at [`SERVER`] was found to say: bearer tokens only, from its service.
fn answered(s: &mut Session) {
    s.run("account.probeJmap", &json!({"url": SERVER})).unwrap();
    s.take_jobs();
    let probe = JmapProbe {
        bearer: true,
        issuer: Some("https://sso.example.com/".into()),
        scope: "openid email".into(),
        client_id: Some("mailcraft".into()),
        ..JmapProbe::default()
    };
    s.apply(Event::JmapProbed { url: SERVER.into(), probe });
}

fn config(s: &Session, id: &str) -> AccountConfig {
    s.account(id).map(|a| a.config.clone()).unwrap()
}

#[test]
fn a_jmap_server_is_asked_once_how_it_is_signed_in_to() {
    let mut s = session("probe-jmap");
    let asked = s.run("account.probeJmap", &json!({"url": format!(" {SERVER} ")})).unwrap();
    assert_eq!(asked, json!({"url": SERVER, "pending": true}));
    assert!(matches!(s.take_jobs().as_slice(), [Job::ProbeJmap { url }] if url == SERVER));
    // Asked again before it answers: still waiting, and not asked twice.
    assert_eq!(s.run("account.probeJmap", &json!({"url": SERVER})).unwrap()["pending"], json!(true));
    assert!(s.take_jobs().is_empty());
    let probe = JmapProbe { password: true, bearer: true, issuer: Some("https://sso.example.com".into()), ..JmapProbe::default() };
    s.apply(Event::JmapProbed { url: SERVER.into(), probe });
    let answer = s.run("account.probeJmap", &json!({"url": SERVER})).unwrap();
    assert_eq!(answer, json!({"url": SERVER, "password": true, "bearer": true, "issuer": "https://sso.example.com"}));
    assert!(s.take_jobs().is_empty());
    // `again`: asked again.
    assert_eq!(s.run("account.probeJmap", &json!({"url": SERVER, "again": true})).unwrap()["pending"], json!(true));
    assert!(matches!(s.take_jobs().as_slice(), [Job::ProbeJmap { .. }]));
    // An answer about a server that wasn't asked about isn't kept.
    s.apply(Event::JmapProbed { url: "https://other.example".into(), probe: JmapProbe::default() });
    assert!(!s.jmap_probes.contains_key("https://other.example"));
    // A few are kept, not every address ever typed.
    for n in 0..100 {
        s.run("account.probeJmap", &json!({"url": format!("https://jmap{n}.example.com")})).unwrap();
    }
    assert!(s.jmap_probes.len() <= 64, "{}", s.jmap_probes.len());
    for bad in [json!({}), json!({"url": ""}), json!({"url": "a b"}), json!({"url": "x\ny"}), json!({"url": "x".repeat(3000)}), json!({"url": 7})] {
        assert!(matches!(s.run("account.probeJmap", &bad), Err(CmdError::Params(_))), "{bad}");
    }
    done(s);
}

#[test]
fn an_account_signs_in_through_the_single_sign_on_its_server_names() {
    let mut s = session("single-sign-on");
    answered(&mut s);
    let add = |s: &mut Session, extra: Value| {
        let mut v = json!({"email": "ana@example.com", "jmap": SERVER, "oauth": "oidc"});
        if let (Some(o), Some(e)) = (v.as_object_mut(), extra.as_object()) {
            o.extend(e.clone());
        }
        s.run("account.add", &v)
    };
    // What the server was found to say: its service, the client id MailCraft was given for it,
    // and what it asks for. Signed in through the browser, with no password asked for.
    let id = add(&mut s, json!({})).unwrap()["account"].as_str().unwrap().to_string();
    let want = OidcSettings { issuer: "https://sso.example.com".into(), client_id: "mailcraft".into(), scope: "openid email".into() };
    let c = config(&s, &id);
    assert_eq!((c.oauth.as_deref(), c.oidc.as_ref()), (Some(OIDC), Some(&want)));
    assert!(matches!(s.take_jobs().as_slice(), [Job::SignIn { account }] if account.oidc.as_ref() == Some(&want)));
    let saved = s.store.as_ref().unwrap().load_accounts().unwrap();
    assert_eq!(saved.iter().find(|c| c.id == id).and_then(|c| c.oidc.clone()), Some(want), "kept for next time");
    s.run("account.remove", &json!({"account": id})).unwrap();

    // What it's given comes first. Another service's client id isn't this one's.
    let id = add(&mut s, json!({"issuer": " https://kc.example.org/realms/mail/ ", "scope": " openid   profile "})).unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    let want = OidcSettings { issuer: "https://kc.example.org/realms/mail".into(), client_id: String::new(), scope: "openid profile".into() };
    assert_eq!(config(&s, &id).oidc, Some(want));
    s.run("account.remove", &json!({"account": id})).unwrap();
    let id = add(&mut s, json!({"clientId": " mc-desktop "})).unwrap()["account"].as_str().unwrap().to_string();
    assert_eq!(config(&s, &id).oidc.map(|o| o.client_id), Some("mc-desktop".into()));
    s.run("account.remove", &json!({"account": id})).unwrap();
    // A test service on this computer, over plain http.
    for local in ["http://127.0.0.1:8080", "http://[::1]:9000/realms/mail"] {
        let id = add(&mut s, json!({"issuer": local})).unwrap()["account"].as_str().unwrap().to_string();
        assert_eq!(config(&s, &id).oidc.map(|o| o.issuer), Some(local.to_string()));
        s.run("account.remove", &json!({"account": id})).unwrap();
    }
    s.take_jobs();

    for (extra, why) in [
        (json!({"issuer": "http://sso.example.com"}), "isn't https"),
        (json!({"issuer": "http://localhost:8080"}), "isn't https"),
        (json!({"issuer": "https://sso.example.com/?realm=mail"}), "issuer"),
        (json!({"issuer": "https://me@sso.example.com"}), "issuer"),
        (json!({"issuer": "https://sso example.com"}), "issuer"),
        (json!({"issuer": "sso.example.com"}), "issuer"),
        (json!({"issuer": "https://"}), "issuer"),
        (json!({"clientId": "a\nb"}), "clientId"),
        (json!({"clientId": "c".repeat(300)}), "clientId"),
        (json!({"scope": "openid \"email\""}), "scope"),
        (json!({"jmap": "https://unknown.example/jmap"}), "`issuer` is required"),
        (json!({"jmap": null}), "`jmap` is required"),
    ] {
        match add(&mut s, extra.clone()) {
            Err(CmdError::Params(e)) => assert!(e.contains(why), "{extra}: {e}"),
            other => panic!("{extra}: {other:?}"),
        }
    }
    assert!(s.accounts.is_empty() && s.take_jobs().is_empty());
    // Not in a copy that can't sign in through the browser.
    s.oauth_available.clear();
    assert!(matches!(add(&mut s, json!({})), Err(CmdError::Params(e)) if e.contains("single sign-on isn't set up")));
    done(s);
}

#[test]
fn an_account_changes_how_it_signs_in() {
    let mut s = session("sign-on-change");
    answered(&mut s);
    let id = s.run("account.add", &json!({"email": "ana@example.com", "password": "pw", "jmap": SERVER})).unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    s.take_jobs();
    // To the server's single sign-on: through the browser, never with the password kept sent to
    // the service as a token.
    s.run("account.update", &json!({"account": id, "oauth": "oidc"})).unwrap();
    let c = config(&s, &id);
    assert_eq!((c.oauth.as_deref(), c.oidc.as_ref().map(|o| o.client_id.as_str())), (Some(OIDC), Some("mailcraft")));
    assert!(matches!(s.take_jobs().as_slice(), [Job::SignIn { account }] if account.oauth.as_deref() == Some(OIDC)));
    // Another client id: signed in again under it. Its name: nothing to sign in again for.
    s.run("account.update", &json!({"account": id, "clientId": "mc-desktop"})).unwrap();
    assert!(matches!(s.take_jobs().as_slice(), [Job::SignIn { account }] if account.oidc.as_ref().is_some_and(|o| o.client_id == "mc-desktop")));
    s.run("account.update", &json!({"account": id, "name": "Ana", "clientId": "mc-desktop"})).unwrap();
    assert!(s.take_jobs().is_empty());
    // Its refresh token is what's kept: a password isn't put in its place.
    assert!(matches!(s.run("account.setPassword", &json!({"account": id, "password": "pw2"})), Err(CmdError::Params(_))));
    for bad in [json!({"account": id, "jmap": null}), json!({"account": id, "oauth": "google"}), json!({"account": id, "oauth": null})] {
        assert!(matches!(s.run("account.update", &bad), Err(CmdError::Params(_))), "{bad}");
    }
    assert_eq!(config(&s, &id).oauth.as_deref(), Some(OIDC));
    // Back to a password: with the password, which takes the refresh token's place.
    s.run("account.update", &json!({"account": id, "oauth": null, "password": "pw3"})).unwrap();
    assert_eq!(config(&s, &id).oauth, None);
    assert!(matches!(s.take_jobs().as_slice(), [Job::Start { password: Some(p), .. }] if p.0 == "pw3"));

    // An IMAP account has no single sign-on of this kind; Google's stays Google's.
    let imap = s.run("account.add", &json!({"email": "bo@example.com", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    assert!(matches!(s.run("account.update", &json!({"account": imap, "oauth": "oidc"})), Err(CmdError::Params(_))));
    s.oauth_available.push("google".into());
    let google = s.run("account.add", &json!({"email": "cy@example.com", "oauth": "google"})).unwrap()["account"].as_str().unwrap().to_string();
    for change in [json!({"account": google, "oauth": null, "password": "pw"}), json!({"account": google, "oauth": "oidc"})] {
        assert!(matches!(s.run("account.update", &change), Err(CmdError::Params(e)) if e.contains("signs in with google")), "{change}");
    }
    done(s);
}
