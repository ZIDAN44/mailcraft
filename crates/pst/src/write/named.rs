//! Named properties: each one given is looked up in the file's name-to-ID map, and added to it
//! when it isn't there (MS-PST 2.4.7).

use std::collections::{BTreeMap, HashMap};
use std::io;

use super::edit::Editor;
use super::heap::build_props;
use super::types::*;
use crate::crc::crc;
use crate::ltp::{Bytes, GuidValue, PropertyValue};
use crate::messaging::{BUCKET_COUNT, NID_NAME_TO_ID_MAP, PS_MAPI, PS_PUBLIC_STRINGS, STREAM_ENTRY, STREAM_GUID, STREAM_STRING};
use crate::ndb::SubNodes;
use crate::util::{invalid, short};

const FIRST_BUCKET: u16 = 0x1000;
/// A map entry's index is at most this.
const MAX_PROP_INDEX: u16 = 0x7FFE;

/// A named property, as the map is searched for it.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub(crate) enum NameKey {
    Id([u8; 16], u32),
    String([u8; 16], Vec<u16>),
}

impl NameKey {
    pub(crate) fn of(set: &GuidValue, name: Name<'_>) -> NameKey {
        match name {
            Name::Id(id) => NameKey::Id(set.to_bytes(), id),
            Name::String(s) => NameKey::String(set.to_bytes(), s.encode_utf16().collect()),
        }
    }

    fn set(&self) -> [u8; 16] {
        match self {
            NameKey::Id(s, _) | NameKey::String(s, _) => *s,
        }
    }
}

/// The IDs named properties have in this file.
#[derive(Default)]
pub(crate) struct NamedIds {
    ids: HashMap<NameKey, u16>,
}

impl NamedIds {
    /// The ID a property given is written with.
    pub(crate) fn property_id(&self, key: &PropertyKey<'_>) -> io::Result<u16> {
        match key {
            PropertyKey::Tag(id) => Ok(*id),
            PropertyKey::Named(set, name) => {
                self.ids.get(&NameKey::of(set, *name)).copied().ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "a named property has no ID"))
            }
        }
    }
}

fn binary(props: &BTreeMap<u16, PropertyValue>, id: u16) -> io::Result<Vec<u8>> {
    match props.get(&id) {
        Some(PropertyValue::Binary(b)) => Ok(b.buffer().to_vec()),
        None => Ok(Vec::new()),
        Some(_) => Err(invalid("a name-to-ID map's stream isn't binary")),
    }
}

/// The name at `offset` of the string stream.
fn string_at(strings: &[u8], offset: u32) -> Option<Vec<u16>> {
    let start = usize::try_from(offset).ok()?;
    let size = usize::try_from(u32::from_le_bytes(strings.get(start..start.checked_add(4)?)?.try_into().ok()?)).ok()?;
    let from = start.checked_add(4)?;
    Some(strings.get(from..from.checked_add(size)?)?.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect())
}

fn guid_index(set: [u8; 16], guids: &mut Vec<u8>) -> io::Result<u16> {
    if set == PS_MAPI.to_bytes() {
        return Ok(1);
    }
    if set == PS_PUBLIC_STRINGS.to_bytes() {
        return Ok(2);
    }
    let index = match guids.as_chunks::<16>().0.iter().position(|g| *g == set) {
        Some(i) => i,
        None => {
            let i = guids.len() / 16;
            guids.extend_from_slice(&set);
            i
        }
    };
    u16::try_from(index)
        .ok()
        .and_then(|i| i.checked_add(3))
        .filter(|i| *i < 0x8000)
        .ok_or_else(|| invalid("the name-to-ID map has too many property sets"))
}

fn collect<'a>(messages: &mut Vec<&'a NewMessage<'a>>, attachments: &'a [NewAttachment<'a>]) {
    for a in attachments {
        if let AttachmentContent::Message(em) = &a.content {
            messages.push(&em.message);
            collect(messages, &em.attachments);
        }
    }
}

