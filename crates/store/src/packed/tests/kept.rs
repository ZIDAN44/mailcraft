//! A data file tells how it keeps a file by the file's first bytes (compressed, or a note of one
//! in pieces): a message that itself starts with those bytes reads back as it came, and what an
//! earlier build kept reads as it did.

use std::sync::atomic::AtomicBool;

use super::compact::{Kept, old_store, read_back};
use super::*;

/// What a compressed file starts with, and the note of one in pieces (as every build wrote them).
const DEFLATED: &[u8] = b"\0MailCraft:deflate\0";
const PIECED: &[u8] = b"\0MailCraft:pieces\0";

/// Messages (hostile: a server sends what it likes) that start with the data file's own markers,
/// short and long, compressible or not, whole and big enough for pieces.
fn marked() -> Vec<Vec<u8>> {
    let big = super::super::WHOLE_MAX * 3;
    let note = |len: u64, rest: &[u8]| [PIECED, &len.to_le_bytes(), rest].concat();
    vec![
        DEFLATED.to_vec(),
        PIECED.to_vec(),
        [DEFLATED, b"hello"].concat(),
        [DEFLATED, &noise(5_000, 1)].concat(),
        [DEFLATED, "Subject: a\r\n\r\nwords words words ".repeat(200).as_bytes()].concat(),
        [DEFLATED, &noise(big, 2)].concat(),
        note(0, b"read as nothing at all"),
        note(5_000, b"read as a file whose pieces are gone"),
        note(7, &noise(5_000, 3)),
        [PIECED, &noise(big, 4)].concat(),
        [PIECED, "Subject: b\r\n\r\nwords words words ".repeat(20_000).as_bytes()].concat(),
        b"\0".to_vec(),
        b"\0MailCraft:pie".to_vec(),
    ]
}

/// Every such message reads back as it came, written one at a time (compressed in the
/// transaction) or made ready away from it (an import's batches); and the data file is whole: its
/// health reads, it's compressed again and copied, and they still read back.
#[test]
fn a_message_starting_with_the_data_files_markers_reads_back_as_it_came() {
    let t = temp("kept-marked");
    let s = packed_store(&t);
    let sources = marked();
    for (i, src) in sources.iter().enumerate() {
        s.write_body(&inbox(i as u32 + 1), src).unwrap();
        let prepared = Store::prepare(true, src.clone());
        assert_eq!(&prepared.source().unwrap(), src, "made ready, source {i}");
        s.write_prepared(vec![(inbox(i as u32 + 101), prepared)], true).unwrap();
    }
    let check = |s: &Store, when: &str| {
        for (i, src) in sources.iter().enumerate() {
            for uid in [i as u32 + 1, i as u32 + 101] {
                assert_eq!(s.load_body(&inbox(uid)).unwrap().as_ref(), Some(src), "{when}: source {i}, uid {uid}");
            }
        }
    };
    check(&s, "written");
    assert!(s.data_file_health("a1").unwrap().is_some());
    s.recompress_data_file("a1", &AtomicBool::new(false), &mut |_| {}).unwrap().unwrap();
    check(&s, "compressed again");
    assert!(s.copy_data_file("a1", &mut |_| true, &mut || true).unwrap().unwrap().copied, "no file is taken as damaged");
    check(&s, "copied");
    drop(s);
}

proptest::proptest! {
    #![proptest_config(proptest::prelude::ProptestConfig { cases: 256, ..proptest::prelude::ProptestConfig::default() })]

    /// Any file, starting with a marker, part of one, or neither, comes back from how it's kept
    /// as it was, and what's kept never reads as the note of a file in pieces.
    #[test]
    fn whatever_a_file_starts_with_it_comes_back(
        head in proptest::sample::select(vec![&b""[..], DEFLATED, PIECED, &DEFLATED[..7], &PIECED[..PIECED.len() - 1], b"\0"]),
        len in proptest::prelude::any::<u64>(),
        rest in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..3_000),
        repeat in 0usize..4,
    ) {
        let mut file = [head, &len.to_le_bytes()[..], &rest].concat();
        for _ in 0..repeat {
            file.extend_from_within(..);
        }
        let kept = super::super::pack(&file).into_owned();
        proptest::prop_assert!(super::super::pieced_len(&kept).is_none());
        proptest::prop_assert_eq!(super::super::unpack(kept).unwrap(), file);
    }
}

/// What an earlier build kept reads as it did: raw or compressed, whole or in pieces, the notes
/// and markers as it wrote them.
#[test]
fn what_an_earlier_build_kept_reads_as_it_did() {
    let t = temp("kept-old");
    let text = "Subject: report\r\n\r\nThe figures are in the attached sheet. ".repeat(3_000).into_bytes();
    let deflate = |b: &[u8]| [DEFLATED, &miniz_oxide::deflate::compress_to_vec(b, 1)].concat();
    let big_raw = noise(super::super::WHOLE_MAX * 5 + 17, 9);
    let small = b"Subject: x\r\n\r\nshort".to_vec();
    let kept: Kept = vec![
        ("INBOX/1/1.eml".into(), small.clone()),
        ("INBOX/1/2.eml".into(), deflate(&text)),
        ("INBOX/1/3.eml".into(), big_raw.clone()),
        ("INBOX/1/4.eml".into(), deflate(&[text.as_slice(), &noise(200_000, 10)].concat())),
        ("INBOX/1/5.eml".into(), noise(3_000, 11)),
    ];
    let s = old_store(&t, &kept);
    {
        // The big ones as an earlier build noted them: the marker, then the length kept.
        let db = packs(&s).db("a1", false).unwrap().unwrap();
        let tx = redb::ReadableDatabase::begin_read(&*db).unwrap();
        let files = tx.open_table(super::super::FILES).unwrap();
        for (k, v) in &kept {
            let note = files.get(k.as_str()).unwrap().unwrap().value().to_vec();
            if v.len() > super::super::WHOLE_MAX {
                assert_eq!(note, [PIECED, &(v.len() as u64).to_le_bytes()].concat(), "{k}");
            } else {
                assert_eq!(&note, v, "{k}");
            }
        }
    }
    let back = read_back(&s);
    let want = [small, text.clone(), big_raw, [text.as_slice(), &noise(200_000, 10)].concat(), noise(3_000, 11)];
    for ((k, _), w) in kept.iter().zip(&want) {
        assert_eq!(back.get(k), Some(&Some(w.clone())), "{k}");
    }
    drop(s);
}
