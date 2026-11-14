//! Engine tests: the junk lists when an entry can't go on, or the mail can't go where they say.

use crate::JunkList;
use crate::session::junk::JUNK_LIST_MAX;

use super::*;

/// An entry a full list refuses stays where it was: a safe sender the full blocked list refuses
/// is still safe (it was taken off the safe list before the refusal, and so on neither).
#[test]
fn a_full_list_leaves_the_other_one_as_it_was() {
    let mut s = sample_session("junk-full");
    s.set_junk_entry(JunkList::Safe, "dana@company.example", true).unwrap();
    s.prefs.blocked_senders = (0..JUNK_LIST_MAX).map(|n| format!("sender{n}@spam.example")).collect();
    let e = s.set_junk_entry(JunkList::Blocked, "dana@company.example", true).unwrap_err();
    assert!(e.to_string().contains("full"), "{e}");
    assert!(s.prefs.safe_senders.contains("dana@company.example"), "still safe");
    assert!(!s.prefs.blocked_senders.contains("dana@company.example"));
    // Block Sender on the selection says the list is full, not that there's no sender.
    s.run("folder.open", &json!({"account": "sample", "folder": "INBOX"})).unwrap();
    let k = key_of(&s, "Storyboard review on Thursday");
    s.run("list.select", &k).unwrap();
    let e = s.run("junk.blockSender", &json!({})).unwrap_err();
    assert!(e.to_string().contains("full"), "{e}");
}

/// Block Sender where the mail can't go to Junk (the account has no Junk folder): the sender is
/// blocked, and the answer and a notice say the mail stayed where it was.
#[test]
fn a_move_the_junk_lists_cant_make_is_said() {
    let mut s = sample_session("junk-no-folder");
    if let Some(a) = s.accounts.iter_mut().find(|a| a.config.id == "sample") {
        a.folders.retain(|f| f.role != FolderRole::Junk);
    }
    s.run("folder.open", &json!({"account": "sample", "folder": "INBOX"})).unwrap();
    let k = key_of(&s, "Storyboard review on Thursday");
    s.run("list.select", &k).unwrap();
    let r = s.run("junk.blockSender", &json!({})).unwrap();
    assert_eq!(r["added"], json!(["zoe@studio.example"]));
    assert_eq!(r["moved"], 0, "{r}");
    assert!(r["notMoved"].as_str().is_some_and(|w| w.contains("Junk")), "{r}");
    assert!(s.notices.iter().any(|n| n.error && n.text.contains("stays where it is")), "{:?}", s.notices);
    assert!(s.list.iter().any(|m| m.subject == "Storyboard review on Thursday"), "still in the Inbox");
}

/// A list with room for one sender more, and several selected: the one that went on is said,
/// and so is that the list was full for the rest (nothing said so, as long as one went on).
#[test]
fn a_list_full_for_some_of_the_senders_is_said() {
    let mut s = sample_session("junk-full-for-some");
    s.prefs.safe_senders = (1..JUNK_LIST_MAX).map(|n| format!("sender{n}@fine.example")).collect();
    s.run("folder.open", &json!({"account": "sample", "folder": "INBOX"})).unwrap();
    s.run("list.selectAll", &json!({})).unwrap();
    let r = s.run("junk.neverBlockSender", &json!({})).unwrap();
    assert_eq!(r["added"].as_array().map(Vec::len), Some(1), "{r}");
    assert!(r["notAdded"].as_str().is_some_and(|w| w.contains("full")), "{r}");
    assert!(s.notices.iter().any(|n| n.error && n.text.contains("Not every sender") && n.text.contains("full")), "{:?}", s.notices);
    assert_eq!(s.prefs.safe_senders.len(), JUNK_LIST_MAX);
    // Room for all of them: nothing of the kind.
    s.prefs.safe_senders.clear();
    s.notices.clear();
    let r = s.run("junk.neverBlockSender", &json!({})).unwrap();
    assert!(r.get("notAdded").is_none() && !s.notices.iter().any(|n| n.error), "{r} {:?}", s.notices);
}