impl Editor {
    /// The ID of each named property `batch` gives; those the map lacks are added to it.
    pub(crate) fn resolve_named(&mut self, batch: &[BatchMessage<'_>]) -> io::Result<NamedIds> {
        let mut wanted: Vec<NameKey> = Vec::new();
        for input in batch {
            let mut messages = vec![&input.message];
            collect(&mut messages, input.attachments);
            for p in messages.iter().flat_map(|m| m.properties) {
                if let PropertyKey::Named(set, name) = p.key {
                    let key = NameKey::of(&set, name);
                    if !wanted.contains(&key) {
                        wanted.push(key);
                    }
                }
            }
        }
        let mut named = NamedIds::default();
        if wanted.is_empty() {
            return Ok(named);
        }
        let map = self.store.named_property_map()?;
        let mut props: BTreeMap<u16, PropertyValue> = map.properties().iter().map(|(k, v)| (*k, v.clone())).collect();
        let buckets = match props.get(&BUCKET_COUNT) {
            Some(PropertyValue::Integer32(n)) => u16::try_from(*n)
                .ok()
                .filter(|n| *n > 0 && *n <= u16::MAX - FIRST_BUCKET)
                .ok_or_else(|| invalid("the name-to-ID map's bucket count is out of range"))?,
            _ => return Err(invalid("the name-to-ID map has no bucket count")),
        };
        let mut guids = binary(&props, STREAM_GUID)?;
        let mut entries = binary(&props, STREAM_ENTRY)?;
        let mut strings = binary(&props, STREAM_STRING)?;

        let mut next_index = 0u16;
        for e in entries.as_chunks::<8>().0 {
            let id = u32::from_le_bytes([e[0], e[1], e[2], e[3]]);
            let guid = u16::from_le_bytes([e[4], e[5]]);
            let index = u16::from_le_bytes([e[6], e[7]]);
            next_index = next_index.max(index.saturating_add(1));
            let set = match guid >> 1 {
                0 => continue,
                1 => PS_MAPI.to_bytes(),
                2 => PS_PUBLIC_STRINGS.to_bytes(),
                n => {
                    let at = usize::from(n - 3) * 16;
                    match guids.get(at..at + 16).and_then(|s| <[u8; 16]>::try_from(s).ok()) {
                        Some(s) => s,
                        None => continue,
                    }
                }
            };
            let key = if guid & 1 == 0 {
                NameKey::Id(set, id)
            } else {
                match string_at(&strings, id) {
                    Some(n) => NameKey::String(set, n),
                    None => continue,
                }
            };
            if index <= MAX_PROP_INDEX && wanted.contains(&key) {
                named.ids.entry(key).or_insert(0x8000 + index);
            }
        }
        next_index = next_index.max(u16::try_from(entries.len() / 8).unwrap_or(u16::MAX));

        let mut added = false;
        let mut bucket_added: BTreeMap<u16, Vec<u8>> = BTreeMap::new();
        for key in wanted {
            if named.ids.contains_key(&key) {
                continue;
            }
            if next_index > MAX_PROP_INDEX {
                return Err(invalid("the name-to-ID map is full"));
            }
            let guid = guid_index(key.set(), &mut guids)?;
            let (id, kind, hash) = match &key {
                NameKey::Id(_, id) => (*id, 0u16, *id),
                NameKey::String(_, name) => {
                    let offset = u32::try_from(strings.len()).map_err(|_| short())?;
                    let bytes: Vec<u8> = name.iter().flat_map(|u| u.to_le_bytes()).collect();
                    strings.extend(u32::try_from(bytes.len()).map_err(|_| short())?.to_le_bytes());
                    strings.extend_from_slice(&bytes);
                    strings.resize(strings.len().next_multiple_of(4), 0);
                    (offset, 1, crc(&bytes))
                }
            };
            let field = (guid << 1) | kind;
            entries.extend(id.to_le_bytes());
            entries.extend(field.to_le_bytes());
            entries.extend(next_index.to_le_bytes());
            // A bucket's entry has the string's CRC where the offset would be (MS-PST 2.4.7.5).
            let bucket = FIRST_BUCKET + u16::try_from((hash ^ u32::from(field)) % u32::from(buckets)).map_err(|_| short())?;
            let record = bucket_added.entry(bucket).or_default();
            record.extend(hash.to_le_bytes());
            record.extend(field.to_le_bytes());
            record.extend(next_index.to_le_bytes());
            named.ids.insert(key, 0x8000 + next_index);
            next_index += 1;
            added = true;
        }
        if added {
            props.insert(STREAM_GUID, PropertyValue::Binary(Bytes(guids)));
            props.insert(STREAM_ENTRY, PropertyValue::Binary(Bytes(entries)));
            props.insert(STREAM_STRING, PropertyValue::Binary(Bytes(strings)));
            for (bucket, record) in bucket_added {
                let mut old = binary(&props, bucket)?;
                old.extend(record);
                props.insert(bucket, PropertyValue::Binary(Bytes(old)));
            }
            self.rewrite_props(NID_NAME_TO_ID_MAP, props.into_iter().collect())?;
        }
        Ok(named)
    }

    /// Node `nid`'s property set written again as `props`.
    pub(crate) fn rewrite_props(&mut self, nid: u32, props: Vec<(u16, PropertyValue)>) -> io::Result<()> {
        let entry = self.node(nid)?;
        let old = self.pst.subnodes(entry.sub)?;
        let built = build_props(self.format(), props, 0x80)?;
        let mut subs = SubNodes::new();
        for (node, bytes) in built.external {
            let bid = self.write_tree(&bytes)?;
            subs.insert(node, (bid, 0));
        }
        // Any other subnodes the node has stay.
        for (node, (data, sub)) in old.iter() {
            if subs.contains_key(node) || *node & crate::ndb::NID_TYPE_MASK == super::heap::NID_TYPE_INTERNAL {
                continue;
            }
            self.add_ref(*data);
            if *sub != 0 {
                self.add_ref(*sub);
            }
            subs.insert(*node, (*data, *sub));
        }
        let data = self.write_leaves(&built.blocks)?;
        let sub = self.write_subnodes(&subs)?;
        self.release_node_data(entry.data, entry.sub)?;
        self.set_node(nid, data, sub, entry.parent);
        Ok(())
    }
}
