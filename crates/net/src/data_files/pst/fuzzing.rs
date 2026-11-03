//! The import's pieces as the fuzz targets reach them: an item rebuilt from hostile property
//! values, a repeating appointment's or task's pattern, and bytes read as text.

use super::*;

/// A mail item rebuilt as a message from its properties and its recipients' alone, as the
/// import writes it, with its summary: for the fuzz target, which hands it hostile values. Its
/// categories are property 0x8000; its one file, when it has one, the bytes of 0x3701 (an
/// attachment's data), of the type 0x370E says, or, when 0x3705 says a link, the link its
/// pathnames (0x370D, 0x3708) name, as one not on this computer (nothing is read).
pub fn rebuilt<'a>(props: &[(u16, Val<'a>)], recipients: &[Vec<(u16, Val<'a>)>]) -> Result<(Vec<u8>, mailcraft_model::Summary), String> {
    fn find<'a>(list: &[(u16, Val<'a>)], id: u16) -> Option<Val<'a>> {
        list.iter().find(|(i, _)| *i == id).map(|(_, v)| v.clone())
    }
    let rows: Vec<_> = recipients.iter().map(|r| move |id: u16| find(r, id)).collect();
    let get = |id: u16| find(props, id);
    let files = match find(props, 0x3701) {
        Some(Val::Bin(data)) => {
            let mime = find(props, 0x370E).and_then(Val::into_text).unwrap_or_default();
            vec![File::Data { name: "smime.p7m".into(), mime, cid: String::new(), data: data.to_vec() }]
        }
        _ if item::link(&get) => match item::linked(&get, &|_, _| None) {
            item::Got::Link(f) | item::Got::File(f) => vec![f],
            _ => Vec::new(),
        },
        _ => Vec::new(),
    };
    let it = item::item_of(&|id| find(props, id), &rows, files, &crate::pst_pim::Named::for_fuzzing());
    let (raw, glance) = rebuild::message(it, "fuzz@mailcraft.invalid")?;
    let key = MessageKey { account: "f1".into(), folder: "INBOX".into(), validity: 1, uid: 1 };
    let summary = rebuild::summary_of(&raw, glance, key, u32::try_from(raw.len()).unwrap_or(u32::MAX));
    Ok((raw, summary))
}

/// A repeating appointment's pattern (PidLidAppointmentRecur) as the import reads it, its first
/// occurrence starting at `start` (Unix seconds): the series as an event, and its changed
/// occurrences as events of their own. For the fuzz target.
pub fn repeating(pattern: &[u8], start: i64) -> Vec<mailcraft_model::ical::CalEvent> {
    let x = crate::pst_pim::Extra { start: Some(start), end: Some(start.saturating_add(3600)), recurrence: pattern.to_vec(), ..Default::default() };
    let from = EmailAddress::default();
    let series = crate::pst_pim::event_of(&x, "Series", "", &from, &[], "u".into());
    let changed = crate::pst_pim::changed_events(&x, "Series", "", &from, &[], "u", &[]);
    series.into_iter().chain(changed.into_iter().map(|(e, _)| e)).collect()
}
pub use crate::pst_pim::repeating_task;

/// Bytes from a data file as the import reads them as text: an HTML body (`html`) in its internet
/// code page, or an 8-bit string in its message's. For the fuzz target.
pub fn text_from_bytes(bytes: &[u8], code_page: u32, html: bool) -> String {
    props::decode(bytes, code_page, html)
}
