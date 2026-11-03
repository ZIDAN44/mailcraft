//! Outlook message files (`.msg`, MS-OXMSG, issue #38): a compound file (MS-CFB) holding one
//! message's properties, its recipients' and its attachments', each attached message a storage
//! of its own. They're read into the item the `.pst` import builds from the same properties, and
//! so written out as a standard message (RFC 5322 / MIME) to show or to file in a folder.
//!
//! Like every file from outside, it is hostile input: its size, each stream's, the counts of
//! properties, recipients and attachments, and how deep messages nest are capped; nothing here
//! panics, and the compound file's reader is held behind a guard besides.

use std::borrow::Cow;
use std::io::{Cursor, Read};

use cfb::CompoundFile;

use crate::pst::{Loose, LooseFile, Val, loose_message};
use crate::pst_pim::Named;

/// The largest file read.
pub const MAX_FILE: usize = 64 << 20;
/// The most read of one stream (a body, an attachment's bytes).
const MAX_STREAM: u64 = 64 << 20;
/// The most read of all of a file's streams together.
const MAX_TOTAL: usize = 192 << 20;
const MAX_PROPS: usize = 4_000;
const MAX_RECIPIENTS: usize = 5_000;
const MAX_ATTACHMENTS: usize = 1_000;
const MAX_VALUES: usize = 1_000;
const MAX_NAMES: usize = 50_000;
/// Attached messages inside attached messages… stop here.
const MAX_DEPTH: usize = 16;

/// The property-stream header's size: the message's (MS-OXMSG §2.4.1.1), an attached message's
/// (§2.4.1.2), and a recipient's or attachment's (§2.4.2).
const TOP_HEADER: usize = 32;
const EMBEDDED_HEADER: usize = 24;
const PART_HEADER: usize = 8;

/// A property's value as stored: fixed-size ones in their entry, the rest in streams.
enum Stored {
    Fixed([u8; 8]),
    Bytes(Vec<u8>),
    Many(Vec<Vec<u8>>),
    Object,
}

struct Prop {
    id: u16,
    ty: u16,
    value: Stored,
}

/// One message of the file, read: its properties, recipients and files.
struct Read1 {
    props: Vec<Prop>,
    recipients: Vec<Vec<Prop>>,
    files: Vec<ReadFile>,
}

enum ReadFile {
    Data { name: String, mime: String, cid: String, data: Vec<u8> },
    Message(Box<Read1>),
}

/// Reads with a running total, so no file costs more than [`MAX_TOTAL`].
struct Reader<'a> {
    cf: CompoundFile<Cursor<&'a [u8]>>,
    used: usize,
}

