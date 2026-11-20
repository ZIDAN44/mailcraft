//! Older Exchange servers: what their schema refuses, MailCraft doesn't ask of them in a request
//! with anything else.

use super::*;

/// The account's special folders, and, as a later Exchange has them, an Archive folder and the
/// two kept out of the list, when `newer` says so.
fn mailbox(newer: bool) -> Data {
    let mut d = Data::default();
    for (name, known) in
        [("Inbox", "inbox"), ("Drafts", "drafts"), ("Sent Items", "sentitems"), ("Deleted Items", "deleteditems"), ("Junk Email", "junkemail")]
    {
        d.folder(name, None, Some(known), "IPF.Note");
    }
    d.folder("Outbox", None, Some("outbox"), "IPF.Note");
    if newer {
        d.folder("Archive", None, Some("archive"), "IPF.Note");
        d.folder("Conversation History", None, Some("conversationhistory"), "IPF.Note");
        d.folder("Sync Issues", None, Some("syncissues"), "IPF.Note");
    }
    d
}

fn roles(d: Data) -> Result<Vec<(String, mailcraft_model::FolderRole)>, crate::NetError> {
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example")?;
    Ok(c.mailboxes()?.into_iter().map(|m| (m.name, m.role)).collect())
}

/// Issue #149: Exchange 2010 SP3 refused the first sync's folder listing as a whole, its schema
/// not knowing the Archive folder's distinguished id, nor Conversation History's or Sync
/// Issues' (2013 brought them). Its folders come with their roles all the same, and asking for
/// them again doesn't ask for those; a later server's Archive folder still has its role, and its
/// Conversation History and Sync Issues stay out of the list.
#[test]
fn exchange_2010_lists_its_folders_without_the_ids_2013_brought() {
    use mailcraft_model::FolderRole;
    let mut d = mailbox(false);
    d.exchange_2010 = true;
    let found = roles(d).unwrap();
    let role = |name: &str, f: &[(String, FolderRole)]| f.iter().find(|(n, _)| n == name).map(|(_, r)| *r);
    assert_eq!(role("Inbox", &found), Some(FolderRole::Inbox), "{found:?}");
    assert_eq!(role("Sent Items", &found), Some(FolderRole::Sent));
    assert_eq!(role("Junk Email", &found), Some(FolderRole::Junk));
    assert!(!found.iter().any(|(_, r)| *r == FolderRole::Archive));

    let found = roles(mailbox(true)).unwrap();
    assert_eq!(role("Archive", &found), Some(FolderRole::Archive), "{found:?}");
    assert_eq!(role("Inbox", &found), Some(FolderRole::Inbox));
    assert!(!found.iter().any(|(n, _)| n == "Conversation History" || n == "Sync Issues"), "{found:?}");

    // Listed again on the same connection: the server's refusals aren't asked for again.
    let mut d = mailbox(false);
    d.exchange_2010 = true;
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    c.mailboxes().unwrap();
    let refused = server.data.lock().unwrap().schema_refusals;
    assert_eq!(refused, 4, "together, then each of the three alone");
    c.mailboxes().unwrap();
    assert_eq!(server.data.lock().unwrap().schema_refusals, refused, "not asked again");
}

/// Issue #197: Exchange 2010 hasn't `MarkAllItemsAsRead` (2013 brought it): a folder marked read
/// has its unread mail found and marked a page at a time, all of it, mail never synced too.
#[test]
fn exchange_2010_marks_a_folder_read_without_mark_all() {
    let mut d = mailbox(false);
    d.exchange_2010 = true;
    let inbox = d.by_name("Inbox").unwrap().id.clone();
    let server = serve(d);
    let store = crate::fake::store("ews-2010-read");
    crate::fakes::fake_dav::looked_for_today(&store, "a1");
    let mut pool = crate::worker::Pool::new(store, Arc::new(crate::worker::MemoryCredentials::default()));
    pool.submit(Job::Start { account: account("a1", &server.url), password: Some(Secret(PASSWORD.into())) });
    until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    let ids: Vec<String> = {
        let mut d = server.data.lock().unwrap();
        (0..3).map(|i| d.deliver(&inbox, &mail(&format!("Unread {i}"), &format!("u{i}")), false, "2020-01-01T00:00:00Z")).collect()
    };
    pool.submit(Job::Apply { account: "a1".into(), op: Op::MarkFolderRead { path: "INBOX".into() } });
    eventually("all read", || {
        let d = server.data.lock().unwrap();
        ids.iter().all(|id| d.items.iter().any(|i| &i.id == id && i.read))
    });
    let calls = server.data.lock().unwrap().calls.clone();
    assert!(calls.iter().any(|c| c == "MarkAllItemsAsRead"), "asked at once first: {calls:?}");
}
