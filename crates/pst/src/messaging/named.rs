//! The name-to-ID map (MS-PST 2.4.7): which ID a named property has in this file.

use std::io;

use crate::ltp::{GuidValue, Properties, PropertyValue};
use crate::util::{invalid, u16_at, u32_at};

const OLE: [u8; 8] = [0xC0, 0, 0, 0, 0, 0, 0, 0x46];
/// The two property sets a map has no GUID for.
pub const PS_MAPI: GuidValue = GuidValue::new(0x0002_0328, 0, 0, OLE);
pub const PS_PUBLIC_STRINGS: GuidValue = GuidValue::new(0x0002_0329, 0, 0, OLE);

/// `PidTagNameidBucketCount`, the GUID, entry and string streams.
pub(crate) const BUCKET_COUNT: u16 = 0x0001;
pub(crate) const STREAM_GUID: u16 = 0x0002;
pub(crate) const STREAM_ENTRY: u16 = 0x0003;
pub(crate) const STREAM_STRING: u16 = 0x0004;

/// Which property set an entry is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NamedPropertyGuid {
    None,
    Mapi,
    PublicStrings,
    /// An index in the map's GUID stream.
    GuidIndex(u16),
}

/// What an entry names: a number, or a string (where it is in the string stream).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NamedPropertyId {
    Number(u32),
    StringOffset(u32),
}

/// One name's entry (`NAMEID`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NamedEntry {
    id: NamedPropertyId,
    guid: NamedPropertyGuid,
    index: u16,
}

impl NamedEntry {
    fn parse(b: &[u8]) -> io::Result<NamedEntry> {
        let (value, guid, index) = (u32_at(b, 0)?, u16_at(b, 4)?, u16_at(b, 6)?);
        if index >= 0x8000 {
            return Err(invalid("a named property's index past those there are"));
        }
        let id = if guid & 1 == 0 { NamedPropertyId::Number(value) } else { NamedPropertyId::StringOffset(value) };
        let guid = match guid >> 1 {
            0 => NamedPropertyGuid::None,
            1 => NamedPropertyGuid::Mapi,
            2 => NamedPropertyGuid::PublicStrings,
            n => NamedPropertyGuid::GuidIndex(n - 3),
        };
        Ok(NamedEntry { id, guid, index })
    }

    pub fn id(&self) -> NamedPropertyId {
        self.id
    }

    pub fn guid(&self) -> NamedPropertyGuid {
        self.guid
    }

    /// The ID the named property has in this file.
    pub fn prop_id(&self) -> u16 {
        0x8000 + self.index
    }
}

/// A file's name-to-ID map.
pub struct NamedMap {
    props: Properties,
}

impl NamedMap {
    pub(crate) fn new(props: Properties) -> NamedMap {
        NamedMap { props }
    }

    pub fn properties(&self) -> &Properties {
        &self.props
    }

    fn binary(&self, id: u16) -> &[u8] {
        match self.props.get(id) {
            Some(PropertyValue::Binary(b)) => b.buffer(),
            _ => &[],
        }
    }

    /// The property sets entries name by index.
    pub fn stream_guid(&self) -> Vec<GuidValue> {
        self.binary(STREAM_GUID).as_chunks::<16>().0.iter().filter_map(|c| GuidValue::parse(c).ok()).collect()
    }

    /// Every entry, as many as read.
    pub fn stream_entry(&self) -> Vec<NamedEntry> {
        self.binary(STREAM_ENTRY).as_chunks::<8>().0.iter().filter_map(|c| NamedEntry::parse(c).ok()).collect()
    }

    /// The bytes string names are in.
    pub fn stream_string(&self) -> &[u8] {
        self.binary(STREAM_STRING)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One entry that can't be read costs that entry, not those after it.
    #[test]
    fn a_bad_entry_is_passed_over() {
        let entry = |index: u16| [7u32.to_le_bytes().as_slice(), 2u16.to_le_bytes().as_slice(), index.to_le_bytes().as_slice()].concat();
        let stream = [entry(1), entry(0x9000), entry(2)].concat();
        let map = NamedMap::new(Properties([(STREAM_ENTRY, PropertyValue::Binary(crate::ltp::Bytes(stream)))].into_iter().collect()));
        assert_eq!(map.stream_entry().iter().map(NamedEntry::prop_id).collect::<Vec<_>>(), vec![0x8001, 0x8002]);
    }

    #[test]
    fn an_entry_says_its_set_its_name_and_its_id() {
        let mut b = 0x8102u32.to_le_bytes().to_vec();
        b.extend(((5u16) << 1).to_le_bytes());
        b.extend(3u16.to_le_bytes());
        let e = NamedEntry::parse(&b).unwrap();
        assert_eq!((e.id(), e.guid(), e.prop_id()), (NamedPropertyId::Number(0x8102), NamedPropertyGuid::GuidIndex(2), 0x8003));
        let mut b = 8u32.to_le_bytes().to_vec();
        b.extend(((2u16 << 1) | 1).to_le_bytes());
        b.extend(0u16.to_le_bytes());
        let e = NamedEntry::parse(&b).unwrap();
        assert_eq!((e.id(), e.guid(), e.prop_id()), (NamedPropertyId::StringOffset(8), NamedPropertyGuid::PublicStrings, 0x8000));
    }
}