impl Reader<'_> {
    fn stream(&mut self, path: &str) -> Option<Vec<u8>> {
        if !self.cf.is_stream(path) {
            return None;
        }
        let len = self.cf.entry(path).ok()?.len();
        let room = u64::try_from(MAX_TOTAL.saturating_sub(self.used)).unwrap_or(0);
        if len > MAX_STREAM || len > room {
            return None;
        }
        let mut s = self.cf.open_stream(path).ok()?;
        let mut out = Vec::with_capacity(usize::try_from(len).unwrap_or(0));
        s.by_ref().take(len).read_to_end(&mut out).ok()?;
        self.used = self.used.saturating_add(out.len());
        Some(out)
    }

    /// The names of the storages right inside `dir` starting with `prefix`, in order.
    fn storages(&self, dir: &str, prefix: &str, max: usize) -> Vec<String> {
        let Ok(entries) = self.cf.read_storage(if dir.is_empty() { "/" } else { dir }) else { return Vec::new() };
        let mut names: Vec<String> =
            entries.filter(|e| e.is_storage() && e.name().starts_with(prefix)).map(|e| e.name().to_string()).take(max).collect();
        names.sort();
        names
    }

    /// A storage's properties, after its stream's header of `header` bytes.
    fn props(&mut self, dir: &str, header: usize) -> Vec<Prop> {
        let Some(stream) = self.stream(&format!("{dir}/__properties_version1.0")) else { return Vec::new() };
        let mut out = Vec::new();
        for e in stream.get(header..).unwrap_or_default().as_chunks::<16>().0.iter().take(MAX_PROPS) {
            let tag = u32::from_le_bytes([e[0], e[1], e[2], e[3]]);
            let (id, ty) = ((tag >> 16) as u16, (tag & 0xFFFF) as u16);
            let mut fixed = [0u8; 8];
            fixed.copy_from_slice(&e[8..16]);
            let sub = format!("{dir}/__substg1.0_{tag:08X}");
            let value = match ty {
                0x000D => Stored::Object,
                0x001E | 0x001F | 0x0102 | 0x0048 => match self.stream(&sub) {
                    Some(b) => Stored::Bytes(b),
                    None => continue,
                },
                // Several strings or byte strings: one stream each (MS-OXMSG §2.1.4.2.2).
                0x101E | 0x101F | 0x1102 => {
                    let per = if ty == 0x1102 { 8 } else { 4 };
                    let count = self.stream(&sub).map_or(0, |l| l.len() / per).min(MAX_VALUES);
                    Stored::Many((0..count).filter_map(|i| self.stream(&format!("{sub}-{i:08X}"))).collect())
                }
                _ => Stored::Fixed(fixed),
            };
            out.push(Prop { id, ty, value });
        }
        out
    }

    /// The message in storage `dir` (`""`: the file's own), its attached messages read in turn.
    fn message(&mut self, dir: &str, header: usize, depth: usize) -> Read1 {
        let props = self.props(dir, header);
        let recipients =
            self.storages(dir, "__recip_version1.0_#", MAX_RECIPIENTS).into_iter().map(|r| self.props(&format!("{dir}/{r}"), PART_HEADER)).collect();
        let mut files = Vec::new();
        for a in self.storages(dir, "__attach_version1.0_#", MAX_ATTACHMENTS) {
            let at = format!("{dir}/{a}");
            let p = self.props(&at, PART_HEADER);
            let text = |id: u16| p.iter().find(|x| x.id == id).and_then(|x| text_value(x, 0));
            let name = text(0x3707).or_else(|| text(0x3704)).or_else(|| text(0x3001)).unwrap_or_default();
            let mime = text(0x370E).unwrap_or_default();
            let cid = text(0x3712).unwrap_or_default();
            let inner = format!("{at}/__substg1.0_3701000D");
            // An attached message (ATTACH_EMBEDDED_MSG); an OLE object's storage isn't one.
            let method = p.iter().find(|x| x.id == 0x3705).map_or(0, |x| match &x.value {
                Stored::Fixed(f) => u32::from_le_bytes([f[0], f[1], f[2], f[3]]),
                _ => 0,
            });
            if method != 5 && self.cf.is_storage(&inner) {
                continue;
            }
            if self.cf.is_storage(&inner) {
                if depth < MAX_DEPTH {
                    files.push(ReadFile::Message(Box::new(self.message(&inner, EMBEDDED_HEADER, depth + 1))));
                }
                continue;
            }
            if let Some(Stored::Bytes(data)) = p.into_iter().find(|x| x.id == 0x3701 && x.ty == 0x0102).map(|x| x.value) {
                files.push(ReadFile::Data { name, mime, cid, data });
            }
        }
        Read1 { props, recipients, files }
    }

    /// The file's named properties (MS-OXMSG §2.2.3).
    fn names(&mut self) -> Named {
        let dir = "/__nameid_version1.0";
        let guids = self.stream(&format!("{dir}/__substg1.0_00020102")).unwrap_or_default();
        let entries = self.stream(&format!("{dir}/__substg1.0_00030102")).unwrap_or_default();
        let strings = self.stream(&format!("{dir}/__substg1.0_00040102")).unwrap_or_default();
        let guid = |i: usize| -> Option<(u32, u16, u16, [u8; 8])> {
            let g = guids.get(i.checked_mul(16)?..i.checked_mul(16)?.checked_add(16)?)?;
            let mut d4 = [0u8; 8];
            d4.copy_from_slice(g.get(8..16)?);
            Some((u32::from_le_bytes([g[0], g[1], g[2], g[3]]), u16::from_le_bytes([g[4], g[5]]), u16::from_le_bytes([g[6], g[7]]), d4))
        };
        let name_at = |at: usize| -> Option<String> {
            let len = usize::try_from(u32::from_le_bytes(strings.get(at..at.checked_add(4)?)?.try_into().ok()?)).ok()?;
            let bytes = strings.get(at.checked_add(4)?..at.checked_add(4)?.checked_add(len)?)?;
            let units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
            Some(String::from_utf16_lossy(&units))
        };
        let list = entries.as_chunks::<8>().0.iter().take(MAX_NAMES).filter_map(|e| {
            let first = u32::from_le_bytes([e[0], e[1], e[2], e[3]]);
            let kind_index = u16::from_le_bytes([e[4], e[5]]);
            let index = u16::from_le_bytes([e[6], e[7]]);
            let id = 0x8000u16.checked_add(index)?;
            let set = match kind_index >> 1 {
                1 => None,
                2 => Some((0, 0, 0, [0; 8])),
                n => Some(guid(usize::from(n).checked_sub(3)?)?),
            };
            let name = if kind_index & 1 == 1 { Err(name_at(usize::try_from(first).ok()?)?) } else { Ok(first) };
            Some((set, name, id))
        });
        Named::of_names(list.collect::<Vec<_>>().into_iter())
    }
}

