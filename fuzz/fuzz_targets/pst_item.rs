//! A mail item from an Outlook data file, rebuilt as a message from its properties alone: the
//! values a damaged or hostile file can hold, without having to get past a `.pst`'s structure.
//!
//! The input is a list of properties, each a little-endian id (two bytes), a kind (text, bytes,
//! number, yes/no, time, UTF-16 text, texts; 5 starts the next recipient's properties), a little-endian
//! length (two bytes) and the value. The headers the import adds from the properties
//! (Thread-Index, Thread-Topic, Sensitivity) must each stay on lines of their own: ASCII, no
//! control characters, at most 998 bytes.
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
    let mut ours = false;
    for line in head.split("\r\n") {
        if !line.starts_with([' ', '\t']) {
            let name = line.split(':').next().unwrap_or_default().to_ascii_lowercase();
            // An original header (0x007D) keeps its own lines as they were.
            ours = ["thread-index", "thread-topic", "sensitivity"].contains(&name.as_str()) && !item.iter().any(|(id, _)| *id == 0x007D);
        }
        if ours {
            assert!(line.len() <= 998 && line.chars().all(|c| c == '\t' || (c.is_ascii() && !c.is_ascii_control())), "{line:?}");
        }
    }
    let _ = mime::parse(&raw);
});
