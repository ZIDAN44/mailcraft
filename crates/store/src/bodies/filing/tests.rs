//! A filing given up takes away what it wrote, and nothing else.

use super::*;

/// A filing dropped unfinished removes the sources it wrote and no others: it removed every
/// source of the folder numbered at or past its first, so one another writer had put there (an
/// import writes its sources before the list names them) went too.
#[test]
fn a_filing_given_up_removes_only_its_own_sources() {
    for packed in [false, true] {
        let dir = crate::tests::scratch("filing-own");
        let s = if packed { Store::open_packed(dir.join("app"), dir.join("Files")).unwrap() } else { Store::memory() };
        let key = |uid: u32| MessageKey { account: "a1".into(), folder: "Archive".into(), validity: 1, uid };
        s.write_body(&key(1), b"Subject: listed\r\n\r\nx").unwrap();
        s.save_folder(
            "a1",
            "Archive",
            &FolderState { validity: 1, uid_next: 2, messages: vec![Summary { key: key(1), ..Default::default() }.into()], ..Default::default() },
        )
        .unwrap();
        let mut filing = Filing::new(&s, "a1", "Archive").unwrap();
        // More than a batch: the first batch is written; one filed without a source between.
        let count = u32::try_from(BATCH).unwrap() + 2;
        let mut filed = Vec::new();
        for i in 0..count {
            let k = filing.key().unwrap();
            filed.push(k.uid);
            filing.add(Summary { key: k, ..Default::default() }, (i != 5).then(|| format!("Subject: {i}\r\n\r\nx").into_bytes())).unwrap();
        }
        assert_eq!(filed.first(), Some(&2));
        // Another writer's, past the list: in the gap the filing left, and past its last.
        let (gap, past) = (filed[5], filed[filed.len() - 1] + 10);
        s.write_body(&key(gap), b"Subject: another's\r\n\r\nx").unwrap();
        s.write_body(&key(past), b"Subject: another's\r\n\r\nx").unwrap();
        assert!(s.has_body(&key(filed[0])), "{packed}: a batch was written");
        drop(filing);
        let mut left = s.cached_bodies("a1", "Archive", 1);
        left.sort_unstable();
        assert_eq!(left, [1, gap, past], "{packed}");
        drop(s);
    }
}
