//! Property values and how they are kept as bytes (MS-OXCDATA 2.11).

use std::io;

use crate::util::{bytes_at, invalid, short, u16_at, u32_at, u64_at};

/// Multi-valued entries read at most, whatever a value's count says.
const MAX_VALUES: usize = 1 << 20;

/// A property's type (`PtypXxx`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PropertyType(pub u16);

#[allow(missing_docs)]
impl PropertyType {
    pub const NULL: u16 = 0x0001;
    pub const INT16: u16 = 0x0002;
    pub const INT32: u16 = 0x0003;
    pub const FLOAT32: u16 = 0x0004;
    pub const FLOAT64: u16 = 0x0005;
    pub const CURRENCY: u16 = 0x0006;
    pub const APP_TIME: u16 = 0x0007;
    pub const ERROR: u16 = 0x000A;
    pub const BOOLEAN: u16 = 0x000B;
    pub const OBJECT: u16 = 0x000D;
    pub const INT64: u16 = 0x0014;
    pub const STRING8: u16 = 0x001E;
    pub const UNICODE: u16 = 0x001F;
    pub const TIME: u16 = 0x0040;
    pub const GUID: u16 = 0x0048;
    pub const BINARY: u16 = 0x0102;
    pub const MULTI: u16 = 0x1000;

    /// Whether properties have this type (MS-OXCDATA 2.11.1).
    pub(crate) fn known(self) -> bool {
        const M: u16 = PropertyType::MULTI;
        Self::kept_whole(self.0)
            || matches!(
                self.0,
                Self::NULL
                    | Self::INT16
                    | Self::INT32
                    | Self::FLOAT32
                    | Self::FLOAT64
                    | Self::CURRENCY
                    | Self::APP_TIME
                    | Self::ERROR
                    | Self::BOOLEAN
                    | Self::OBJECT
                    | Self::INT64
                    | Self::STRING8
                    | Self::UNICODE
                    | Self::TIME
                    | Self::GUID
                    | Self::BINARY
            )
            || (self.0 & M != 0
                && matches!(
                    self.0 & !M,
                    Self::INT16
                        | Self::INT32
                        | Self::FLOAT32
                        | Self::FLOAT64
                        | Self::CURRENCY
                        | Self::APP_TIME
                        | Self::INT64
                        | Self::STRING8
                        | Self::UNICODE
                        | Self::TIME
                        | Self::GUID
                        | Self::BINARY
                ))
    }

    /// A type MailCraft has no use for, whose bytes are kept whole to be written back: the
    /// server ID, the restriction and the rule action.
    pub(crate) fn kept_whole(kind: u16) -> bool {
        matches!(kind, 0x00FB | 0x00FD | 0x00FE)
    }

    /// Whether a value of the type fits a property record's four bytes (MS-PST 2.3.3.3).
    pub(crate) fn fits_record(self) -> bool {
        matches!(self.0, Self::INT16 | Self::INT32 | Self::FLOAT32 | Self::ERROR | Self::BOOLEAN)
    }
}

/// Bytes of a value (a binary, or a string in the file's 8-bit encoding).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Bytes(pub Vec<u8>);

impl Bytes {
    pub fn buffer(&self) -> &[u8] {
        &self.0
    }
}

/// A UTF-16 string as it is kept.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Utf16(pub Vec<u16>);

impl Utf16 {
    pub fn buffer(&self) -> &[u16] {
        &self.0
    }
}

/// A GUID, as MS-OXCDATA 2.5.1 keeps one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GuidValue {
    data1: u32,
    data2: u16,
    data3: u16,
    data4: [u8; 8],
}

impl GuidValue {
    pub const fn new(data1: u32, data2: u16, data3: u16, data4: [u8; 8]) -> GuidValue {
        GuidValue { data1, data2, data3, data4 }
    }

    pub fn data1(&self) -> u32 {
        self.data1
    }

    pub fn data2(&self) -> u16 {
        self.data2
    }

    pub fn data3(&self) -> u16 {
        self.data3
    }

    pub fn data4(&self) -> &[u8; 8] {
        &self.data4
    }

    pub(crate) fn parse(b: &[u8]) -> io::Result<GuidValue> {
        Ok(GuidValue { data1: u32_at(b, 0)?, data2: u16_at(b, 4)?, data3: u16_at(b, 6)?, data4: bytes_at(b, 8)? })
    }

    pub(crate) fn to_bytes(self) -> [u8; 16] {
        let mut b = [0u8; 16];
        b[..4].copy_from_slice(&self.data1.to_le_bytes());
        b[4..6].copy_from_slice(&self.data2.to_le_bytes());
        b[6..8].copy_from_slice(&self.data3.to_le_bytes());
        b[8..].copy_from_slice(&self.data4);
        b
    }
}

