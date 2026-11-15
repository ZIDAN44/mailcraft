//! Where requests go: a folder kept in a shared mailbox or among the public folders is asked for
//! there, and nothing the server didn't find is asked again in every other mailbox.

use super::*;

fn ops(server: &Server, name: &str) -> usize {
    server.data.lock().unwrap().calls.iter().filter(|c| *c == name).count()
}

fn forget(server: &Server) {
    let mut d = server.data.lock().unwrap();
    d.calls.clear();
    d.anchors.clear();
}

/// A message the server no longer has was asked for again in each shared mailbox and public
/// folder mailbox, and a move, a deletion or a new message with it: each is sent once now, to
/// where its folder is kept. What lives in a shared mailbox or a public folder is still found.
#[test]
fn requests_go_where_their_folder_is_and_nowhere_else() {
    let mut d =
        Data { pf_mailbox: Some("PFContent@contoso.example".into()), pf_hierarchy: Some("PFHierarchy@contoso.example".into()), ..Default::default() };
    let inbox = d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.folder("Projects", None, None, "IPF.Note");
    let own = mail("Own", "o1");
    let gone = d.deliver(&inbox, &own, false, "2026-10-19T08:00:00Z");
    let root = d.folder("IPM_SUBTREE", None, Some("publicfoldersroot"), "IPF.Note");
    let team = d.folder("Team", Some(&root), None, "IPF.Note");
    let public = mail("Holiday hours", "p1");
    d.deliver(&team, &public, false, "2026-10-19T09:00:00Z");
    let shared = |address: &str, name: &str| SharedBox { address: address.into(), name: name.into(), automap: true, legacy: false, granted: true };
    let sales = d.shared_mailbox(shared("sales@contoso.example", "Sales"));
    d.shared_mailbox(shared("support@contoso.example", "Support"));
    let order = mail("Order 17", "s1");
    d.deliver(&sales, &order, false, "2026-10-19T10:00:00Z");
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    let store = crate::fake::store("ews-elsewhere");
    let mut st = crate::jmap::State::default();
    // Listed again, the folders the mailbox hasn't (an Archive) are asked for once, as the
    // first time, when there were no shared mailboxes or public folders known to ask.
    forget(&server);
    let folders = crate::ews::list_folders(&c, &mut st, "a1").unwrap();
    let first = server.data.lock().unwrap().calls.len();
    forget(&server);
    crate::ews::list_folders(&c, &mut st, "a1").unwrap();
    assert_eq!(server.data.lock().unwrap().calls.len(), first, "listing the folders again asks as much");
    let team_path = folders.iter().find(|f| f.path.ends_with("/Team")).map(|f| f.path.clone()).unwrap();
    let opts = crate::sync::SyncOptions::default();
    // Found where they are, their sources whole.
    for (path, raw) in [("INBOX", &own), (team_path.as_str(), &public), ("Sales/Inbox", &order)] {
        forget(&server);
        crate::ews::sync_folder(&c, &mut st, &store, "a1", path, &opts).unwrap();
        let fs = store.load_folder("a1", path).unwrap();
        assert_eq!(fs.messages.len(), 1, "{path}");
        store.remove_body(&fs.messages[0].key);
        assert_eq!(&crate::ews::fetch_body(&c, &st, &store, &fs.messages[0].key, 0).unwrap().0, raw, "{path}");
        let (calls, anchors) = {
            let d = server.data.lock().unwrap();
            (d.calls.clone(), d.anchors.clone())
        };
        let wanted = match path {
            "INBOX" => None,
            "Sales/Inbox" => Some("sales@contoso.example"),
            _ => Some("PFContent@contoso.example"),
        };
        match wanted {
            Some(at) => assert!(anchors.len() == calls.len() && anchors.iter().all(|a| a == at), "{path}: {calls:?} {anchors:?}"),
            None => assert!(anchors.is_empty(), "{path}: {anchors:?}"),
        }
    }
    // Gone from the server since: asked for, moved and deleted once each.
    let key = store.load_folder("a1", "INBOX").unwrap().messages[0].key.clone();
    server.data.lock().unwrap().items.retain(|i| i.id != gone);
    forget(&server);
    let asked = c.call(&format!(
        "<m:GetItem><m:ItemShape><t:BaseShape>IdOnly</t:BaseShape></m:ItemShape><m:ItemIds><t:ItemId Id=\"{gone}\"/></m:ItemIds></m:GetItem>"
    ));
    assert!(crate::ews::messages(&asked.unwrap()).iter().all(|m| m.attr("ResponseClass") == Some("Error")));
    assert_eq!(ops(&server, "GetItem"), 1, "a message not found is asked for once");
    let moved = crate::ews::apply_remote(&c, &st, &Op::Move { keys: vec![key.clone()], to: "Projects".into() });
    assert!(moved.is_err());
    assert_eq!(ops(&server, "MoveItem"), 1, "a move isn't sent again elsewhere");
    let deleted = crate::ews::apply_remote(&c, &st, &Op::Expunge { keys: vec![key] });
    assert!(deleted.is_err());
    assert_eq!(ops(&server, "DeleteItem"), 1, "a deletion isn't sent again elsewhere");
    // A message put in a public folder goes there the first time.
    forget(&server);
    crate::ews::append(&c, &st, &team_path, &mail("Posted", "n1")).unwrap();
    assert_eq!(ops(&server, "CreateItem"), 1, "a new message isn't sent again elsewhere");
    assert_eq!(server.data.lock().unwrap().items.iter().filter(|i| i.folder == team).count(), 2);
    // A shared mailbox's calendar is asked for in that mailbox.
    forget(&server);
    crate::ews_pim::sync(&c, &mailcraft_model::pim::Pim::default(), 0).unwrap();
    assert!(server.data.lock().unwrap().anchors.iter().any(|a| a == "sales@contoso.example"));
}

