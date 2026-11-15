use std::io::Write;

use super::*;
use crate::data_files::test_data::filetime;

/// A property of a message file being built: its tag, and its value as the file keeps it.
enum P {
    Text(&'static str),
    Bin(Vec<u8>),
    Int(i32),
    Time(i64),
}

fn utf16le(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

/// Write one storage's properties: the stream with `header` bytes before the entries, and a
/// stream of its own for each variable-size value.
fn write_props(cf: &mut CompoundFile<Cursor<Vec<u8>>>, dir: &str, header: usize, props: &[(u32, P)]) {
    let mut entries = vec![0u8; header];
    for (tag, v) in props {
        entries.extend(tag.to_le_bytes());
        entries.extend(6u32.to_le_bytes());
        let mut value = [0u8; 8];
        let data = match v {
            P::Text(s) => Some(utf16le(s)),
            P::Bin(b) => Some(b.clone()),
            P::Int(n) => {
                value[..4].copy_from_slice(&n.to_le_bytes());
                None
            }
            P::Time(t) => {
                value.copy_from_slice(&t.to_le_bytes());
                None
            }
        };
        if let Some(d) = data {
            value[..4].copy_from_slice(&u32::try_from(d.len()).unwrap().to_le_bytes());
            cf.create_stream(format!("{dir}/__substg1.0_{tag:08X}")).unwrap().write_all(&d).unwrap();
        }
        entries.extend(value);
    }
    cf.create_stream(format!("{dir}/__properties_version1.0")).unwrap().write_all(&entries).unwrap();
}

/// 2026-11-03 10:00 UTC as a FILETIME.
const FILETIME: i64 = filetime(1_793_700_000);

/// A message as Outlook saves one: text and HTML, a sender, two recipients, a file and an
/// attached message with a file of its own.
pub(crate) fn sample() -> Vec<u8> {
    let mut cf = CompoundFile::create(Cursor::new(Vec::new())).unwrap();
    write_props(
        &mut cf,
        "",
        TOP_HEADER,
        &[
            (0x001A_001F, P::Text("IPM.Note")),
            (0x0037_001F, P::Text("Quarterly figures — räder")),
            (0x1000_001F, P::Text("Hello Ana,\r\nthe figures are attached.\r\n")),
            (0x1013_0102, P::Bin(b"<html><body><p>Hello Ana,</p><p>the <b>figures</b> are attached.</p></body></html>".to_vec())),
            (0x0C1A_001F, P::Text("Sam Lee")),
            (0x5D01_001F, P::Text("sam@studio.test")),
            (0x0042_001F, P::Text("Sam Lee")),
            (0x0065_001F, P::Text("sam@studio.test")),
            (0x1035_001F, P::Text("<figures-1@studio.test>")),
            (0x0039_0040, P::Time(FILETIME)),
            (0x0E06_0040, P::Time(FILETIME)),
            (0x3FFD_0003, P::Int(65001)),
        ],
    );
    for (i, (name, email, kind)) in [("Ana Lima", "ana@studio.test", 1), ("Ben Ode", "ben@studio.test", 2)].into_iter().enumerate() {
        let dir = format!("/__recip_version1.0_#{i:08X}");
        cf.create_storage(&dir).unwrap();
        write_props(&mut cf, &dir, PART_HEADER, &[(0x3001_001F, P::Text(name)), (0x39FE_001F, P::Text(email)), (0x0C15_0003, P::Int(kind))]);
    }
    let file = "/__attach_version1.0_#00000000";
    cf.create_storage(file).unwrap();
    write_props(
        &mut cf,
        file,
        PART_HEADER,
        &[
            (0x3707_001F, P::Text("figures.csv")),
            (0x370E_001F, P::Text("text/csv")),
            (0x3705_0003, P::Int(1)),
            (0x3701_0102, P::Bin(b"quarter,total\nQ3,42\n".to_vec())),
        ],
    );
    let attached = "/__attach_version1.0_#00000001";
    cf.create_storage(attached).unwrap();
    write_props(&mut cf, attached, PART_HEADER, &[(0x3001_001F, P::Text("Earlier message")), (0x3705_0003, P::Int(5))]);
    let inner = format!("{attached}/__substg1.0_3701000D");
    cf.create_storage(&inner).unwrap();
    write_props(
        &mut cf,
        &inner,
        EMBEDDED_HEADER,
        &[
            (0x001A_001F, P::Text("IPM.Note")),
            (0x0037_001F, P::Text("Earlier figures")),
            (0x1000_001F, P::Text("Last quarter's.")),
            (0x0C1A_001F, P::Text("Ana Lima")),
            (0x5D01_001F, P::Text("ana@studio.test")),
        ],
    );
    let inner_file = format!("{inner}/__attach_version1.0_#00000000");
    cf.create_storage(&inner_file).unwrap();
    write_props(
        &mut cf,
        &inner_file,
        PART_HEADER,
        &[(0x3707_001F, P::Text("old.txt")), (0x3705_0003, P::Int(1)), (0x3701_0102, P::Bin(b"old".to_vec()))],
    );
    cf.flush().unwrap();
    cf.into_inner().into_inner()
}

/// Issue #38: a message saved from Outlook reads as the message it is: its subject, sender,
/// recipients (To and Cc), date, text and formatted text, its file, and the message attached
/// to it with that one's own file.
#[test]
fn an_outlook_message_file_reads_whole() {
    let raw = to_mime(&sample()).unwrap();
    let p = mailcraft_mime::parse(&raw).unwrap();
    assert_eq!(p.subject, "Quarterly figures — räder");
    assert_eq!((p.from.name.as_str(), p.from.email.as_str()), ("Sam Lee", "sam@studio.test"));
    assert_eq!(p.to.iter().map(|a| a.email.as_str()).collect::<Vec<_>>(), ["ana@studio.test"]);
    assert_eq!(p.cc.iter().map(|a| a.email.as_str()).collect::<Vec<_>>(), ["ben@studio.test"]);
    assert_eq!(p.message_id, "figures-1@studio.test");
    assert_eq!(p.date, 1_793_700_000);
    assert!(p.had_html && p.text.contains("figures"), "{}", p.text);
    let names: Vec<&str> = p.attachments.iter().map(|a| a.name.as_str()).collect();
    assert!(names.contains(&"figures.csv"), "{names:?}");
    let at = |pred: &dyn Fn(&mailcraft_mime::AttachmentInfo) -> bool| p.attachments.iter().position(pred).unwrap();
    let (msg, csv) = (at(&|a| a.mime == "message/rfc822"), at(&|a| a.name == "figures.csv"));
    let files = mailcraft_mime::attachments_data(&raw, &[msg, csv], 1 << 20, 1 << 20);
    let data = |i: usize| files.iter().find(|(j, _)| *j == i).map(|(_, d)| d.clone()).unwrap();
    assert_eq!(data(csv), b"quarter,total\nQ3,42\n");
    let ip = mailcraft_mime::parse(&data(msg)).unwrap();
    assert_eq!((ip.subject.as_str(), ip.from.email.as_str()), ("Earlier figures", "ana@studio.test"));
    assert!(ip.attachments.iter().any(|a| a.name == "old.txt"), "{:?}", ip.attachments);
}

/// An OLE object attached (a storage of its own, as an attached message is) isn't taken for an
/// attached message.
#[test]
fn an_ole_object_isnt_an_attached_message() {
    let mut cf = CompoundFile::create(Cursor::new(Vec::new())).unwrap();
    write_props(
        &mut cf,
        "",
        TOP_HEADER,
        &[(0x001A_001F, P::Text("IPM.Note")), (0x0037_001F, P::Text("With a chart")), (0x1000_001F, P::Text("See the chart."))],
    );
    let at = "/__attach_version1.0_#00000000";
    cf.create_storage(at).unwrap();
    write_props(&mut cf, at, PART_HEADER, &[(0x3001_001F, P::Text("Chart")), (0x3705_0003, P::Int(6))]);
    let inner = format!("{at}/__substg1.0_3701000D");
    cf.create_storage(&inner).unwrap();
    cf.create_stream(format!("{inner}/CONTENTS")).unwrap().write_all(b"not a message").unwrap();
    cf.flush().unwrap();
    let raw = to_mime(&cf.into_inner().into_inner()).unwrap();
    let p = mailcraft_mime::parse(&raw).unwrap();
    assert_eq!(p.subject, "With a chart");
    assert!(p.attachments.iter().all(|a| a.mime != "message/rfc822"), "{:?}", p.attachments);
}

/// A message saved in a code page (8-bit strings, not Unicode) names its files in it too: a
/// file's name was read as UTF-8, its letters lost, while the subject beside it read right. An
/// attached message names its own in its own code page.
#[test]
fn eight_bit_file_names_are_read_in_the_messages_code_page() {
    let mut cf = CompoundFile::create(Cursor::new(Vec::new())).unwrap();
    write_props(
        &mut cf,
        "",
        TOP_HEADER,
        &[
            (0x001A_001E, P::Bin(b"IPM.Note".to_vec())),
            (0x0037_001E, P::Bin(b"Gr\xfc\xdfe".to_vec())),
            (0x1000_001E, P::Bin(b"Anbei.".to_vec())),
            (0x3FFD_0003, P::Int(1252)),
        ],
    );
    let file = "/__attach_version1.0_#00000000";
    cf.create_storage(file).unwrap();
    write_props(
        &mut cf,
        file,
        PART_HEADER,
        &[(0x3707_001E, P::Bin(b"R\xe4der.csv".to_vec())), (0x3705_0003, P::Int(1)), (0x3701_0102, P::Bin(b"a,b\n".to_vec()))],
    );
    let attached = "/__attach_version1.0_#00000001";
    cf.create_storage(attached).unwrap();
    write_props(&mut cf, attached, PART_HEADER, &[(0x3705_0003, P::Int(5))]);
    let inner = format!("{attached}/__substg1.0_3701000D");
    cf.create_storage(&inner).unwrap();
    write_props(
        &mut cf,
        &inner,
        EMBEDDED_HEADER,
        &[(0x001A_001E, P::Bin(b"IPM.Note".to_vec())), (0x0037_001E, P::Bin(b"\xce\xf2\xf7\xe5\xf2".to_vec())), (0x3FFD_0003, P::Int(1251))],
    );
    let inner_file = format!("{inner}/__attach_version1.0_#00000000");
    cf.create_storage(&inner_file).unwrap();
    write_props(
        &mut cf,
        &inner_file,
        PART_HEADER,
        &[(0x3707_001E, P::Bin(b"\xce\xf2\xf7\xe5\xf2.txt".to_vec())), (0x3705_0003, P::Int(1)), (0x3701_0102, P::Bin(b"x".to_vec()))],
    );
    cf.flush().unwrap();
    let raw = to_mime(&cf.into_inner().into_inner()).unwrap();
    let p = mailcraft_mime::parse(&raw).unwrap();
    assert_eq!(p.subject, "Grüße");
    assert!(p.attachments.iter().any(|a| a.name == "Räder.csv"), "{:?}", p.attachments);
    let msg = p.attachments.iter().position(|a| a.mime == "message/rfc822").unwrap();
    let files = mailcraft_mime::attachments_data(&raw, &[msg], 1 << 20, 1 << 20);
    let ip = mailcraft_mime::parse(&files[0].1).unwrap();
    assert_eq!(ip.subject, "Отчет");
    assert!(ip.attachments.iter().any(|a| a.name == "Отчет.txt"), "{:?}", ip.attachments);
}

/// A file whose long name is blank is named by its short name, as a data file's is: it came
/// in named with spaces.
#[test]
fn a_blank_long_file_name_gives_way_to_the_short_one() {
    let mut cf = CompoundFile::create(Cursor::new(Vec::new())).unwrap();
    write_props(&mut cf, "", TOP_HEADER, &[(0x001A_001F, P::Text("IPM.Note")), (0x0037_001F, P::Text("Plan")), (0x1000_001F, P::Text("Attached."))]);
    let file = "/__attach_version1.0_#00000000";
    cf.create_storage(file).unwrap();
    write_props(
        &mut cf,
        file,
        PART_HEADER,
        &[(0x3707_001F, P::Text("   ")), (0x3704_001F, P::Text("PLAN~1.CSV")), (0x3705_0003, P::Int(1)), (0x3701_0102, P::Bin(b"a,b\n".to_vec()))],
    );
    cf.flush().unwrap();
    let raw = to_mime(&cf.into_inner().into_inner()).unwrap();
    let p = mailcraft_mime::parse(&raw).unwrap();
    assert_eq!(p.attachments.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), ["PLAN~1.CSV"]);
}

/// What isn't a message file is an error, never a panic: other files, a compound file that
/// holds no message, one cut short, one damaged byte by byte, and one too big.
#[test]
fn junk_is_an_error_not_a_panic() {
    assert!(to_mime(b"From: x\r\n\r\nhi").is_err());
    assert!(to_mime(&[]).is_err());
    let mut cf = CompoundFile::create(Cursor::new(Vec::new())).unwrap();
    cf.create_stream("/other").unwrap().write_all(b"x").unwrap();
    cf.flush().unwrap();
    assert!(to_mime(&cf.into_inner().into_inner()).is_err(), "no message in it");
    let good = sample();
    for cut in [8, 512, 1024, good.len() / 2, good.len() - 1] {
        let _ = to_mime(&good[..cut]);
    }
    for at in (0..good.len()).step_by(97) {
        let mut bad = good.clone();
        bad[at] ^= 0xA5;
        let _ = to_mime(&bad);
    }
    let mut huge = good.clone();
    huge.resize(MAX_FILE + 1, 0);
    assert!(to_mime(&huge).is_err());
}

/// The fuzz target's seed (`fuzz/seeds/msg/outlook.msg`) holds [`sample`] as made here, so the
/// weekly run starts from the message these tests read. `MAILCRAFT_WRITE_SEED=1` writes it again
/// first.
#[test]
fn the_fuzz_seed_is_the_sample() {
    let seed = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fuzz/seeds/msg/outlook.msg");
    if std::env::var_os("MAILCRAFT_WRITE_SEED").is_some() {
        std::fs::write(&seed, sample()).unwrap();
    }
    // Its bytes differ from each new sample's by the times the compound file notes; what it holds
    // doesn't.
    let written = std::fs::read(&seed).unwrap_or_default();
    assert_eq!(to_mime(&written).ok(), Some(to_mime(&sample()).unwrap()), "MAILCRAFT_WRITE_SEED=1 writes it");
}

/// A message file with a 60 MB file, read: its bytes were copied twice more on their way.
/// `cargo test --release -p mailcraft-net --lib measure_msg_big_file -- --ignored --nocapture`.
#[test]
#[ignore = "a measurement"]
fn measure_msg_big_file() {
    let mut cf = CompoundFile::create(Cursor::new(Vec::new())).unwrap();
    write_props(&mut cf, "", TOP_HEADER, &[(0x001A_001F, P::Text("IPM.Note")), (0x0037_001F, P::Text("Big")), (0x1000_001F, P::Text("Attached."))]);
    let file = "/__attach_version1.0_#00000000";
    cf.create_storage(file).unwrap();
    let data: Vec<u8> = (0..60_000_000u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 24) as u8).collect();
    write_props(&mut cf, file, PART_HEADER, &[(0x3707_001F, P::Text("big.bin")), (0x3705_0003, P::Int(1)), (0x3701_0102, P::Bin(data))]);
    cf.flush().unwrap();
    let bytes = cf.into_inner().into_inner();
    let n = to_mime(&bytes).unwrap().len();
    let t = std::time::Instant::now();
    for _ in 0..5 {
        assert_eq!(to_mime(&bytes).unwrap().len(), n);
    }
    println!("MEASURE .msg with a 60 MB file read: {:.1} ms", t.elapsed().as_secs_f64() * 1000.0 / 5.0);
}

