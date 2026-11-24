//! Engine tests: a table pasted into a message (issue #268), by its command.

use super::*;

/// `compose.pasteHtml` pastes as the editor does: a table in HTML stays a table, and the message
/// goes with it; HTML with no table is its text.
#[test]
fn a_table_is_pasted_by_its_command() {
    let mut s = sample_session("paste-table");
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"to": "ann@x.example", "subject": "Prices", "body": "Prices below"})).unwrap();
    let html = "<table><tr><td>Paper</td><td>12.50</td></tr><tr><td>Ink</td><td>8.00</td></tr></table>";
    let answer = s.run("compose.pasteHtml", &json!({"html": html})).unwrap();
    assert_eq!(answer["tables"], 1);
    let c = s.compose.clone().unwrap();
    assert_eq!(c.body, "Prices below\nPaper\t12.50\nInk\t8.00");
    assert!(c.bodies().1.is_some_and(|h| h.contains(mailcraft_mime::TABLE_CLASS)));
    let answer = s.run("compose.pasteHtml", &json!({"html": "<p>Thanks, <b>Riley</b></p>", "at": 0})).unwrap();
    assert_eq!((answer["at"].as_u64(), answer["tables"].as_u64()), (Some(13), Some(1)));
    assert!(s.compose.as_ref().unwrap().body.starts_with("Thanks, Riley"));
    assert!(s.run("compose.pasteHtml", &json!({})).is_err(), "html is needed");
}
