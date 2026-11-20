//! Engine tests: folders in an order of the user's own (issue #183). The folder pane kept them A
//! to Z, so they were numbered to keep an order.

use super::*;

/// The sample's folders, as the pane has them: the user's (Clients, Projects with Short film in
/// it) after the special folders.
fn names(s: &Session) -> Vec<String> {
    let a = s.account("sample").unwrap();
    s.pane_folders(a).into_iter().map(|f| f.path.clone()).collect()
}

#[test]
fn folders_go_where_they_are_put() {
    let mut s = sample_session("folder-order");
    s.run("folder.create", &json!({"account": "sample", "name": "Clients"})).unwrap();
    s.run("folder.create", &json!({"account": "sample", "name": "Zeta"})).unwrap();
    let user = |s: &Session| {
        names(s).into_iter().filter(|p| ["Clients", "Projects", "Projects/Short film", "Zeta"].contains(&p.as_str())).collect::<Vec<_>>()
    };
    assert_eq!(user(&s), ["Clients", "Projects", "Projects/Short film", "Zeta"], "A to Z at first");
    assert!(!s.folders_ordered("sample"));

    // Zeta before Clients; Projects after Zeta: the folder inside it goes with it.
    s.run("folder.order", &json!({"account": "sample", "folder": "Zeta", "before": "Clients"})).unwrap();
    assert_eq!(user(&s), ["Zeta", "Clients", "Projects", "Projects/Short film"]);
    s.run("folder.order", &json!({"account": "sample", "folder": "Projects", "after": "Zeta"})).unwrap();
    assert_eq!(user(&s), ["Zeta", "Projects", "Projects/Short film", "Clients"]);
    assert!(s.folders_ordered("sample"));
    let special = names(&s).into_iter().take(6).collect::<Vec<_>>();
    assert_eq!(special.first().map(String::as_str), Some("INBOX"), "special folders keep their places: {special:?}");

    // Move up and down among those beside it; not past the ends.
    s.run("folder.moveDown", &json!({"account": "sample", "folder": "Zeta"})).unwrap();
    assert_eq!(user(&s), ["Projects", "Projects/Short film", "Zeta", "Clients"]);
    assert!(!s.can_reorder_folder("sample", "Projects", -1) && s.can_reorder_folder("sample", "Projects", 1));
    assert!(s.run("folder.moveUp", &json!({"account": "sample", "folder": "Projects"})).is_err());
    assert!(s.run("folder.order", &json!({"account": "sample", "folder": "Projects", "before": "Projects/Short film"})).is_err(), "not into itself");
    assert!(s.run("folder.order", &json!({"account": "sample", "folder": "INBOX", "before": "Zeta"})).is_err(), "a special folder keeps its place");

    // Put beside a folder in another: moved there, in its place.
    s.run("folder.order", &json!({"account": "sample", "folder": "Clients", "before": "Projects/Short film"})).unwrap();
    assert_eq!(user(&s), ["Projects", "Projects/Short film", "Zeta"]);
    let inside: Vec<String> = names(&s).into_iter().filter(|p| p.starts_with("Projects/")).collect();
    assert_eq!(inside, ["Projects/Clients", "Projects/Short film"]);

    // Renamed, it keeps its place; deleted, it leaves the order.
    s.run("folder.rename", &json!({"account": "sample", "folder": "Zeta", "name": "Alpha"})).unwrap();
    let pane = names(&s);
    let (p, a) = (pane.iter().position(|x| x == "Projects"), pane.iter().position(|x| x == "Alpha"));
    assert!(p < a, "Alpha where Zeta was, after Projects, not A to Z first: {pane:?}");
    let order = s.prefs.folder_order.get("sample").cloned().unwrap_or_default();
    assert!(order.contains(&"Alpha".to_string()) && !order.contains(&"Zeta".to_string()), "{order:?}");
    s.run("folder.delete", &json!({"account": "sample", "folder": "Alpha", "permanent": true})).unwrap();
    assert!(!s.prefs.folder_order.get("sample").cloned().unwrap_or_default().contains(&"Alpha".to_string()));

    // A to Z again.
    s.run("folder.sortAZ", &json!({"account": "sample"})).unwrap();
    assert!(!s.folders_ordered("sample"));
}
