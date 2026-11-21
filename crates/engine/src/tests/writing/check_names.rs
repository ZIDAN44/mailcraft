//! Issue #211: Check Names on the message being written: a name typed that's one person is put
//! in, several are offered to pick from, none is said; one the organisation's directory is still
//! asked about is said too, and found once it answers.

use super::*;

fn writing(s: &mut Session, to: &str) {
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"to": to})).unwrap();
}

fn to(s: &Session) -> String {
    s.compose.as_ref().map(|c| c.to.clone()).unwrap_or_default()
}

#[test]
fn names_are_put_in_offered_or_said_missing() {
    let mut s = sample_session("check-names");
    writing(&mut s, "zoe; ops@studio.example; nobody-by-that-name");
    let r = s.run("compose.checkNames", &json!({})).unwrap();
    assert_eq!(r["resolved"], 1, "{r}");
    assert_eq!(to(&s), "Zoe Park <zoe@studio.example>; ops@studio.example; nobody-by-that-name; ");
    assert_eq!(r["none"], json!([{"field": "to", "typed": "nobody-by-that-name"}]));
    // Several people: offered, the box as it was; the one picked goes in its place.
    s.run("compose.update", &json!({"to": "", "cc": "studio"})).unwrap();
    let r = s.run("compose.checkNames", &json!({})).unwrap();
    let several = r["several"].as_array().expect("several");
    assert_eq!((several.len(), several[0]["field"].as_str(), several[0]["typed"].as_str()), (1, Some("cc"), Some("studio")), "{r}");
    let people = several[0]["people"].as_array().unwrap();
    assert!(people.len() > 1 && people.iter().all(|p| p["email"].as_str().is_some_and(|e| e.contains('@'))), "{people:?}");
    assert_eq!(s.compose.as_ref().unwrap().cc, "studio", "left as typed");
    let one = &people[1];
    s.run("compose.pickName", &json!({"field": "cc", "typed": "studio", "email": one["email"], "name": one["name"]})).unwrap();
    let cc = s.compose.as_ref().unwrap().cc.clone();
    assert!(cc.contains(one["email"].as_str().unwrap()) && !cc.contains("studio;"), "{cc}");
    // Gone from the box meanwhile, a field that isn't one, or no address: refused.
    assert!(s.run("compose.pickName", &json!({"field": "cc", "typed": "studio", "email": "a@b.example"})).is_err());
    assert!(s.run("compose.pickName", &json!({"field": "from", "typed": "x", "email": "a@b.example"})).is_err());
    assert!(s.run("compose.pickName", &json!({"field": "to", "typed": "x", "email": "not an address"})).is_err());
}

#[test]
fn a_directory_still_asked_is_said_then_found() {
    let mut s = sample_session("check-names-directory");
    let p = json!({"email": "riley@contoso.example", "password": "pw", "username": "CONTOSO\\riley", "ews": "mail.contoso.example"});
    let account = s.run("account.add", &p).unwrap()["account"].as_str().unwrap().to_string();
    s.take_jobs();
    // Bo: no one known here. Park: Zoe Park alone, but the directory may know others by it, so
    // she waits for it too. Zoe: her address's own name, put in at once.
    writing(&mut s, "Bo; Park; zoe");
    let r = s.run("compose.checkNames", &json!({})).unwrap();
    assert_eq!(r["pending"], json!([{"field": "to", "typed": "Bo"}, {"field": "to", "typed": "Park"}]), "{r}");
    let asked: Vec<String> =
        s.take_jobs().into_iter().filter_map(|j| if let Job::SearchDirectory { query, .. } = j { Some(query) } else { None }).collect();
    assert!(asked.contains(&"bo".to_string()), "asked, short as it is: {asked:?}");
    assert_eq!(to(&s), "Bo; Park; Zoe Park <zoe@studio.example>; ");
    // Asked again before it answers: not asked twice.
    s.run("compose.checkNames", &json!({})).unwrap();
    assert!(!s.take_jobs().iter().any(|j| matches!(j, Job::SearchDirectory { .. })));
    let people = vec![mailcraft_model::DirectoryPerson { name: "Bo Ek".into(), email: "bo.ek@contoso.example".into(), ..Default::default() }];
    s.apply(Event::Directory { account: account.clone(), query: "bo".into(), people, error: None, consent: false });
    s.apply(Event::Directory { account, query: "park".into(), people: Vec::new(), error: None, consent: false });
    let r = s.run("compose.checkNames", &json!({})).unwrap();
    assert_eq!((r["resolved"].as_u64(), r["pending"].as_array().map(Vec::len)), (Some(2), Some(0)), "{r}");
    assert!(!s.take_jobs().iter().any(|j| matches!(j, Job::SearchDirectory { .. })), "answered lately: not asked again");
    assert_eq!(to(&s), "Bo Ek <bo.ek@contoso.example>; Zoe Park <zoe@studio.example>; Zoe Park <zoe@studio.example>; ");
}

#[test]
fn nothing_written_nothing_to_check() {
    let mut s = sample_session("check-names-none");
    assert!(s.run("compose.checkNames", &json!({})).is_err(), "no message being written");
    writing(&mut s, "");
    let r = s.run("compose.checkNames", &json!({})).unwrap();
    assert_eq!((r["resolved"].as_u64(), r["several"].as_array().map(Vec::len)), (Some(0), Some(0)));
}