/// How long turning a message file's multi-megabyte UTF-16 and 8-bit text property into its
/// value takes.
/// `cargo test --release -p mailcraft-net --lib msg_text_property_speed -- --ignored --nocapture`
#[test]
#[ignore = "a measurement"]
fn msg_text_property_speed() {
    const ROUNDS: u32 = 20;
    let line = "Dear Ana, the r\u{e9}sum\u{e9} is attached; see you on Thursday at the caf\u{e9}.\r\n";
    let text = line.repeat(8 * 1024 * 1024 / line.len());
    let wide = Prop { id: 0x1000, ty: 0x001F, value: Stored::Bytes([utf16le(&text), vec![0; 4]].concat()) };
    let narrow_bytes = text.chars().map(|c| if c == '\u{e9}' { 0xE9 } else { c as u8 }).chain([0]).collect();
    let narrow = Prop { id: 0x1000, ty: 0x001E, value: Stored::Bytes(narrow_bytes) };
    for (name, p) in [("UTF-16", &wide), ("8-bit", &narrow)] {
        assert_eq!(val(p, 1252), Val::Text(Cow::Borrowed(text.as_str())), "{name}");
        let started = std::time::Instant::now();
        for _ in 0..ROUNDS {
            std::hint::black_box(val(std::hint::black_box(p), 1252));
        }
        let per = started.elapsed() / ROUNDS;
        println!("MEASURE .msg {name} text property of {} bytes into its value: {per:?} each ({ROUNDS} rounds)", text.len());
    }
}

