//! Issue #211 (Check Names) through a real Exchange directory on this computer (never a real
//! account), run by hand:
//!
//! ```text
//! MAILCRAFT_REAL_NAMES="http://127.0.0.1:18080/EWS/Exchange.asmx user@ews.test password" \
//!   cargo test -p mailcraft-cli real_names -- --ignored --nocapture
//! ```
//!
//! A server whose directory has Olga Rand alone by "olga", several people by "a" and no one by
//! "nobodyzz" (Gromox's, here). The whole app's way: the engine's commands, the worker asking the
//! server, the names checked again once it answers.

use super::*;

#[test]
#[ignore = "needs an Exchange server in MAILCRAFT_REAL_NAMES"]
fn real_names_are_checked_in_the_directory() {
    let Some(v) = std::env::var("MAILCRAFT_REAL_NAMES").ok() else {
        eprintln!("MAILCRAFT_REAL_NAMES not set: skipped");
        return;
    };
    let p: Vec<&str> = v.split_whitespace().collect();
    let [url, user, password] = p[..] else { panic!("MAILCRAFT_REAL_NAMES: url user password") };
    let dir = std::env::temp_dir().join(format!("mailcraft-real-names-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open(&dir).unwrap();
    let mut s = Session::open(Some(store.clone()), clock());
    let id = s.run("account.add", &json!({"email": user, "username": user, "ews": url, "deferSignIn": true})).unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    let config = s.account(&id).unwrap().config.clone();
    let mut pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    s.take_jobs();
    pool.submit(Job::Start { account: config, password: Some(Secret(password.into())) });
    drive(&mut s, &mut pool, Duration::from_secs(90), |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }).then_some(Ok(())))
        .unwrap();

    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"to": "olga; a; nobodyzz", "cc": "ews"})).unwrap();
    let r = s.run("compose.checkNames", &json!({})).unwrap();
    eprintln!("first: {r}");
    let pending: Vec<&str> = r["pending"].as_array().unwrap().iter().filter_map(|x| x["typed"].as_str()).collect();
    assert_eq!(pending, ["olga", "a", "nobodyzz", "ews"], "every one asked of the directory");
    let mut answered = std::collections::HashSet::new();
    drive(&mut s, &mut pool, Duration::from_secs(60), |e| {
        if let Event::Directory { query, error, people, .. } = e {
            eprintln!("directory {query:?}: {} {error:?}", people.len());
            answered.insert(query.clone());
        }
        (answered.len() >= 4).then_some(Ok(()))
    })
    .unwrap();
    let r = s.run("compose.checkNames", &json!({})).unwrap();
    eprintln!("again: {r}");
    let c = s.compose.as_ref().unwrap();
    assert!(c.to.starts_with("Olga Rand <org204@ews.test>; a; nobodyzz"), "{}", c.to);
    assert_eq!(r["none"], json!([{"field": "to", "typed": "nobodyzz"}]));
    assert_eq!(r["pending"], json!([]));
    let several = r["several"].as_array().unwrap();
    let emails = |i: usize| -> Vec<String> {
        several[i]["people"].as_array().unwrap().iter().filter_map(|p| p["email"].as_str().map(str::to_string)).collect()
    };
    assert_eq!(several.len(), 2, "{r}");
    assert!(emails(0).len() >= 3 && emails(0).contains(&"org204@ews.test".to_string()), "{:?}", emails(0));
    assert!(emails(1).iter().all(|e| e.ends_with("@ews.test")), "{:?}", emails(1));
    // One picked: put in where "a" was; asked again, nothing left of it.
    let ana = several[0]["people"].as_array().unwrap().iter().find(|p| p["email"] == "ana@ews.test").cloned().unwrap();
    s.run("compose.pickName", &json!({"field": "to", "typed": "a", "email": ana["email"], "name": ana["name"]})).unwrap();
    let r = s.run("compose.checkNames", &json!({})).unwrap();
    let c = s.compose.as_ref().unwrap();
    assert!(c.to.contains("ana@ews.test") && !c.to.contains("; a;"), "{}", c.to);
    assert_eq!(r["several"].as_array().map(Vec::len), Some(1), "only Cc's left: {r}");
    pool.submit(Job::Remove { account: id });
    drop(pool);
    let _ = std::fs::remove_dir_all(&dir);
}
