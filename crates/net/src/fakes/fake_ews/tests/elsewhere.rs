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
    let store = crate::fake::tests_store("ews-elsewhere");
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
        assert_eq!(&crate::ews::fetch_body(&c, &st, &store, &fs.messages[0].key).unwrap().0, raw, "{path}");
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
    let _ = std::fs::remove_dir_all(store.root());
}
