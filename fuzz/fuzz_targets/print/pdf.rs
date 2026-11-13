//! A message printed: a picture read from any bytes (a JPEG's header, or decoded) and embedded,
//! and tables laid out on pages as the input says (their nesting, spans, widths, room and
//! lines; sizes that aren't numbers), with words, links and the picture in them.
#![no_main]

use libfuzzer_sys::fuzz_target;
use mailcraft_print as print;

/// The next byte of the input (0 once it's used up).
fn byte(data: &mut &[u8]) -> u8 {
    match data.split_first() {
        Some((b, rest)) => {
            *data = rest;
            *b
        }
        None => 0,
    }
}

/// A length: mostly small, now and then whatever four bytes say (not a number, endless, negative).
fn length(data: &mut &[u8]) -> f32 {
    match byte(data) {
        b if b % 16 == 0 => f32::from_bits(u32::from_le_bytes([byte(data), byte(data), byte(data), byte(data)])),
        b => f32::from(b) * 2.0,
    }
}

fn block(data: &mut &[u8], depth: usize) -> print::Block {
    let kind = byte(data);
    if kind % 3 == 0 || depth > 8 || data.is_empty() {
        let n = usize::from(byte(data) % 6);
        let runs = (0..n)
            .map(|_| {
                let flags = byte(data);
                let take = usize::from(byte(data) % 24).min(data.len());
                let (text, rest) = data.split_at(take);
                *data = rest;
                print::Run {
                    text: String::from_utf8_lossy(text).into_owned(),
                    bold: flags & 1 != 0,
                    italic: flags & 2 != 0,
                    underline: flags & 4 != 0,
                    strike: flags & 8 != 0,
                    size: (flags & 16 != 0).then(|| length(data)),
                    color: (flags & 32 != 0).then(|| [flags, 0, 255]),
                    background: (flags & 64 != 0).then(|| [0, flags, 9]),
                    on_background: flags & 128 != 0,
                    link: (flags % 5 == 0).then(|| format!("https://x.example/{flags}")),
                    picture: (flags % 7 == 0).then(|| (usize::from(flags % 3), length(data), length(data))),
                }
            })
            .collect();
        return print::Block::Text(vec![runs]);
    }
    let width = |data: &mut &[u8]| match byte(data) % 4 {
        0 => Some(print::Width::Pt(length(data))),
        1 => Some(print::Width::Percent(length(data))),
        _ => None,
    };
    let line = |data: &mut &[u8]| (byte(data) % 4 == 0).then(|| print::Line { width: length(data), color: [1, 2, 3] });
    let rows = (0..byte(data) % 4)
        .map(|_| {
            (0..byte(data) % 5)
                .map(|_| print::Cell {
                    blocks: (0..byte(data) % 3).map(|_| block(data, depth + 1)).collect(),
                    width: width(data),
                    background: (byte(data) % 2 == 0).then_some([200, 220, 240]),
                    align: [print::Align::Left, print::Align::Center, print::Align::Right][usize::from(byte(data) % 3)],
                    span: u16::from(byte(data) % 6),
                    padding: (byte(data) % 3 == 0).then(|| [length(data), length(data), length(data), length(data)]),
                    valign: [print::VAlign::Top, print::VAlign::Middle, print::VAlign::Bottom][usize::from(byte(data) % 3)],
                    height: (byte(data) % 3 == 0).then(|| length(data)),
                    radius: length(data),
                    borders: [line(data), line(data), line(data), line(data)],
                })
                .collect()
        })
        .collect();
    print::Block::Table(print::Table {
        rows,
        width: width(data),
        fit: byte(data) % 2 == 0,
        align: [print::Align::Left, print::Align::Center, print::Align::Right][usize::from(byte(data) % 3)],
        background: (byte(data) % 2 == 0).then_some([250, 240, 230]),
        spacing: length(data),
        padding: length(data),
        radius: length(data),
        border: line(data),
    })
}

fuzz_target!(|input: &[u8]| {
    // The picture, read from the input as it is.
    let pictures: Vec<print::Picture> = print::Picture::decode(input).into_iter().collect();
    let mut data = input;
    let blocks: Vec<print::Block> = (0..4).map(|_| block(&mut data, 0)).collect();
    let doc = print::Doc { title: "Fuzzed".into(), blocks, pictures, ..Default::default() };
    if let Ok(pdf) = print::pdf(&doc, None) {
        assert!(pdf.starts_with(b"%PDF-") && pdf.ends_with(b"%%EOF\n"));
    }
});
