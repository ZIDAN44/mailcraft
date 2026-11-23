//! A job whose thread can't start (the computer out of threads) is answered with its own
//! failure, as its thread would have: nothing waits for an answer that never comes (a message to
//! be signed before it's sent waited for good).

use mailcraft_model::Revocation;
use mailcraft_model::job::{PgpWork, Private, Secret};

use super::*;
use crate::worker::routing::NO_THREADS;

/// What the pool says for each job given while no thread can start.
fn answers(jobs: Vec<Job>) -> Vec<Event> {
    let mut pool = Pool::new(Store::memory(), Arc::new(MemoryCredentials::default()));
    NO_THREADS.set(true);
    for job in jobs {
        pool.submit(job);
    }
    NO_THREADS.set(false);
    std::iter::from_fn(|| pool.wait(Duration::from_millis(50))).collect()
}

fn account() -> AccountConfig {
    AccountConfig { id: "a1".into(), email: "ana@studio.example".into(), ..Default::default() }
}

#[test]
fn signing_decrypting_and_keys_are_answered() {
    let key = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid: 7 };
    let raw = Private(b"Subject: hi\r\n\r\nhello\r\n".to_vec());
    let pgp = |work| Job::Pgp { account: account(), work };
    let said = answers(vec![
        pgp(PgpWork::Protect { id: 5, raw: raw.clone(), sign: true, encrypt_to: Vec::new(), hide_subject: false, signer: None }),
        pgp(PgpWork::Decrypt { key: key.clone(), data: Vec::new(), inline: false, charset: String::new(), verify_with: Vec::new() }),
        pgp(PgpWork::Use { fingerprint: "00aa".into() }),
        Job::Protect { account: account(), id: 6, raw, sign: true, encrypt_to: Vec::new(), gcm: true, hide_subject: false, signer: None },
        Job::Decrypt { account: account(), key: key.clone(), cms: Vec::new() },
        Job::ImportIdentity { account: account(), data: Vec::new(), password: Secret(String::new()) },
        Job::FindHeldKeys { module: None, pin: None },
        Job::CheckRevocation { chain: vec![vec![1, 2, 3]] },
        Job::ForgetIdentity { account: account() },
    ]);
    let failed = |e: &Option<String>| e.as_deref().is_some_and(|e| e.contains("no thread"));
    let found = |what: &str, f: &dyn Fn(&Event) -> bool| assert_eq!(said.iter().filter(|e| f(e)).count(), 1, "{what}: {said:?}");
    found("OpenPGP signed", &|e| matches!(e, Event::Protected { id: 5, raw: None, error, .. } if failed(error)));
    found("OpenPGP decrypted", &|e| matches!(e, Event::PgpDecrypted { key: k, error, .. } if *k == key && failed(error)));
    found("OpenPGP keys", &|e| matches!(e, Event::PgpKeys { keys, error, .. } if keys.is_empty() && failed(error)));
    found("S/MIME signed", &|e| matches!(e, Event::Protected { id: 6, raw: None, error, .. } if failed(error)));
    found("S/MIME decrypted", &|e| matches!(e, Event::Decrypted { key: k, error, .. } if *k == key && failed(error)));
    found("certificate", &|e| matches!(e, Event::Identity { account, error, .. } if account == "a1" && failed(error)));
    found("held keys", &|e| matches!(e, Event::HeldKeys { error, .. } if failed(error)));
    found("revocation", &|e| matches!(e, Event::Revocation { cert, status: Revocation::Unknown { .. } } if *cert == [1, 2, 3]));
    assert_eq!(said.len(), 8, "forgetting says nothing: {said:?}");
}

#[test]
fn export_discovery_and_sign_in_are_answered() {
    let mut jobs = vec![
        Job::ExportPst { account: "a1".into(), path: "out.pst".into(), name: "Out".into(), follow_up: Default::default() },
        Job::Discover { email: "ana@studio.example".into(), rules: Default::default() },
    ];
    // Signing in needs a provider registered in this build.
    let oauth = AccountConfig { oauth: Some("microsoft".into()), graph: Some(String::new()), ..account() };
    let signs_in = crate::oauth::provider_for_account(&oauth).is_some();
    if signs_in {
        jobs.extend([Job::SignIn { account: oauth.clone() }, Job::AllowDirectory { account: oauth }]);
    }
    jobs.push(Job::DataFileHealth { account: "a1".into() });
    let said = answers(jobs);
    let failed = |e: &Option<String>| e.as_deref().is_some_and(|e| e.contains("no thread"));
    // The idle policy waits for each account it asked about.
    assert!(said.iter().any(|e| matches!(e, Event::DataFileHealth { account, health: None, error } if account == "a1" && failed(error))), "{said:?}");
    assert!(said.iter().any(|e| matches!(e, Event::Exported { account, done: true, error, .. } if account == "a1" && failed(error))), "{said:?}");
    assert!(said.iter().any(|e| matches!(e, Event::Discovered { email, config: None, .. } if email == "ana@studio.example")), "{said:?}");
    if signs_in {
        let needs = |e: &Event| matches!(e, Event::Status { account, status: SyncStatus::NeedsPassword { error } } if account == "a1" && error.contains("no thread"));
        assert!(said.iter().any(needs), "{said:?}");
        assert!(said.iter().any(|e| matches!(e, Event::DirectoryAllowed { account, error } if account == "a1" && failed(error))), "{said:?}");
    }
}