/// An object held in a subnode: an attached message, or OLE storage (`PtypObject`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ObjectRef {
    pub node: u32,
    pub size: u32,
}

/// A property's value.
#[derive(Clone, Debug, PartialEq)]
pub enum PropertyValue {
    Null,
    Integer16(i16),
    Integer32(i32),
    Floating32(f32),
    Floating64(f64),
    Currency(i64),
    FloatingTime(f64),
    ErrorCode(i32),
    Boolean(bool),
    Integer64(i64),
    String8(Bytes),
    Unicode(Utf16),
    Time(i64),
    Guid(GuidValue),
    Binary(Bytes),
    Object(ObjectRef),
    MultipleInteger16(Vec<i16>),
    MultipleInteger32(Vec<i32>),
    MultipleFloating32(Vec<f32>),
    MultipleFloating64(Vec<f64>),
    MultipleCurrency(Vec<i64>),
    MultipleFloatingTime(Vec<f64>),
    MultipleInteger64(Vec<i64>),
    MultipleString8(Vec<Bytes>),
    MultipleUnicode(Vec<Utf16>),
    MultipleTime(Vec<i64>),
    MultipleGuid(Vec<GuidValue>),
    MultipleBinary(Vec<Bytes>),
    /// A value of a type read only to be written back as it was, its type and bytes.
    Other(u16, Bytes),
}

/// The value of a fixed-size type kept in `b`, `None` for a type that isn't.
fn fixed(kind: u16, b: &[u8]) -> io::Result<Option<PropertyValue>> {
    use PropertyValue as V;
    let i64_of = |b: &[u8]| -> io::Result<i64> { bytes_at(b, 0).map(i64::from_le_bytes) };
    Ok(Some(match kind {
        PropertyType::INT16 => V::Integer16(i16::from_le_bytes(bytes_at(b, 0)?)),
        PropertyType::INT32 => V::Integer32(i32::from_le_bytes(bytes_at(b, 0)?)),
        PropertyType::FLOAT32 => V::Floating32(f32::from_le_bytes(bytes_at(b, 0)?)),
        PropertyType::FLOAT64 => V::Floating64(f64::from_bits(u64_at(b, 0)?)),
        PropertyType::CURRENCY => V::Currency(i64_of(b)?),
        PropertyType::APP_TIME => V::FloatingTime(f64::from_bits(u64_at(b, 0)?)),
        PropertyType::ERROR => V::ErrorCode(i32::from_le_bytes(bytes_at(b, 0)?)),
        PropertyType::BOOLEAN => V::Boolean(crate::util::u8_at(b, 0)? != 0),
        PropertyType::INT64 => V::Integer64(i64_of(b)?),
        PropertyType::TIME => V::Time(i64_of(b)?),
        PropertyType::GUID => V::Guid(GuidValue::parse(b)?),
        PropertyType::OBJECT => V::Object(ObjectRef { node: u32_at(b, 0)?, size: u32_at(b, 4)? }),
        _ => return Ok(None),
    }))
}

/// UTF-16LE text, up to its NUL (MS-OXCDATA 2.11.1).
fn utf16(b: &[u8]) -> Utf16 {
    Utf16(b.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).take_while(|c| *c != 0).collect())
}

/// 8-bit text, up to its NUL.
fn string8(b: &[u8]) -> Bytes {
    Bytes(b.iter().copied().take_while(|c| *c != 0).collect())
}

/// The pieces of a multi-valued variable-size value: a count, that many offsets, then the data.
fn pieces(b: &[u8]) -> io::Result<Vec<&[u8]>> {
    let count = crate::util::size(u64::from(u32_at(b, 0)?))?;
    if count > MAX_VALUES || count.saturating_mul(4).saturating_add(4) > b.len() {
        return Err(invalid("a list of values longer than its data"));
    }
    let offset = |i: usize| -> io::Result<usize> { crate::util::size(u64::from(u32_at(b, 4 + i * 4)?)) };
    (0..count)
        .map(|i| {
            let start = offset(i)?;
            let end = if i + 1 < count { offset(i + 1)? } else { b.len() };
            b.get(start..end).ok_or_else(short)
        })
        .collect()
}

/// A multi-valued fixed-size value: as many as fit.
fn each<T>(b: &[u8], width: usize, one: impl Fn(&[u8]) -> io::Result<T>) -> io::Result<Vec<T>> {
    let n = b.len() / width.max(1);
    if n > MAX_VALUES {
        return Err(invalid("a list of more values than any has"));
    }
    (0..n).map(|i| one(b.get(i * width..).ok_or_else(short)?)).collect()
}