/// A property's text: a string's (8-bit ones in `code_page`).
fn text_value(p: &Prop, code_page: u32) -> Option<String> {
    let Stored::Bytes(b) = &p.value else { return None };
    let s = match p.ty {
        0x001F => utf16(b),
        0x001E => eight_bit(b, code_page),
        _ => return None,
    };
    Some(s.trim_end_matches('\0').to_string())
}

fn utf16(b: &[u8]) -> String {
    let units: Vec<u16> = b.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
    String::from_utf16_lossy(&units)
}

fn eight_bit(b: &[u8], code_page: u32) -> String {
    if code_page == 0 || code_page == 65001 {
        return String::from_utf8_lossy(b).into_owned();
    }
    mailcraft_mime::decode_code_page(b, code_page)
}

/// A property as the import reads one (its types as the `.pst` reader gives them).
fn val(p: &Prop, code_page: u32) -> Val<'_> {
    let int = |f: &[u8; 8]| i64::from(i32::from_le_bytes([f[0], f[1], f[2], f[3]]));
    match (&p.value, p.ty) {
        (Stored::Bytes(_), 0x001E | 0x001F) => text_value(p, code_page).map_or(Val::Other, |t| Val::Text(Cow::Owned(t))),
        (Stored::Bytes(b), 0x0102) => Val::Bin(b),
        (Stored::Fixed(f), 0x0003) => Val::Int(int(f)),
        (Stored::Fixed(f), 0x000B) => Val::Bool(f[0] != 0),
        (Stored::Fixed(f), 0x0040) => Val::Time(i64::from_le_bytes(*f)),
        (Stored::Fixed(f), 0x0005) => Val::Float(u64::from_le_bytes(*f)),
        (Stored::Many(list), 0x101F) => Val::Texts(list.iter().map(|b| utf16(b).trim_end_matches('\0').to_string()).collect()),
        (Stored::Many(list), 0x101E) => Val::Texts(list.iter().map(|b| eight_bit(b, code_page).trim_end_matches('\0').to_string()).collect()),
        (Stored::Many(list), 0x1102) => Val::Bins(list.iter().map(Vec::as_slice).collect()),
        (Stored::Object, _) => Val::Object,
        _ => Val::Other,
    }
}

/// The message's code page (PidTagMessageCodepage), for its 8-bit strings.
fn code_page(props: &[Prop]) -> u32 {
    match props.iter().find(|p| p.id == 0x3FFD).map(|p| &p.value) {
        Some(Stored::Fixed(f)) => u32::from_le_bytes([f[0], f[1], f[2], f[3]]),
        _ => 0,
    }
}

fn vals(ps: &[Prop], cp: u32) -> Vec<(u16, Val<'_>)> {
    ps.iter().map(|p| (p.id, val(p, cp))).collect()
}

fn loose(m: &Read1) -> Loose<'_> {
    let cp = code_page(&m.props);
    Loose {
        props: vals(&m.props, cp),
        recipients: m.recipients.iter().map(|r| vals(r, cp)).collect(),
        files: m
            .files
            .iter()
            .map(|f| match f {
                ReadFile::Data { name, mime, cid, data } => {
                    LooseFile::Data { name: name.clone(), mime: mime.clone(), cid: cid.clone(), data: data.clone() }
                }
                ReadFile::Message(inner) => LooseFile::Message(Box::new(loose(inner))),
            })
            .collect(),
    }
}

/// Whether `bytes` are a compound file (a `.msg`, or an older Office document).
pub fn is_compound_file(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1])
}

/// An Outlook message file as a standard message.
pub fn to_mime(bytes: &[u8]) -> Result<Vec<u8>, String> {
    if bytes.len() > MAX_FILE {
        return Err("the file is too big to be a message".into());
    }
    if !is_compound_file(bytes) {
        return Err("not an Outlook message file".into());
    }
    let read = std::panic::catch_unwind(|| -> Result<(Read1, Named), String> {
        let cf = CompoundFile::open(Cursor::new(bytes)).map_err(|e| format!("not an Outlook message file: {e}"))?;
        let mut r = Reader { cf, used: 0 };
        let names = r.names();
        Ok((r.message("", TOP_HEADER, 0), names))
    });
    let (m, named) = read.map_err(|_| "the file is damaged".to_string())??;
    if m.props.is_empty() {
        return Err("the file holds no message".into());
    }
    loose_message(&loose(&m), &named)
}

#[cfg(test)]
pub(crate) mod tests;