/// Public folders on Exchange 2013 and later: the hierarchy and the content each in a
/// mailbox of their own.
fn with_public_folders() -> Data {
    let mut d =
        Data { pf_mailbox: Some("PFContent@contoso.example".into()), pf_hierarchy: Some("PFHierarchy@contoso.example".into()), ..Default::default() };
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    d.folder("Contacts", None, Some("contacts"), "IPF.Contact");
    d.folder("IPM_SUBTREE", None, Some("publicfoldersroot"), "IPF.Note");
    d
}

fn public_root(d: &Data) -> String {
    d.folders.iter().find(|f| f.known.as_deref() == Some("publicfoldersroot")).map(|f| f.id.clone()).unwrap()
}

/// A public folder holding more folders than one page of the server's answer has (the fake
/// answers three at a time): every one is listed, at the top and further down.
#[test]
fn every_page_of_a_public_folders_folders_is_listed() {
    let mut d = with_public_folders();
    let root = public_root(&d);
    let names: Vec<String> = (1..=7).map(|i| format!("Project {i}")).collect();
    let mut last = String::new();
    for n in &names {
        last = d.folder(n, Some(&root), None, "IPF.Note");
    }
    for i in 1..=5 {
        d.folder(&format!("Part {i}"), Some(&last), None, "IPF.Note");
    }
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    let paths: Vec<String> = crate::ews::list_folders(&c, &mut crate::jmap::State::default(), "a1").unwrap().into_iter().map(|f| f.path).collect();
    for n in names.iter().chain(&(1..=5).map(|i| format!("Part {i}")).collect::<Vec<_>>()) {
        assert!(paths.iter().any(|p| p.ends_with(&format!("/{n}"))), "{n} in {paths:?}");
    }
}

/// A server that lists a public folder's folders without ids and says, page after page, that
/// more follow: the listing ends at the first page that brings no folder, where it was asked
/// for without end. The account's own folders are listed all the same.
#[test]
fn a_public_folder_listing_that_never_ends_is_given_up() {
    let mut d = with_public_folders();
    d.endless_public = true;
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    let folders = crate::ews::list_folders(&c, &mut crate::jmap::State::default(), "a1").unwrap();
    assert!(folders.iter().any(|f| f.path == "INBOX"), "{:?}", folders.iter().map(|f| &f.path).collect::<Vec<_>>());
    assert!(folders.iter().all(|f| !f.path.contains("No id")));
    let pages = ops(&server, "FindFolder");
    assert!(pages < folders::ENDLESS_PAGES / 2, "{pages} pages asked for");
}

/// The names of the calendars and address books a calendar and contacts sync read.
fn read(sync: &mailcraft_model::pim::DavSync) -> (Vec<String>, Vec<String>) {
    let calendars = sync.calendars.iter().flatten().map(|c| c.name.clone()).collect();
    let books = sync.books.iter().flatten().map(|b| b.name.clone()).collect();
    (calendars, books)
}

