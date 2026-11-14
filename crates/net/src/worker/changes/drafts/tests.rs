use std::cell::Cell;

use super::*;

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
