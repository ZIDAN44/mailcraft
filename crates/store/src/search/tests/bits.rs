//! Search index: the bits of the docs a word was found in.

use proptest::prelude::*;

use super::*;

proptest! {
    /// The docs set, in order and the last of them, as each bit of each word looked at says, for
    /// any docs set over any room (a partial last word, a whole one, none).
    #[test]
    fn the_docs_set_are_those_set(len in 0u32..700, picks in prop::collection::vec(0u32..800, 0..60)) {
        let mut b = Bits::new(len);
        for &d in &picks {
            b.set(d);
        }
        let every: Vec<u32> = (0..len.div_ceil(64) * 64).filter(|&d| b.get(d)).collect();
        prop_assert_eq!(b.ones().collect::<Vec<_>>(), every.clone());
        prop_assert_eq!(b.last(), every.last().copied());
        prop_assert_eq!(b.ones().count(), b.count() as usize);
    }
}

#[test]
fn the_first_and_last_bits_of_a_word() {
    let mut b = Bits::new(200);
    for d in [0, 63, 64, 127, 199] {
        b.set(d);
    }
    assert_eq!(b.ones().collect::<Vec<_>>(), [0, 63, 64, 127, 199]);
    assert_eq!(b.last(), Some(199));
    assert_eq!(Bits::new(200).last(), None);
    assert_eq!(Bits::new(0).ones().count(), 0);
}
