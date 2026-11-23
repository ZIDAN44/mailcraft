//! Issue #245: what an organisation's profile says, and what applies to whom.

use serde_json::json;

use super::*;

fn org(v: Value) -> Organisation {
    Organisation::from_sources(vec![("test".into(), v, true)])
}

#[test]
fn a_profile_reads_what_it_can_and_says_what_it_cant() {
    let o = org(json!({
        "organisation": "Contoso",
        "settings": {"accent": "#0055aa", "autoPictures": {"value": false, "locked": true}, "x": {"value": 1, "locked": "yes"}},
        "theme": {"value": "Dark", "locked": true},
        "updates": "sometimes",
        "requireEncryption": true,
        "certificateExceptions": "no",
        "accounts": {
            "servers": {
                "contoso.example": {"imap": "mail.contoso.example", "smtp": {"host": "smtp.contoso.example", "port": 587, "security": "starttls"},
                                    "caldav": "http://dav.contoso.example/", "pop": "pop:0"},
                "not a domain": {}
            },
            "microsoft": {"clientId": "1f1e-not-hex!"},
            "lookups": {"thunderbird": false, "carrier": true}
        },
        "signature": {"html": "<p>{name}</p>"},
        "colour": "teal",
        "domains": ["a.example"]
    }));
    let t = &o.top;
    assert_eq!(t.organisation.as_deref(), Some("Contoso"));
    assert_eq!(t.settings["accent"], Given { value: json!("#0055aa"), locked: false });
    assert!(t.settings["autoPictures"].locked);
    assert!(!t.settings["x"].locked, "a lock that isn't true or false is a default");
    assert_eq!(t.theme, Some(Given { value: json!("dark"), locked: true }));
    assert_eq!(t.updates, None);
    assert_eq!((t.require_encryption, t.certificate_exceptions), (Some(true), None));
    let s = &t.servers["contoso.example"];
    assert_eq!(s.imap, Some(Server { host: "mail.contoso.example".into(), port: 993, security: "tls".into() }));
    assert_eq!(s.smtp, Some(Server { host: "smtp.contoso.example".into(), port: 587, security: "starttls".into() }));
    assert_eq!((s.caldav.as_ref(), s.pop.as_ref()), (None, None), "http elsewhere than here, and port 0, aren't taken");
    assert_eq!(t.servers.len(), 1);
    assert_eq!(t.microsoft, None);
    assert_eq!(t.lookups, Some(Lookups { thunderbird: false, ..Lookups::default() }));
    let sig = t.signature.as_ref().unwrap();
    assert_eq!((sig.name.as_str(), sig.locked, sig.new_messages, sig.replies), ("Organisation", false, true, true));
    let said = o.ignored.join("\n");
    for bit in ["updates", "certificateExceptions", "caldav", "pop", "not a domain", "microsoft", "carrier", "colour", "domains", "x.locked"] {
        assert!(said.contains(bit), "{bit} isn't said in:\n{said}");
    }
}

#[test]
fn what_isnt_a_profile_is_said_and_nothing_else() {
    for v in [json!([1, 2]), json!("text"), json!(null)] {
        let o = Organisation::read(&v);
        assert_eq!(o.top, Profile::default());
        assert_eq!(o.ignored.len(), 1);
    }
    // Deep and wide: kept to what's read, never more.
    let mut deep = json!({});
    for _ in 0..500 {
        deep = json!({"settings": deep});
    }
    let o = org(deep);
    assert!(o.ignored.len() <= 100);
    let many: serde_json::Map<String, Value> = (0..2000).map(|i| (format!("k{i}"), json!(i))).collect();
    let o = org(json!({"settings": many, "profiles": (0..200).map(|i| json!({"domains": [format!("d{i}.example")]})).collect::<Vec<_>>()}));
    assert_eq!((o.top.settings.len(), o.profiles.len()), (512, 64));
}

#[test]
fn a_later_source_wins_and_a_portable_ones_locks_dont_count() {
    let portable = json!({"organisation": "Ours", "settings": {"accent": {"value": "#111111", "locked": true}, "compact": true},
                          "requireEncryption": true, "theme": {"value": "dark", "locked": true}});
    let system = json!({"settings": {"compact": false}, "updates": {"value": "off", "locked": true}});
    let o = Organisation::from_sources(vec![("portable".into(), portable, false), ("system".into(), system, true)]);
    assert_eq!(o.sources, ["portable", "system"]);
    let t = &o.top;
    assert_eq!(t.settings["accent"], Given { value: json!("#111111"), locked: false });
    assert_eq!(t.settings["compact"], Given { value: json!(false), locked: false });
    assert_eq!(t.require_encryption, None);
    assert_eq!(t.theme, Some(Given { value: json!("dark"), locked: false }));
    assert_eq!(t.updates, Some((Updates::Off, true)));
    assert!(!Organisation::default().sources.iter().any(|_| true) && Organisation::default().is_empty());
}

