//! What an export to an Outlook data file keeps in an item's text and the import reads back
//! (`pst_pim::rest`): a contact's text, a card of what Outlook has no field for after it, and a
//! task's, a to-do's properties after it; and a task's rule as the recurrence pattern the export
//! writes. What comes back is held to what a card and a task keep, and a pattern written is one
//! the import reads.
//!
//! The input is the task's first day (eight bytes, little-endian Unix seconds), then the text,
//! whose first line is also the rule.
#![no_main]

use libfuzzer_sys::fuzz_target;
use mailcraft_net::{pst, pst_export};

fuzz_target!(|input: &[u8]| {
    let Some((from, text)) = input.split_first_chunk::<8>() else { return };
    let from = i64::from_le_bytes(*from);
    let text = String::from_utf8_lossy(text);
    let (card, task) = pst_export::read_back_text(&text);
    assert!(card.emails.len() <= 50 && card.phones.len() <= 50 && card.addresses.len() <= 20 && card.extra.len() <= 201);
    // A to-do's lines as a to-do keeps them: none of its own says the task's uid, title, days or
    // rule, and each component one begins is ended (so none ends the to-do).
    assert!(task.extra.len() <= 200);
    let mut open: Vec<String> = Vec::new();
    for l in &task.extra {
        let name = l.split([':', ';']).next().unwrap_or_default().trim().to_ascii_uppercase();
        let value = l.split_once(':').map_or("", |(_, v)| v).trim().to_ascii_uppercase();
        match name.as_str() {
            "BEGIN" => open.push(value),
            "END" => assert_eq!(open.pop().as_deref(), Some(value.as_str()), "{l}"),
            "UID" | "SUMMARY" | "DTSTART" | "DUE" | "RRULE" => assert!(!open.is_empty(), "{l}"),
            _ => {}
        }
    }
    assert!(open.is_empty());
    let rule = text.lines().next().unwrap_or_default();
    if let Some((pattern, _)) = pst_export::task_pattern(rule, from) {
        // The pattern counts from the task's first day: a task due then reads it.
        let back = pst::repeating_task(&pattern, from.div_euclid(86_400) * 86_400);
        assert!(back.rrule.is_some(), "{rule}");
    }
});
