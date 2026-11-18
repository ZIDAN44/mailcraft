//! The mailbox's size and quota, as Mailbox Settings shows them.

use super::*;

/// Issue #143: Mailbox Settings said Exchange "doesn't report how much space is used", where
/// Outlook shows the mailbox's size and its limit. Its size is every folder's added up (more
/// folders than come in one page), its limit where it can no longer send, else receive, else
/// where it's warned; with no limit, its size alone; a server that says neither, nothing.
#[test]
fn the_mailbox_says_its_size_and_quota() {
    let mut d = Data::default();
    let inbox = d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let raw = |n: usize| format!("Subject: s{n}\r\nMessage-ID: <m{n}@x>\r\n\r\n{}\r\n", "x".repeat(n)).into_bytes();
    let mut total = 0;
    for n in 1..=5 {
        let f = d.folder(&format!("Project {n}"), Some(&inbox), None, "IPF.Note");
        d.deliver(&f, &raw(n * 1000), true, "2026-11-01T10:00:00Z");
        total += raw(n * 1000).len() as u64;
    }
    d.deliver(&inbox, &raw(700), false, "2026-11-01T10:00:00Z");
    total += raw(700).len() as u64;
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    assert_eq!(c.quota().unwrap(), None, "said of neither");
    for (limits, limit) in [([2048, 4096, 1024], 2048), ([0, 4096, 1024], 4096), ([0, 0, 1024], 1024), ([0, 0, 0], 0)] {
        server.data.lock().unwrap().quota_kb = Some(limits);
        assert_eq!(c.quota().unwrap(), Some((total, limit * 1024)), "{limits:?}");
    }
}

/// The worker says it with the mailbox's full sync, as it says an IMAP or JMAP account's: it
/// said nothing of an Exchange account's.
#[test]
fn an_exchange_accounts_sync_says_its_quota() {
    let mut d = Data::default();
    let inbox = d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let raw = b"Subject: hi
Message-ID: <q1@x>

hello
"
    .to_vec();
    d.deliver(&inbox, &raw, false, "2026-11-01T10:00:00Z");
    d.quota_kb = Some([2048, 0, 0]);
    let server = serve(d);
    let store = Store::open(crate::data_files::scratch::scratch("ews-quota")).unwrap();
    crate::fakes::fake_dav::looked_for_today(&store, "a1");
    let mut pool = crate::worker::Pool::new(store, std::sync::Arc::new(crate::worker::MemoryCredentials::default()));
    pool.submit(Job::Start { account: account("a1", &server.url), password: Some(Secret(PASSWORD.into())) });
    let used = raw.len() as u64;
    until(&pool, "the quota", |e| matches!(e, Event::Quota { used: u, limit: 2_097_152, .. } if *u == used));
}
