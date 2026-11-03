//! A mail item from an Outlook data file, rebuilt as a message from its properties alone: the
//! values a damaged or hostile file can hold, without having to get past a `.pst`'s structure.
//!
//! The input is a list of properties, each a little-endian id (two bytes), a kind (text, bytes,
//! number, yes/no, time, UTF-16 text, texts; 5 starts the next recipient's properties), a little-endian
//! length (two bytes) and the value; 0x3701 is the item's one file (an S/MIME item's content),
//! 0x370E its type; or, with 0x3705 saying a link (2, 3, 4, 7), the link its pathnames (0x370D,
//! 0x3708) name, kept as a short text file (nothing is read). 0x8000 is the categories, and 0x8001
//! to 0x8006 a sharing invitation's properties (what it shares, of what kind, where, by whom).
//! Every header line the import writes (all but an original
//! header's own) must stay a line of its own: no control characters, at most 998 bytes; those
//! it adds from the properties (Thread-Index, Thread-Topic, Sensitivity) ASCII too.
#![no_main]

use std::borrow::Cow;

use libfuzzer_sys::fuzz_target;
use mailcraft_mime as mime;
use mailcraft_net::pst::{self, Val};

fn props(mut data: &[u8]) -> (Vec<(u16, Val<'_>)>, Vec<Vec<(u16, Val<'_>)>>) {
    let mut item = Vec::new();
    let mut recipients: Vec<Vec<(u16, Val<'_>)>> = Vec::new();
    while let [a, b, kind, l1, l2, rest @ ..] = data {
        let len = usize::from(u16::from_le_bytes([*l1, *l2])).min(rest.len());
        let (value, after) = rest.split_at(len);
        data = after;
        let id = u16::from_le_bytes([*a, *b]);
        let number = |n: usize| value.iter().take(n).rev().fold(0i64, |acc, b| (acc << 8) | i64::from(*b));
        let v = match kind {
            0 => Val::Text(String::from_utf8_lossy(value)),
            1 => Val::Bin(value),
            2 => Val::Int(i64::from(number(4) as i32)),
            3 => Val::Bool(value.first().is_some_and(|b| *b != 0)),
            4 => Val::Time(number(8)),
            5 => {
                recipients.push(Vec::new());
                continue;
            }
            6 => Val::Text(Cow::Owned(String::from_utf16_lossy(&value.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect::<Vec<_>>()))),
            // A list of texts, NUL between them (property 0x8000 is the categories).
            7 => Val::Texts(value.split(|b| *b == 0).map(|t| String::from_utf8_lossy(t).into_owned()).collect()),
            _ => Val::Other,
        };
        match recipients.last_mut() {
            Some(r) => r.push((id, v)),
            None => item.push((id, v)),
        }
    }
    (item, recipients)
}

fuzz_target!(|data: &[u8]| {
    let (item, recipients) = props(data);
    let Ok((raw, summary)) = pst::rebuilt(&item, &recipients) else { return };
    assert!((-1..=1).contains(&summary.importance));
    assert!(summary.flags.keywords.len() <= 64 && summary.flags.keywords.iter().all(|k| mailcraft_model::is_keyword(k)));
    let end = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap_or(raw.len());
    let head = String::from_utf8_lossy(&raw[..end]);
    // An original header (0x007D, read as the import reads it: its first value, when it has a
    // colon) keeps its own lines as they were: a line named as one of them is left alone.
    let name_of = |line: &str| line.split(':').next().unwrap_or_default().trim().to_ascii_lowercase();
    let original: Vec<String> = match item.iter().find(|(id, _)| *id == 0x007D) {
        Some((_, Val::Text(t))) if t.trim().contains(':') => {
            t.trim().lines().filter(|l| !l.starts_with([' ', '\t'])).map(|l| name_of(l.trim_end_matches('\r'))).collect()
        }
        _ => Vec::new(),
    };
    let (mut ours, mut added) = (false, false);
    for line in head.split("\r\n") {
        if !line.starts_with([' ', '\t']) {
            let name = name_of(line);
            ours = !original.contains(&name);
            added = ours && ["thread-index", "thread-topic", "sensitivity"].contains(&name.as_str());
        }
        if ours {
            assert!(line.len() <= 998 && line.chars().all(|c| c == '\t' || !c.is_control()), "{line:?}");
        }
        if added {
            assert!(line.is_ascii(), "{line:?}");
        }
    }
    let _ = mime::parse(&raw);
});
