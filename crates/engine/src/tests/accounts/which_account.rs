//! Which account a command acts on: the one it names, else the one on screen.

use super::*;

/// A command that names an account that isn't one is refused for it, alike for each (some took
/// any name and went on with it, or said "`account` is required" for the account on screen
/// missing); with none named, the one on screen.
#[test]
fn an_account_named_must_be_one() {
    let mut s = sample_session("which-account");
    let path = s.store.as_ref().unwrap().root().join("out.pst");
    let path = path.to_string_lossy();
    for (id, params) in [
        ("dataFile.health", json!({"account": "nobody"})),
        ("dataFile.compact", json!({"account": "nobody"})),
        ("account.exportPst", json!({"account": "nobody", "path": path})),
        ("event.create", json!({"account": "nobody", "summary": "x", "start": "2026-11-20T10:00"})),
    ] {
        let r = s.run(id, &params);
        assert!(matches!(&r, Err(CmdError::Params(m)) if m == "no account `nobody`"), "{id}: {r:?}");
    }
    // None named: the one on screen.
    let made = s.run("event.create", &json!({"summary": "y", "start": "2026-11-20T10:00"})).unwrap();
    assert_eq!(made["account"], "sample", "{made}");
    drop(s);
    // No account at all: said alike (each copy had words of its own, and two called it a bad
    // parameter).
    let mut s = Session::open(Some(temp("which-account-none")), mailcraft_model::dates::Clock::new(NOW, 0));
    for (id, params) in [
        ("dataFile.health", json!({})),
        ("dataFile.compact", json!({})),
        ("account.exportPst", json!({"path": path})),
        ("event.create", json!({"summary": "x", "start": "2026-11-20T10:00"})),
    ] {
        let r = s.run(id, &params);
        assert!(matches!(&r, Err(CmdError::Disabled(m)) if m == "add an account first"), "{id}: {r:?}");
    }
}
