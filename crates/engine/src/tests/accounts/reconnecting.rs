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
