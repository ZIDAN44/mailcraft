//! Issue #189: a heading of the message list folded away hides its mail, counted on it, as
//! Outlook's groups collapse; kept for each folder, and between runs.

use super::*;
use crate::Row;

/// The headings of the list, in order, as the list names them.
fn headings(s: &Session) -> Vec<String> {
    s.rows()
        .iter()
        .filter_map(|r| match r {
            Row::Header(h) => Some(h.to_string()),
            Row::Group(g) => Some(g.clone()),
            _ => None,
        })
        .collect()
}

/// The messages under the `n`th heading, as rows are shown with nothing folded.
fn under(s: &Session, n: usize) -> Vec<MessageKey> {
    let mut at = None;
    let mut out = Vec::new();
    for r in s.rows() {
        match r {
            Row::Header(_) | Row::Group(_) => at = Some(at.map_or(0, |k| k + 1)),
            Row::Message { index, .. } if at == Some(n) => out.extend(s.list.get(*index).map(|m| m.key.clone())),
            _ => {}
        }
    }
    out
}

#[test]
fn a_folded_heading_hides_its_mail_and_counts_it() {
    let mut s = sample_session("fold-heading");
    let names = headings(&s);
    assert!(names.len() >= 3, "the sample's Inbox has days apart: {names:?}");
    let second = under(&s, 1);
    let (rows_before, total) = (s.row_count(), s.list.len());
    let r = s.run("list.foldGroup", &json!({"heading": names[1]})).unwrap();
    assert_eq!(r["folded"], true);
    // Its heading stays, its mail counted under it and gone from the rows.
    let at = s.rows().iter().position(|r| matches!(r, Row::Header(h) if *h == names[1])).unwrap();
    assert!(s.heading_folded(at));
    let unread = second.iter().filter(|k| s.summary(k).is_some_and(|m| !m.flags.seen)).count();
    assert_eq!(s.rows().get(at + 1), Some(&Row::Folded { count: second.len(), unread }));
    assert_eq!(s.row_count(), rows_before - second.len());
    assert!(s.row_keys().iter().all(|k| !second.contains(k)), "not reached by the arrows");
    assert_eq!(s.list.len(), total, "still in the list");
    let (rows, _, _) = s.build_rows();
    assert_eq!(s.rows(), &rows[..], "as built afresh");
    // Select All still takes it all, as Outlook's does.
    s.run("list.selectAll", &json!({})).unwrap();
    assert_eq!(s.selection.len(), total);
    // Clicked again: shown.
    assert_eq!(s.run("list.foldGroup", &json!({"heading": names[1]})).unwrap()["folded"], false);
    assert_eq!(s.row_count(), rows_before);
    assert!(s.prefs.folded_groups.is_empty(), "nothing kept for nothing folded");
    assert!(s.run("list.foldGroup", &json!({"heading": "No such day"})).is_err());
}

#[test]
fn the_arrows_step_over_a_folded_heading() {
    let mut s = sample_session("fold-arrows");
    let names = headings(&s);
    let (first, second, third) = (under(&s, 0), under(&s, 1), under(&s, 2));
    // Reading the second day's first message, its day folded: Down goes to the next day's,
    // Up to the day before's last.
    s.open_message(&second[0]);
    s.run("list.foldGroup", &json!({"heading": names[1], "folded": true})).unwrap();
    assert_eq!(s.reading.as_ref().map(|r| r.key.clone()), Some(second[0].clone()), "still read");
    s.run("list.next", &json!({})).unwrap();
    assert_eq!(s.focus.as_ref(), third.first());
    s.open_message(&second[0]);
    s.run("list.prev", &json!({})).unwrap();
    assert_eq!(s.focus.as_ref(), first.last());
    // From the day before's last, Down goes over the folded day.
    s.run("list.next", &json!({})).unwrap();
    assert_eq!(s.focus.as_ref(), third.first());
}

#[test]
fn folds_are_kept_for_each_folder_and_not_in_a_search() {
    let mut s = sample_session("fold-places");
    let names = headings(&s);
    s.run("list.foldGroup", &json!({"heading": names[0]})).unwrap();
    let folded_rows = s.row_count();
    // Another folder: nothing folded there; back, still folded.
    s.run("folder.open", &json!({"account": "sample", "folder": "Sent"})).unwrap();
    assert!(!s.rows().iter().any(|r| matches!(r, Row::Folded { .. })));
    s.run("folder.open", &json!({"account": "sample", "folder": "INBOX"})).unwrap();
    assert_eq!(s.row_count(), folded_rows);
    // Kept with the settings.
    let saved = serde_json::to_value(&s.prefs).unwrap();
    assert_eq!(saved["foldedGroups"]["sample\u{1}INBOX"], json!([format!("date\u{1}{}", names[0])]));
    // Another arrangement folds its own headings.
    s.run("view.arrange", &json!({"by": "from"})).unwrap();
    assert!(!s.rows().iter().any(|r| matches!(r, Row::Folded { .. })), "by sender: none folded");
    s.run("view.arrange", &json!({"by": "date"})).unwrap();
    assert_eq!(s.row_count(), folded_rows);
    // Without headings, nothing is hidden; with them again, it is.
    s.run("view.groups", &json!({"on": false})).unwrap();
    assert_eq!(s.row_count(), s.list.len());
    assert!(s.run("list.foldGroup", &json!({"heading": names[0]})).is_err());
    s.run("view.groups", &json!({"on": true})).unwrap();
    assert_eq!(s.row_count(), folded_rows);
    // A search's list folds nothing.
    s.run("search.run", &json!({"query": "a"})).unwrap();
    assert!(!s.rows().iter().any(|r| matches!(r, Row::Folded { .. })));
    assert!(s.run("list.foldGroup", &json!({"heading": names[0]})).is_err());
}

#[test]
fn every_heading_folds_and_shows_at_once() {
    let mut s = sample_session("fold-all");
    let total = s.row_count();
    let r = s.run("list.collapseAll", &json!({})).unwrap();
    assert_eq!(r["headings"].as_u64(), Some(headings(&s).len() as u64));
    assert_eq!(s.row_count(), 0, "every message folded away");
    let hidden: usize = s.rows().iter().map(|r| if let Row::Folded { count, .. } = r { *count } else { 0 }).sum();
    assert_eq!(hidden, total);
    s.run("list.expandAll", &json!({})).unwrap();
    assert_eq!(s.row_count(), total);
}

#[test]
fn a_folder_renamed_keeps_what_was_folded() {
    let mut s = sample_session("fold-rename");
    s.prefs.folded_groups.insert("sample\u{1}Projects/Short film".into(), ["date\u{1}Today".to_string()].into());
    s.prefs.folded_groups.insert("other\u{1}Projects".into(), ["date\u{1}Today".to_string()].into());
    s.run("folder.rename", &json!({"account": "sample", "folder": "Projects", "name": "Work"})).unwrap();
    let keys: Vec<&String> = s.prefs.folded_groups.keys().collect();
    assert_eq!(keys, ["other\u{1}Projects", "sample\u{1}Work/Short film"], "another account's stays");
    // Deleted for good, with what's in it: what was folded in it goes too.
    s.run("folder.delete", &json!({"account": "sample", "folder": "Work", "permanent": true})).unwrap();
    assert!(s.folder("sample", "Work").is_none());
    let keys: Vec<&String> = s.prefs.folded_groups.keys().collect();
    assert_eq!(keys, ["other\u{1}Projects"]);
}
