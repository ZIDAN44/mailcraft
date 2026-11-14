//! The Outbox's files, when they can't be changed.

use super::*;

/// A message taken out of the Outbox whose files can't be removed isn't listed again (it would be
/// sent again); what's left goes once it can. A new message of the id isn't hidden by the mark.
#[test]
fn an_outbox_message_taken_out_stays_out() {
    let s = Store::failing();
    let item = OutboxItem { id: 7, from: "z@x".into(), recipients: vec!["a@y".into()], ..Default::default() };
    let files = |s: &Store| s.files.list(&s.outbox_dir("a1")).into_iter().map(|(n, _)| n).collect::<Vec<_>>();
    // One file held: the other going is enough. Both: the mark keeps it out. The mark says to
    // remove what's left.
    for held in ["outbox/7.json", "outbox/7.eml", "outbox/7.json|outbox/7.eml"] {
        s.outbox_add("a1", &item, b"x").unwrap();
        s.fail(None, Some(held));
        s.outbox_remove("a1", 7).unwrap();
        assert!(s.outbox("a1").is_empty(), "{held}");
        s.outbox_sweep("a1");
        assert!(s.outbox("a1").is_empty(), "{held}");
        s.fail(None, None);
        assert!(files(&s).iter().any(|p| p.ends_with("7.gone")), "{held}: {:?}", files(&s));
        s.outbox_sweep("a1");
        assert!(files(&s).is_empty(), "{held}: {:?}", files(&s));
    }
    // Nothing can be written there: still in the Outbox, and said.
    s.outbox_add("a1", &item, b"x").unwrap();
    s.fail(None, Some("outbox/7.json|outbox/7.eml|outbox/7.gone"));
    assert!(s.outbox_remove("a1", 7).is_err());
    assert_eq!(s.outbox("a1"), vec![item.clone()]);
    // Marked, then queued again with the same id: listed.
    s.fail(None, Some("outbox/7.json|outbox/7.eml"));
    s.outbox_remove("a1", 7).unwrap();
    s.fail(None, None);
    s.outbox_add("a1", &item, b"y").unwrap();
    assert_eq!(s.outbox("a1"), vec![item]);
    assert_eq!(s.outbox_raw("a1", 7).unwrap(), b"y");
}