mod text_values {
    use proptest::prelude::*;

    use super::*;

    /// What a text came to before NULs were cut in place: trimmed, then copied.
    fn copied(s: &str) -> String {
        s.trim_end_matches('\0').to_string()
    }

    proptest! {
        /// A message file's text value is what it was when it was trimmed and copied: trailing
        /// NULs gone, inner ones kept, single and multi-valued, UTF-16 and 8-bit in a code page.
        #[test]
        fn a_text_value_is_unchanged(
            s in proptest::collection::vec(prop_oneof![3 => Just('\0'), 2 => any::<char>()], 0..40),
            b in proptest::collection::vec(prop_oneof![3 => Just(0u8), 2 => any::<u8>()], 0..40),
            page in prop_oneof![Just(0u32), Just(1252), Just(65001), Just(932)],
        ) {
            let wide_bytes = utf16le(&s.into_iter().collect::<String>());
            let wide = copied(&utf16(&wide_bytes));
            let narrow = copied(&eight_bit(&b, page));
            let one = |ty: u16, bytes: &[u8]| Prop { id: 0x0037, ty, value: Stored::Bytes(bytes.to_vec()) };
            let many = |ty: u16, bytes: &[u8]| Prop { id: 0x0037, ty, value: Stored::Many(vec![bytes.to_vec(), bytes.to_vec()]) };
            let (one_wide, one_narrow) = (one(0x001F, &wide_bytes), one(0x001E, &b));
            let (some_wide, some_narrow) = (many(0x101F, &wide_bytes), many(0x101E, &b));
            prop_assert_eq!(val(&one_wide, page), Val::Text(Cow::Borrowed(wide.as_str())));
            prop_assert_eq!(val(&one_narrow, page), Val::Text(Cow::Borrowed(narrow.as_str())));
            prop_assert_eq!(val(&some_wide, page), Val::Texts(vec![wide.clone(), wide]));
            prop_assert_eq!(val(&some_narrow, page), Val::Texts(vec![narrow.clone(), narrow]));
        }
    }
}
