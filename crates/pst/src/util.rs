//! Reading numbers out of bytes that can't be trusted: every read says when it falls off the end.

use std::io;

/// An error saying the file isn't what it should be.
pub(crate) fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

pub(crate) fn short() -> io::Error {
    invalid("a record ends past its data")
}

pub(crate) fn u8_at(b: &[u8], at: usize) -> io::Result<u8> {
    b.get(at).copied().ok_or_else(short)
}

pub(crate) fn bytes_at<const N: usize>(b: &[u8], at: usize) -> io::Result<[u8; N]> {
    b.get(at..at.checked_add(N).ok_or_else(short)?).and_then(|s| s.try_into().ok()).ok_or_else(short)
}

pub(crate) fn u16_at(b: &[u8], at: usize) -> io::Result<u16> {
    bytes_at(b, at).map(u16::from_le_bytes)
}

pub(crate) fn u32_at(b: &[u8], at: usize) -> io::Result<u32> {
    bytes_at(b, at).map(u32::from_le_bytes)
}

pub(crate) fn u64_at(b: &[u8], at: usize) -> io::Result<u64> {
    bytes_at(b, at).map(u64::from_le_bytes)
}

/// `n` as a `usize`, when it fits.
pub(crate) fn size(n: u64) -> io::Result<usize> {
    usize::try_from(n).map_err(|_| invalid("a size too big to hold"))
}

/// `n` rounded up to the next multiple of 64 (where blocks start and end).
pub(crate) fn align64(n: usize) -> usize {
    n.saturating_add(63) & !63
}
