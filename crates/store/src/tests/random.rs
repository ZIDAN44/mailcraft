//! Numbers and bytes for tests, the same every run: one small xorshift.

/// A 64-bit xorshift (shifts 13, 7, 17), going on from the number it holds (never 0: it would
/// stay 0).
pub(crate) struct Rng(pub(crate) u64);

impl Rng {
    /// The next number.
    pub(crate) fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    /// The next number, below `n` (0 when `n` is 0).
    pub(crate) fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n.max(1)
    }
}

/// `n` bytes that don't compress, different for each seed.
pub(crate) fn noise(n: usize, seed: u64) -> Vec<u8> {
    let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    (0..n).map(|_| rng.next_u64() as u8).collect()
}
