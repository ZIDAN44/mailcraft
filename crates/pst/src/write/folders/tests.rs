//! The special folders of files whose nodes don't say where they are.

use std::path::PathBuf;

use super::*;
use crate::Format;
use crate::scratch::scratch;
use crate::write::{create_empty, create_folder};

/// A file `name` in a scratch folder of its own.
fn file(name: &str) -> PathBuf {
    scratch(&format!("pst-folders-{name}")).join(name)
}

/// The folder at `names` in the file, with its node's parent made `parent(node, as it was)`.
fn reparent(path: &Path, names: &[&str], parent: impl Fn(&Editor) -> u32) {
    let (node, mut ed) = editor_at(path, names, "IPF.Note").unwrap();
    let entry = ed.node(node).unwrap();
    let to = parent(&ed);
    assert_ne!(entry.parent, to);
    ed.set_node(node, entry.data, entry.sub, to);
    ed.commit().unwrap();
}

/// A recorded folder is found by the folders that list it when its node names no parent (0, as
/// other programs write it), a parent that isn't above it (stale: the path through it was
/// given, though no folder there lists it), or itself: found only by going up from node to
/// parent, the file's Sent Items and Drafts weren't found, or were said to be where they aren't.
#[test]
fn a_special_folder_is_found_whatever_its_node_says_its_parent_is() {
    let sent = Some(vec!["Archive".to_string(), "Sent".to_string()]);
    let drafts = Some(vec!["Drafts".to_string()]);
    let make = |name: &str| {
        let path = file(name);
        create_empty(&path, Format::Unicode).unwrap();
        create_folder(&path, &["Elsewhere", "Deeper"], "IPF.Note").unwrap();
        set_special_folder(&path, SpecialFolder::SentItems, &["Archive", "Sent"]).unwrap();
        set_special_folder(&path, SpecialFolder::Drafts, &["Drafts"]).unwrap();
        assert_eq!(special_folder(&path, SpecialFolder::SentItems).unwrap(), sent);
        assert_eq!(special_folder(&path, SpecialFolder::Drafts).unwrap(), drafts);
        path
    };
    // No parent named: the folder itself, then the one above it.
    let path = make("zeroed.pst");
    reparent(&path, &["Archive", "Sent"], |_| 0);
    reparent(&path, &["Drafts"], |_| 0);
    assert_eq!(special_folder(&path, SpecialFolder::SentItems).unwrap(), sent);
    assert_eq!(special_folder(&path, SpecialFolder::Drafts).unwrap(), drafts);
    reparent(&path, &["Archive"], |_| 0);
    assert_eq!(special_folder(&path, SpecialFolder::SentItems).unwrap(), sent);
    // A parent that doesn't list it: not said to be under that one.
    let path = make("stale.pst");
    let deeper = |ed: &Editor| {
        let top = ed.store.ipm_subtree().unwrap();
        let elsewhere = ed.find_child(top, "Elsewhere").unwrap().unwrap();
        ed.find_child(elsewhere, "Deeper").unwrap().unwrap()
    };
    reparent(&path, &["Archive", "Sent"], deeper);
    assert_eq!(special_folder(&path, SpecialFolder::SentItems).unwrap(), sent);
    // Its own parent.
    let path = make("own.pst");
    let itself = |ed: &Editor| ed.find_child(ed.store.ipm_subtree().unwrap(), "Drafts").unwrap().unwrap();
    reparent(&path, &["Drafts"], itself);
    assert_eq!(special_folder(&path, SpecialFolder::Drafts).unwrap(), drafts);
}
