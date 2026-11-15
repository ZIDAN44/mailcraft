use std::cell::Cell;

use super::*;

/// Found in review: the list of MailCraft's own copies of drafts, when it couldn't be read, was
/// taken for none and written over with the one just filed, every other copy in it forgotten
/// (and left on the server beside the next filing of its draft). It's left as it was.
#[test]
fn a_list_of_drafts_filed_that_cant_be_read_isnt_written_over() {
    let store = Store::failing();
    let list = serde_json::json!([{ "message_id": "a@x", "mark": "m1" }, { "message_id": "b@x", "mark": "m2" }]);
    store.save_account_doc("a1", "drafts-filed.json", &list).unwrap();
    let pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    let acc = AccountConfig { id: "a1".into(), ..Default::default() };
    let w = Worker::new(acc, Auth::shared("pw"), store.clone(), pool.sink.clone(), SyncOptions::default());
    store.fail(Some("drafts-filed.json"), None);
    w.filed_draft("c@x", "m3");
    store.fail(None, None);
    assert_eq!(store.load_account_doc::<serde_json::Value>("a1", "drafts-filed.json").unwrap(), list);
    // Read, the one filed is remembered with the others.
    w.filed_draft("c@x", "m3");
    assert_eq!(store.load_account_doc::<Vec<serde_json::Value>>("a1", "drafts-filed.json").unwrap().len(), 3);
}

/// An outcome by name.
fn named(o: &Outcome) -> &'static str {
    match o {
        Outcome::LeftToPool(None) => "left to the pool",
        Outcome::LeftToPool(Some(_)) => "left to the pool, failed here",
        Outcome::Removed(None) => "removed, filed",
        Outcome::Removed(Some(_)) => "removed, not filed",
        Outcome::Filed(..) => "filed",
        Outcome::KeepMeanwhile => "kept meanwhile",
        Outcome::Failed(_) => "failed",
    }
}

/// Each way a save handed to a worker can go, and what's done for it: the table `save_draft`'s
/// match kept in the order of its guards, before the outcomes had names.
#[test]
fn each_way_a_save_goes_has_its_one_outcome() {
    let filing = Arc::new(Filing::default());
    for (n, ours, removed) in (0..4).flat_map(|n| [(n, false, false), (n, false, true), (n, true, false), (n, true, true)]) {
        let being_filed = Filing::begin(&filing, "a1", n, "m");
        let r = match n {
            0 => Ok(Saving::LeftToPool),
            1 => Ok(Saving::Elsewhere),
            2 => Ok(Saving::Filed(("Drafts".into(), false), being_filed.unwrap())),
            _ => Err(NetError::Closed("dropped".into())),
        };
        let asked = Cell::new(0);
        let o = Outcome::of(r, ours, || {
            asked.set(asked.get() + 1);
            removed
        });
        let want = match (n, ours, removed) {
            (0, ..) | (1, false, _) => "left to the pool",
            (1, true, _) => "kept meanwhile",
            (2, _, true) => "removed, filed",
            (2, _, false) => "filed",
            (_, false, _) => "left to the pool, failed here",
            (_, true, true) => "removed, not filed",
            (_, true, false) => "failed",
        };
        assert_eq!(named(&o), want, "save {n}, ours {ours}, removed {removed}");
        // The account looked at only where it decides (as the guards did).
        let looks = usize::from(n == 2 || (n == 3 && ours));
        assert_eq!(asked.get(), looks, "save {n}, ours {ours}, removed {removed}");
    }
}