/// A sync lists the folders, then reads the calendars and contacts: the public folders were
/// walked again for them (a FindFolder for each folder holding others) and each shared
/// mailbox added by hand looked up again; the listing's are used. A folder made on the
/// server since still comes in: the calendars read with no listing before walk them again.
#[test]
fn a_sync_walks_the_public_folders_once() {
    let mut d = with_public_folders();
    let root = public_root(&d);
    let team = d.folder("Team", Some(&root), None, "IPF.Note");
    d.folder("Rooms", Some(&team), None, "IPF.Appointment");
    d.folder("Company contacts", Some(&root), None, "IPF.Contact");
    d.shared_mailbox(SharedBox { address: "desk@contoso.example".into(), name: "Front desk".into(), automap: false, legacy: false, granted: true });
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    c.set_shared(&["desk@contoso.example".to_string()]);
    let (mut st, pim) = (crate::jmap::State::default(), mailcraft_model::pim::Pim::default());
    let paths =
        |st: &mut crate::jmap::State| -> Vec<String> { crate::ews::list_folders(&c, st, "a1").unwrap().into_iter().map(|f| f.path).collect() };
    assert!(paths(&mut st).contains(&"Front desk/Inbox".to_string()));
    forget(&server);
    let sync = crate::ews_pim::sync(&c, &pim, 0).unwrap();
    assert_eq!((ops(&server, "FindFolder"), ops(&server, "ResolveNames")), (0, 0), "nothing asked again after the listing");
    let (calendars, books) = read(&sync);
    assert_eq!((calendars, books), (vec!["Calendar".to_string(), "Team › Rooms".into()], vec!["Contacts".to_string(), "Company contacts".into()]));
    assert!(server.data.lock().unwrap().anchors.iter().any(|a| a == "desk@contoso.example"), "the shared mailbox's calendar is looked for");
    // Made on the server since, then read without a listing before: walked again.
    {
        let mut d = server.data.lock().unwrap();
        d.folder("Desks", Some(&root), None, "IPF.Appointment");
        d.folder("Notices", Some(&root), None, "IPF.Note");
    }
    forget(&server);
    let sync = crate::ews_pim::sync(&c, &pim, 0).unwrap();
    assert!(ops(&server, "FindFolder") > 0 && ops(&server, "ResolveNames") > 0);
    assert!(read(&sync).0.contains(&"Desks".to_string()));
    // The next listing has it, and the calendars read after it.
    assert!(paths(&mut st).iter().any(|p| p.ends_with("/Notices")));
    forget(&server);
    let sync = crate::ews_pim::sync(&c, &pim, 0).unwrap();
    assert_eq!(ops(&server, "FindFolder"), 0);
    assert!(read(&sync).0.contains(&"Desks".to_string()));
}

/// A sync (the folders listed, then the calendars and contacts read) with 420 public folders
/// (40 of them calendars and address books) and three shared mailboxes added by hand, five
/// times: how long, how many requests, and a digest of what was listed and read.
/// Run: `cargo test --release -p mailcraft-net --lib measure_syncs_with_public_folders -- --ignored --nocapture`.
#[test]
#[ignore = "a measurement: run in release"]
fn measure_syncs_with_public_folders() {
    use std::hash::{Hash, Hasher};
    let mut d = with_public_folders();
    let root = public_root(&d);
    for t in 0..20 {
        let area = d.folder(&format!("Area {t}"), Some(&root), None, "IPF.Note");
        for k in 0..20 {
            let class = match k {
                0 => "IPF.Appointment",
                1 => "IPF.Contact",
                _ => "IPF.Note",
            };
            d.folder(&format!("Folder {t}.{k}"), Some(&area), None, class);
        }
    }
    let shared: Vec<String> = (0..3).map(|n| format!("box{n}@contoso.example")).collect();
    for (n, address) in shared.iter().enumerate() {
        d.shared_mailbox(SharedBox { address: address.clone(), name: format!("Box {n}"), automap: false, legacy: false, granted: true });
    }
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    c.set_shared(&shared);
    let (mut st, pim) = (crate::jmap::State::default(), mailcraft_model::pim::Pim::default());
    forget(&server);
    let mut digest = std::collections::hash_map::DefaultHasher::new();
    let began = std::time::Instant::now();
    for _ in 0..5 {
        let folders = crate::ews::list_folders(&c, &mut st, "a1").unwrap();
        folders.iter().map(|f| &f.path).collect::<Vec<_>>().hash(&mut digest);
        let sync = crate::ews_pim::sync(&c, &pim, 0).unwrap();
        read(&sync).hash(&mut digest);
        sync.errors.hash(&mut digest);
    }
    let took = began.elapsed();
    let total = server.data.lock().unwrap().calls.len();
    println!(
        "MEASURE 5 syncs, 420 public folders, 3 shared mailboxes: {} ms, {total} requests ({} FindFolder, {} ResolveNames), digest {:016x}",
        took.as_millis(),
        ops(&server, "FindFolder"),
        ops(&server, "ResolveNames"),
        digest.finish()
    );
}