#[test]
fn several_institutions_in_one_file() {
    let o = org(json!({
        "organisation": "City IT",
        "settings": {"accent": "#222222", "compact": {"value": true, "locked": true}},
        "updates": {"value": "stable", "locked": false},
        "accounts": {"onlyOrganisationDomains": true, "domains": ["city.example"],
                     "servers": {"city.example": {"imap": "imap.city.example"}}},
        "signature": {"text": "City"},
        "profiles": [
            {"domains": ["school.example"], "default": true, "organisation": "School", "settings": {"accent": "#333333"},
             "signature": {"text": "School"}, "disclaimer": {"text": "Confidential"},
             "accounts": {"servers": {"school.example": {"imap": "imap.school.example"}}}},
            {"domains": ["clinic.example"], "organisation": "Clinic", "requireEncryption": true, "updates": "off",
             "settings": {"autoPictures": {"value": false, "locked": true}, "compact": {"value": false, "locked": true}},
             "accounts": {"lookups": {"mx": false}}},
            {"domains": ["other.example"], "default": true}
        ]
    }));
    assert!(o.ignored.iter().any(|i| i.contains("more than one is the default")), "{:?}", o.ignored);
    assert_eq!(o.profiles.iter().filter(|p| p.default).count(), 1);
    // Each account follows its domain's profile, else the top.
    assert_eq!(o.signature_for("a@school.example").unwrap().text, "School");
    assert_eq!(o.signature_for("a@city.example").unwrap().text, "City");
    assert_eq!(o.signature_for("a@clinic.example").unwrap().text, "City");
    assert_eq!(o.disclaimer_for("a@SCHOOL.example").unwrap().text, "Confidential");
    assert!(o.disclaimer_for("a@clinic.example").is_none());
    assert_eq!(o.servers_for("a@school.example").and_then(|s| s.imap.as_ref()).unwrap().host, "imap.school.example");
    assert_eq!(o.servers_for("a@city.example").and_then(|s| s.imap.as_ref()).unwrap().host, "imap.city.example");
    assert!(o.servers_for("a@clinic.example").is_none());

    // Someone at the school alone: the default profile's look, no clinic rules.
    let school = o.effective(&["school.example".into()]);
    assert_eq!(school.organisation.as_deref(), Some("School"));
    assert_eq!(school.settings["accent"].value, json!("#333333"));
    assert_eq!(school.settings["compact"], Given { value: json!(true), locked: true });
    assert!(!school.require_encryption && school.lookups.mx && !school.locked("autoPictures"));
    assert_eq!(school.updates, Some((Updates::Stable, false)));
    assert!(school.allows_address("x@city.example") && school.allows_address("x@clinic.example"));
    assert!(!school.allows_address("x@gmail.example"));

    // With a clinic account too: its locks and the strictest policies, the window's own locks
    // kept (compact stays as the top locked it).
    let both = o.effective(&["school.example".into(), "clinic.example".into()]);
    assert!(both.require_encryption && !both.lookups.mx && both.locked("autoPictures"));
    assert_eq!(both.settings["compact"].value, json!(true));
    assert_eq!(both.updates, Some((Updates::Off, false)));
    assert_eq!(both.organisation.as_deref(), Some("School"));
}

#[test]
fn no_profile_changes_nothing() {
    let e = Organisation::default().effective(&["anything.example".into()]);
    assert_eq!(e, Effective { lookups: Lookups::default(), ..Effective::default() });
    assert!(e.allows_address("a@b.example") && !e.require_encryption && !e.no_certificate_exceptions);
}

/// A server named with its port and no security is taken as that port is used: submission's
/// 587 and IMAP's and POP's own ports with STARTTLS, any other with TLS.
#[test]
fn a_ports_security_is_its_own() {
    let o = org(json!({"accounts": {"servers": {"fabrikam.example": {
        "imap": "imap.fabrikam.example:143", "smtp": "smtp.fabrikam.example:587", "pop": "pop.fabrikam.example:2995"}}}}));
    let s = &o.top.servers["fabrikam.example"];
    let security = |x: &Option<Server>| x.as_ref().map(|x| (x.port, x.security.clone()));
    assert_eq!(security(&s.imap), Some((143, "starttls".to_string())));
    assert_eq!(security(&s.smtp), Some((587, "starttls".to_string())));
    assert_eq!(security(&s.pop), Some((2995, "tls".to_string())));
}

/// The organisation's Microsoft registrations, each for the accounts of its domains: a domain's
/// profile's for its own; the top's for the others it names, and its directory's domain.
#[test]
fn microsoft_registrations_are_for_the_organisations_domains() {
    let o = org(json!({
        "accounts": {"domains": ["contoso.example"], "microsoft": {"clientId": "0f1e", "tenant": "Contoso.onmicrosoft.example"}},
        "profiles": [
            {"domains": ["fabrikam.example"], "accounts": {"microsoft": {"clientId": "aa11"}}},
            {"domains": ["tailspin.example"]}
        ]
    }));
    let apps: Vec<(Vec<String>, String)> = o.microsoft_apps().into_iter().map(|(d, a)| (d, a.client_id)).collect();
    assert_eq!(
        apps,
        [
            (vec!["fabrikam.example".to_string()], "aa11".to_string()),
            (vec!["contoso.example".into(), "contoso.onmicrosoft.example".into(), "tailspin.example".into()], "0f1e".into())
        ]
    );
    assert!(Organisation::default().microsoft_apps().is_empty());
}

/// A macOS configuration profile with both named values and the profile's own keys is two
/// sources from one file: named once where the profile says where it came from (found on a
/// real Mac, its log naming the computer's file twice).
#[test]
fn one_place_is_named_once() {
    let o = Organisation::from_sources(vec![
        ("managed".into(), json!({"organisation": "Contoso"}), true),
        ("managed".into(), json!({"requireEncryption": true}), true),
        ("file".into(), json!({}), true),
    ]);
    assert_eq!(o.sources, ["managed", "file"]);
    assert_eq!(o.top.require_encryption, Some(true));
}
