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
