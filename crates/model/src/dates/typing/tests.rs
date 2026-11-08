use super::*;
use crate::dates::{DateStyle, time_text};

/// Some languages' words, as their catalogs write a 12-hour time (`{time} a. m.`, `午後{time}`):
/// (before the time, after it) for each half.
const WRITTEN: &[((&str, &str), (&str, &str))] = &[
    (("", " AM"), ("", " PM")),
    (("", " a. m."), ("", " p. m.")),
    (("", " a.m."), ("", " p.m.")),
    (("午前", ""), ("午後", "")),
    (("오전 ", ""), ("오후 ", "")),
    (("上午", ""), ("下午", "")),
    (("", " ص"), ("", " م")),
    (("", " לפנה״צ"), ("", " אחה״צ")),
    (("", " ÖÖ"), ("", " ÖS")),
];

fn halves() -> DayHalves {
    let mut h = DayHalves::default();
    for ((a0, a1), (p0, p1)) in WRITTEN {
        h.add(&format!("{a0}{a1}"), &format!("{p0}{p1}"));
    }
    h
}

fn read(text: &str) -> Option<(u32, u32)> {
    read_time(text, &halves())
}

/// Gap 9: a time typed on either clock, as people write it.
#[test]
fn times_as_people_type_them() {
    for (typed, time) in [
        ("13:30", (13, 30)),
        ("09:05", (9, 5)),
        ("9:05", (9, 5)),
        ("0:00", (0, 0)),
        ("23:59", (23, 59)),
        (" 13:30 ", (13, 30)),
        ("1:30 PM", (13, 30)),
        ("1:30pm", (13, 30)),
        ("1:30 p.m.", (13, 30)),
        ("1:30 P.M.", (13, 30)),
        ("1:30 P M", (13, 30)),
        ("1 PM", (13, 0)),
        ("1pm", (13, 0)),
        ("11 am", (11, 0)),
        ("12 AM", (0, 0)),
        ("12:30 AM", (0, 30)),
        ("12 PM", (12, 0)),
        ("12:45 pm", (12, 45)),
        ("PM 1:30", (13, 30)),
        // Other languages' words, in their places or not, as typed.
        ("1:30 a. m.", (1, 30)),
        ("1:30 a.m.", (1, 30)),
        ("1:30 p. m.", (13, 30)),
        ("午後1:30", (13, 30)),
        ("午前12:05", (0, 5)),
        ("오후 3:15", (15, 15)),
        ("下午 3:15", (15, 15)),
        ("3:15 م", (15, 15)),
        ("3:15 ص", (3, 15)),
        ("3:15 אחה\"צ", (15, 15)),
        ("3:15 לפנהצ", (3, 15)),
        ("3:15 ös", (15, 15)),
        ("3:15 \u{2068}م\u{2069}", (15, 15)),
        // Digits as East Asian input methods and Arabic write them.
        ("午後１：３０", (13, 30)),
        ("\u{663}:\u{661}\u{665} م", (15, 15)),
    ] {
        assert_eq!(read(typed), Some(time), "{typed}");
    }
}

/// Gap 9: what isn't a time is refused, never half-read.
#[test]
fn what_isnt_a_time_is_refused() {
    for typed in [
        "",
        "  ",
        "PM",
        "noon",
        "13 PM",
        "13:30 PM",
        "0 AM",
        "0:30 am",
        "25:00",
        "24:00",
        "12:60",
        "1:5",
        "1:050",
        "123:00",
        "9",
        "13",
        "1:30 xm",
        "1:30 pm pm",
        "1:30 pm 2",
        "1:30:00",
        "1::30",
        ":30",
        "1.30 pm",
        "１３ PM",
        "a1:30",
        "午後13:30",
    ] {
        assert_eq!(read(typed), None, "{typed}");
    }
    assert_eq!(read(&format!("1:30 {}", "x".repeat(100))), None);
    assert_eq!(read(&"1".repeat(1000)), None);
    // A language that leaves its word empty doesn't make every bare number a time.
    let mut h = DayHalves::default();
    h.add("", " . ");
    assert_eq!(read_time("9", &h), None);
    // A word already known keeps its half.
    h.add("pm", "am");
    assert_eq!(read_time("1 pm", &h), Some((13, 0)));
}

/// Gap 9: the box shows its time as times are shown elsewhere.
#[test]
fn times_written_for_the_box() {
    let twelve = DateStyle { hours12: true, ..DateStyle::default() };
    assert_eq!(time_text(13, 30, DateStyle::default()), "13:30");
    assert_eq!(time_text(9, 5, DateStyle::default()), "09:05");
    assert_eq!(time_text(13, 30, twelve), "1:30 PM");
    assert_eq!(time_text(0, 5, twelve), "12:05 AM");
    assert_eq!(time_text(12, 0, twelve), "12:00 PM");
}

proptest::proptest! {
    /// Gap 9: any minute of the day, shown on either clock in any of these languages' words,
    /// written in any case, with or without its dots and spaces, reads back as itself.
    #[test]
    fn shown_then_read_is_the_same_time(minute in 0u32..1440, lang in 0..WRITTEN.len(), hours12: bool, upper: bool, bare: bool) {
        let (hour, minute) = (minute / 60, minute % 60);
        let shown = time_text(hour, minute, DateStyle { hours12, ..DateStyle::default() });
        let shown = match (WRITTEN.get(lang), shown.split_once(' ')) {
            (Some(((a0, a1), (p0, p1))), Some((time, half))) => {
                let (before, after) = if half == "AM" { (a0, a1) } else { (p0, p1) };
                format!("{before}{time}{after}")
            }
            _ => shown,
        };
        let shown = if upper { shown.to_uppercase() } else { shown.to_lowercase() };
        let shown = if bare { shown.replace(['.', ' '], "") } else { shown };
        proptest::prop_assert_eq!(read(&shown), Some((hour, minute)), "{}", shown);
    }

    /// Whatever is typed, reading it never panics, and what it reads is a time of day.
    #[test]
    fn anything_typed_reads_safely(text in "\\PC{0,80}") {
        if let Some((hour, minute)) = read(&text) {
            proptest::prop_assert!(hour < 24 && minute < 60);
        }
    }
}
