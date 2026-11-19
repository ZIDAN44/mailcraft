//! Issue #148: a conversation shows the user's own replies from Sent Items, as Outlook's do
//! (messages from other folders), found by their Message-IDs.

use super::*;

fn open_subject(s: &mut Session, subject: &str) -> mailcraft_model::MessageKey {
    let k = s.list.iter().find(|m| m.subject == subject).map(|m| m.key.clone()).unwrap_or_else(|| panic!("{subject} in the list"));
    s.open_message(&k);
    k
}

#[test]
fn a_conversation_shows_my_replies_from_sent_items() {
    let mut s = sample_session("conversation-sent");
    // Group messages by conversation (off in the sample mailbox as it starts).
    s.run("view.conversations", &json!({})).unwrap();
    assert!(s.prefs.conversations);
    s.run("folder.open", &json!({"account": "sample", "folder": "INBOX"})).unwrap();
    let subject = "Your proof for the festival poster is ready";
    let original = open_subject(&mut s, subject);
    assert!(s.reading.as_ref().unwrap().aside.is_empty(), "no reply yet");
    // Replied to and sent: filed in Sent Items (the sample mailbox sends nowhere).
    s.run("mail.reply", &json!({})).unwrap();
    s.run("compose.update", &json!({"body": "Approved, thanks."})).unwrap();
    s.run("compose.send", &json!({})).unwrap();
    s.run("folder.open", &json!({"account": "sample", "folder": "INBOX"})).unwrap();
    let k = open_subject(&mut s, subject);
    assert_eq!(k, original);
    let r = s.reading.clone().unwrap();
    let sent = s.account("sample").and_then(|a| a.folder(FolderRole::Sent)).map(|f| f.path.clone()).unwrap();
    let [mine] = r.aside.as_slice() else { panic!("{:?}", r.aside) };
    assert_eq!((mine.key.folder.as_str(), mine.subject.as_str()), (sent.as_str(), "Re: Your proof for the festival poster is ready"));
    // Another message of the Inbox, with nothing of mine in Sent: none.
    open_subject(&mut s, "Character turnarounds");
    assert!(s.reading.as_ref().unwrap().aside.is_empty(), "only replies to its own conversation");

    // Not with conversations off, in a search, nor in Sent Items itself.
    s.run("view.conversations", &json!({})).unwrap();
    open_subject(&mut s, subject);
    assert!(s.reading.as_ref().unwrap().aside.is_empty(), "conversations off");
    s.run("view.conversations", &json!({})).unwrap();
    open_subject(&mut s, subject);
    assert_eq!(s.reading.as_ref().unwrap().aside.len(), 1, "conversations on again");
    s.run("folder.open", &json!({"account": "sample", "folder": sent})).unwrap();
    open_subject(&mut s, "Re: Your proof for the festival poster is ready");
    assert!(s.reading.as_ref().unwrap().aside.is_empty(), "my own mail is in the list here");
    // Deleting the conversation's message in the Inbox leaves my reply in Sent Items.
    s.run("folder.open", &json!({"account": "sample", "folder": "INBOX"})).unwrap();
    let k = open_subject(&mut s, subject);
    s.selection = vec![k];
    s.run("mail.delete", &json!({})).unwrap();
    let kept = s.folder_state("sample", &sent).messages.iter().any(|m| m.subject == "Re: Your proof for the festival poster is ready");
    assert!(kept, "my reply stays in Sent Items");
}

/// Issue #170: Delete on a conversation's row (the list grouped by conversation) deletes the
/// whole conversation in the folder, as Outlook does; it deleted its newest message alone, one
/// press for each. An older message picked in the reading pane's conversation goes alone.
#[test]
fn deleting_a_conversations_row_deletes_the_whole_conversation() {
    let mut s = sample_session("conversation-delete");
    s.run("view.conversations", &json!({"on": true})).unwrap();
    s.run("folder.open", &json!({"account": "sample", "folder": "INBOX"})).unwrap();
    let inbox =
        |s: &mut Session| -> Vec<mailcraft_model::MessageKey> { s.folder_state("sample", "INBOX").messages.iter().map(|m| m.key.clone()).collect() };
    let row = (0..s.row_count()).filter_map(|p| s.row_key(p).cloned()).find(|k| s.thread_of(k).len() > 1).expect("a conversation");
    let members = s.thread_of(&row);
    let before = inbox(&mut s).len();

    // An older one of them, picked by itself (in the reading pane's conversation): it alone.
    let older = members.iter().find(|k| **k != row).cloned().unwrap();
    s.selection = vec![older.clone()];
    s.run("mail.delete", &json!({})).unwrap();
    let left = inbox(&mut s);
    assert!(left.len() == before - 1 && !left.contains(&older) && left.contains(&row), "the older one alone");
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(inbox(&mut s).len(), before);

    // Its row: the whole conversation together, and back together at one Undo.
    let row =
        (0..s.row_count()).filter_map(|p| s.row_key(p).cloned()).find(|k| s.thread_of(k).len() == members.len()).expect("the conversation again");
    let members = s.thread_of(&row);
    s.selection = vec![row];
    s.run("mail.delete", &json!({})).unwrap();
    let left = inbox(&mut s);
    assert!(members.iter().all(|k| !left.contains(k)), "the whole conversation deleted");
    assert_eq!(left.len(), before - members.len(), "nothing else");
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(inbox(&mut s).len(), before, "back at one Undo");

    // With conversations off, the row is one message.
    s.run("view.conversations", &json!({"on": false})).unwrap();
    let row = s.list.iter().map(|m| m.key.clone()).find(|k| members.iter().any(|x| x.uid == k.uid)).or_else(|| s.list.first().map(|m| m.key.clone()));
    s.selection = row.into_iter().collect();
    s.run("mail.delete", &json!({})).unwrap();
    assert_eq!(inbox(&mut s).len(), before - 1);
}
