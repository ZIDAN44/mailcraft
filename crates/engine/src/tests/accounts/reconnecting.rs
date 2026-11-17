//! An account changed in its settings: what it connects with changed, it starts again with it.

use super::*;

/// What an account signs in and connects with, changed, starts it again; what only shows it
/// doesn't. Its address, the sign-in name when it has no user name of its own, did nothing: the
/// account went on with the old one until MailCraft was started again.
#[test]
fn an_account_starts_again_when_what_it_connects_with_changes() {
    let mut s = Session::open(Some(temp("reconnecting")), mailcraft_model::dates::Clock::new(NOW, 0));
    let id = s.run("account.add", &json!({"email": "riley@studio.example", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    let mut c = s.account(&id).unwrap().config.clone();
    c.username = String::new();
    s.update_account(c, None).unwrap();
    s.take_jobs();
    let started = |s: &mut Session| s.take_jobs().iter().any(|j| matches!(j, Job::Start { account, .. } if account.id == id));
    let mut c = s.account(&id).unwrap().config.clone();
    c.email = "riley@studio.test".into();
    s.update_account(c, None).unwrap();
    assert!(started(&mut s), "the address it signs in with");
    let mut c = s.account(&id).unwrap().config.clone();
    c.oauth_user = Some("riley.r@studio.test".into());
    s.update_account(c, None).unwrap();
    assert!(started(&mut s), "who it signed in as");
    let mut c = s.account(&id).unwrap().config.clone();
    c.check_every = Some(15);
    s.update_account(c, None).unwrap();
    assert!(started(&mut s), "how often it looks");
    // What only shows the account doesn't.
    let mut c = s.account(&id).unwrap().config.clone();
    (c.name, c.color) = ("Riley R.".into(), 3);
    s.update_account(c, None).unwrap();
    assert!(!started(&mut s), "its name and colour");
}

/// An IMAP or POP account's address corrected in its settings, as Outlook lets it be: mistyped as
/// it was added, every message went from it and the server turned them away ("550 Invalid
/// Sender"), and only removing the account changed it. It connects again with it; an address
/// that isn't one, another account's, or one an account signs in as isn't taken.
#[test]
fn an_accounts_mistyped_address_is_corrected() {
    let mut s = Session::open(Some(temp("address-corrected")), mailcraft_model::dates::Clock::new(NOW, 0));
    let add = |s: &mut Session, v: serde_json::Value| s.run("account.add", &v).unwrap()["account"].as_str().unwrap().to_string();
    let id = add(&mut s, json!({"email": "monsur@shinetbd.example", "username": "monsur@shinestbd.example", "password": "pw", "pop": true}));
    let other = add(&mut s, json!({"email": "ana@studio.example", "password": "pw"}));
    s.take_jobs();
    s.run("account.update", &json!({"account": id, "email": " monsur@shinestbd.example "})).unwrap();
    let c = &s.account(&id).unwrap().config;
    assert_eq!((c.email.as_str(), c.username.as_str()), ("monsur@shinestbd.example", "monsur@shinestbd.example"));
    assert!(s.take_jobs().iter().any(|j| matches!(j, Job::Start { account, .. } if account.id == id && account.email == "monsur@shinestbd.example")));
    // Not an address, or another account's: refused, and nothing changed.
    for bad in ["monsur", "", "ana@studio.example", "ANA@studio.example"] {
        assert!(s.run("account.update", &json!({"account": id, "email": bad})).is_err(), "{bad:?}");
    }
    assert_eq!(s.account(&id).unwrap().config.email, "monsur@shinestbd.example");
    // Its own address again (in another case), and the other settings alone: nothing to change.
    s.run("account.update", &json!({"account": id, "email": "Monsur@ShinestBD.example", "name": "MA Monsur"})).unwrap();
    assert_eq!(s.account(&id).unwrap().config.email, "monsur@shinestbd.example");
    // An account that signs in as its address keeps it.
    let mut c = s.account(&other).unwrap().config.clone();
    c.oauth = Some("google".into());
    s.update_account(c, None).unwrap();
    assert!(s.run("account.update", &json!({"account": other, "email": "ana2@studio.example"})).is_err());
}
