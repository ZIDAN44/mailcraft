//! Search index: the texts kept, read one after another as a phrase is verified.

use super::*;

/// Texts read one after another from the same pages, and again once a page is written anew
/// under the next generation: each doc's own, a removed doc's none, a page read once while the
/// cache holds it; one damaged is damage however often it's asked for.
#[test]
fn texts_read_one_after_another_are_each_docs_own() {
    let s = Store::memory();
    let mut w = Writer::open(&s, "a1").unwrap();
    let docs: Vec<u32> = (0..600u32)
        .map(|i| w.add(&mk("INBOX", i + 1), 1, 1, &text(&format!("text {i} {}", "word ".repeat(i as usize % 300)), &[], 0)).unwrap())
        .collect();
    w.flush(&s).unwrap();
    let mut r = Reader::open(&s, "a1").unwrap().unwrap();
    let pages = load_meta(&s, "a1").unwrap().unwrap().text_pages.len() as u64;
    assert!(pages > 1, "{pages} pages");
    let first = |r: &mut Reader, d: u32| r.text(&s, d).unwrap().map(|b| b.text.split(' ').take(2).collect::<Vec<_>>().join(" "));
    for _ in 0..2 {
        for (i, &d) in docs.iter().enumerate() {
            assert_eq!(first(&mut r, d), Some(format!("text {i}")));
        }
    }
    assert_eq!(r.text_pages_read(), pages, "each page read once");
    // The first page written again without one of its texts.
    w.remove(docs[1]);
    let new = w.add(&mk("INBOX", 1000), 1, 1, &text("text new", &[], 0)).unwrap();
    w.flush(&s).unwrap();
    assert!(r.refresh(&s).unwrap());
    assert_eq!(first(&mut r, docs[0]), Some("text 0".into()));
    assert_eq!(first(&mut r, docs[1]), None);
    assert_eq!(first(&mut r, docs[2]), Some("text 2".into()));
    assert_eq!(first(&mut r, new), Some("text new".into()));
    // A page damaged: an error each time.
    let meta = load_meta(&s, "a1").unwrap().unwrap();
    let t = meta.text_pages[0];
    let path = Name::Text { generation: t.generation, page: 0 }.path(&dir(&s, "a1"));
    let mut b = s.files.read(&path).unwrap();
    b.truncate(b.len() - 1);
    s.files.write(&path, &b).unwrap();
    let mut r = Reader::open(&s, "a1").unwrap().unwrap();
    for _ in 0..2 {
        assert!(r.text(&s, docs[0]).is_err_and(|e| is_damaged(&e)));
    }
}
