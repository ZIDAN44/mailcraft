//! FNV-1a, 64 bits: the small, stable hash MailCraft keys and fingerprints are made with. Some
//! are kept (conversation keys, folders' directory names, a data file's checkpoint), so it must
//! never change: one copy, tested against the published values.

/// The hash of nothing.
pub const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
/// What each byte is folded in with.
pub const FNV_PRIME: u64 = 0x0100_0000_01b3;

/// FNV-1a of `bytes`.
pub fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h = Fnv::new();
    h.add(bytes);
    h.0
}

/// FNV-1a taken piece by piece: the same as [`fnv1a`] of the pieces one after another. As a
/// [`std::hash::Hasher`], numbers go in as that trait writes them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fnv(u64);

impl Default for Fnv {
    fn default() -> Fnv {
        Fnv(FNV_OFFSET)
    }
}

impl Fnv {
    pub fn new() -> Fnv {
        Fnv::default()
    }

    /// Fold `bytes` in.
    pub fn add(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.0 = (self.0 ^ u64::from(*b)).wrapping_mul(FNV_PRIME);
        }
    }

    /// The hash of what was folded in.
    pub fn value(&self) -> u64 {
        self.0
    }
}

impl std::hash::Hasher for Fnv {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        self.add(bytes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The published FNV-1a 64-bit values (Fowler, Noll and Vo's test suite).
    #[test]
    fn the_published_values() {
        assert_eq!(fnv1a(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv1a(b"foobar"), 0x8594_4171_f739_67e8);
        let mut h = Fnv::new();
        h.add(b"foo");
        h.add(b"");
        h.add(b"bar");
        assert_eq!(h.value(), fnv1a(b"foobar"));
    }
}