/// The value of type `kind` kept as `b`.
pub(crate) fn parse(kind: u16, b: &[u8]) -> io::Result<PropertyValue> {
    use PropertyValue as V;
    if let Some(v) = fixed(kind, b)? {
        return Ok(v);
    }
    let i64_of = |b: &[u8]| -> io::Result<i64> { bytes_at(b, 0).map(i64::from_le_bytes) };
    let f64_of = |b: &[u8]| -> io::Result<f64> { u64_at(b, 0).map(f64::from_bits) };
    const M: u16 = PropertyType::MULTI;
    Ok(match kind {
        PropertyType::NULL => V::Null,
        PropertyType::STRING8 => V::String8(string8(b)),
        PropertyType::UNICODE => V::Unicode(utf16(b)),
        PropertyType::BINARY => V::Binary(Bytes(b.to_vec())),
        k if k == M | PropertyType::INT16 => V::MultipleInteger16(each(b, 2, |b| Ok(i16::from_le_bytes(bytes_at(b, 0)?)))?),
        k if k == M | PropertyType::INT32 => V::MultipleInteger32(each(b, 4, |b| Ok(i32::from_le_bytes(bytes_at(b, 0)?)))?),
        k if k == M | PropertyType::FLOAT32 => V::MultipleFloating32(each(b, 4, |b| Ok(f32::from_le_bytes(bytes_at(b, 0)?)))?),
        k if k == M | PropertyType::FLOAT64 => V::MultipleFloating64(each(b, 8, f64_of)?),
        k if k == M | PropertyType::CURRENCY => V::MultipleCurrency(each(b, 8, i64_of)?),
        k if k == M | PropertyType::APP_TIME => V::MultipleFloatingTime(each(b, 8, f64_of)?),
        k if k == M | PropertyType::INT64 => V::MultipleInteger64(each(b, 8, i64_of)?),
        k if k == M | PropertyType::TIME => V::MultipleTime(each(b, 8, i64_of)?),
        k if k == M | PropertyType::GUID => V::MultipleGuid(each(b, 16, GuidValue::parse)?),
        k if k == M | PropertyType::STRING8 => V::MultipleString8(pieces(b)?.into_iter().map(string8).collect()),
        k if k == M | PropertyType::UNICODE => V::MultipleUnicode(pieces(b)?.into_iter().map(utf16).collect()),
        k if k == M | PropertyType::BINARY => V::MultipleBinary(pieces(b)?.into_iter().map(|p| Bytes(p.to_vec())).collect()),
        k if PropertyType::kept_whole(k) => V::Other(k, Bytes(b.to_vec())),
        k => return Err(invalid(format!("a property of type {k:#x}, which none has"))),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_read_as_kept() {
        assert_eq!(parse(PropertyType::INT32, &7i32.to_le_bytes()).unwrap(), PropertyValue::Integer32(7));
        assert_eq!(parse(PropertyType::BOOLEAN, &[1, 0, 0, 0]).unwrap(), PropertyValue::Boolean(true));
        assert_eq!(parse(PropertyType::UNICODE, &[b'h', 0, b'i', 0]).unwrap(), PropertyValue::Unicode(Utf16(vec![104, 105])));
        assert_eq!(parse(PropertyType::BINARY, &[1, 2]).unwrap(), PropertyValue::Binary(Bytes(vec![1, 2])));
        assert_eq!(parse(PropertyType::OBJECT, &[5, 0, 0, 0, 9, 0, 0, 0]).unwrap(), PropertyValue::Object(ObjectRef { node: 5, size: 9 }));
    }

    #[test]
    fn a_list_of_strings_reads_by_its_offsets() {
        // Two strings: "a" and "bc".
        let mut b = Vec::new();
        b.extend(2u32.to_le_bytes());
        b.extend(12u32.to_le_bytes());
        b.extend(14u32.to_le_bytes());
        b.extend([b'a', 0, b'b', 0, b'c', 0]);
        let v = parse(PropertyType::MULTI | PropertyType::UNICODE, &b).unwrap();
        assert_eq!(v, PropertyValue::MultipleUnicode(vec![Utf16(vec![97]), Utf16(vec![98, 99])]));
    }

    #[test]
    fn a_short_or_lying_value_is_an_error() {
        assert!(parse(PropertyType::INT32, &[1, 2]).is_err());
        assert!(parse(PropertyType::GUID, &[0; 15]).is_err());
        assert!(parse(PropertyType::MULTI | PropertyType::UNICODE, &[0xFF, 0xFF, 0xFF, 0xFF]).is_err());
        assert!(parse(0x7777, &[]).is_err());
        let mut b = Vec::new();
        b.extend(1u32.to_le_bytes());
        b.extend(900u32.to_le_bytes());
        assert!(parse(PropertyType::MULTI | PropertyType::BINARY, &b).is_err());
    }
}
