//! The tests' random numbers: xorshift64, the same stream from the same seed on every run, for
//! bytes that don't compress, damage at made-up places and made-up words.

/// A repeatable stream of numbers (xorshift64). A seed of 0 gives only zeros.
pub(crate) struct Rng(u64);

impl Rng {
    pub(crate) fn new(seed: u64) -> Rng {
        Rng(seed)
    }

    /// The stream's next number.
    pub(crate) fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

/// `len` bytes from the stream seeded with `seed`, the low byte of each number: the same each
/// time, and nothing a compressor or a writer keeping repeated blocks once could make smaller.
pub(crate) fn noise(len: usize, seed: u64) -> Vec<u8> {
    let mut rng = Rng::new(seed);
    (0..len).map(|_| rng.next_u64() as u8).collect()
}
