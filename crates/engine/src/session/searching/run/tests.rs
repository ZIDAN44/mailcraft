//! Search runs' own tests: what needs a run's insides.

use std::time::{Duration, Instant};

use super::*;

/// A run over two accounts' 40 folders, the first of them on view (the second account's name
/// first in the alphabet), holding a common phrase's 16,667 candidates to check, in the
/// folders' order, their docs in no order.
fn run_to_verify() -> SearchRun {
    let folders: Vec<(String, String)> = (0..40)
        .map(|i| (if i < 25 { "zed" } else { "alpha" }.to_string(), if i == 0 { "INBOX".to_string() } else { format!("Folder {i}") }))
        .collect();
    let mut run = SearchRun::new("\"a b\"", SearchScope::All, Query::parse("\"a b\""), folders.clone(), false);
    run.on_view = 1;
    for (n, x) in (0..16_667u32).zip(crate::tests::Xorshift(crate::tests::XORSHIFT_SEED)) {
        let fi = usize::try_from(u64::from(n) * 40 / 16_667).unwrap();
        let (account, folder) = folders[fi].clone();
        let key = MessageKey { account, folder, validity: 1, uid: n + 1 };
        let m = SharedSummary::new(Summary { key, ..Default::default() });
        // Some docs alike: each account's index numbers its own.
        let doc = u32::try_from(x % 20_000).unwrap();
        run.verify.push_back(Candidate { fi, m, doc: Some(doc), awaiting: false, at: Some(usize::try_from(n).unwrap()) });
    }
    run.verify_sorted = false;
    run
}

/// The candidates are checked on-view folders first, then by account (by name), then in their
/// docs' order: as when the order was worked out from the folders' names, each compared.
#[test]
fn candidates_are_checked_on_view_first_then_by_account_and_doc() {
    let mut run = run_to_verify();
    let view = run.folders.get(..run.on_view).unwrap_or_default().to_vec();
    let off = |c: &Candidate| !view.iter().any(|(a, f)| *a == c.m.key.account && *f == c.m.key.folder);
    let mut want: Vec<Candidate> = run.verify.iter().cloned().collect();
    want.sort_by(|a, b| off(a).cmp(&off(b)).then(a.m.key.account.cmp(&b.m.key.account)).then(a.doc.cmp(&b.doc)));
    run.sort_verify();
    let order = |cs: &mut dyn Iterator<Item = &Candidate>| cs.map(|c| (c.fi, c.m.key.uid, c.doc)).collect::<Vec<_>>();
    assert_eq!(order(&mut run.verify.iter()), order(&mut want.iter()));
    assert!(run.verify.front().is_some_and(|c| c.fi == 0), "the folder on view first");
    assert_eq!(run.verify.get(500).map(|c| c.m.key.account.as_str()), Some("alpha"), "then the accounts by name");
}

/// How long putting a common phrase's 16,667 candidates in order takes (`cargo test --release
/// -p mailcraft-engine --lib sorting_the_candidates -- --ignored --nocapture`).
#[test]
#[ignore = "a measurement: run in release"]
fn sorting_the_candidates_takes() {
    let mut took: Vec<Duration> = (0..21)
        .map(|_| {
            let mut run = run_to_verify();
            let t = Instant::now();
            run.sort_verify();
            t.elapsed()
        })
        .collect();
    took.sort();
    println!("MEASURE sorting 16,667 candidates: median {:?}, best {:?}", took[10], took[0]);
}
