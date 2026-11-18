//! What a message answers comes with it from Exchange (its In-Reply-To and References), so a
//! reply joins its conversation: one from Sent Items too (issue #148).

use super::*;

#[test]
fn a_reply_says_what_it_answers() {
    let mut d = Data::default();
    let inbox = d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let reply = "From: ana@contoso.example\r\nTo: riley@contoso.example\r\nSubject: Re: Plan\r\nMessage-ID: <r2@contoso.example>\r\n\
                 In-Reply-To: <r1@contoso.example>\r\nReferences: <r0@contoso.example> <r1@contoso.example>\r\n\r\nYes\r\n";
    d.deliver(&inbox, reply.as_bytes(), false, "2026-11-01T10:00:00Z");
    let server = serve(d);
    let store = Store::open(crate::data_files::scratch::scratch("ews-threading")).unwrap();
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    let mut st = crate::jmap::State::default();
    crate::ews::list_folders(&c, &mut st, "a1").unwrap();
    // Its headers alone (no source downloaded ahead, as for mail past the newest).
    let opts = crate::sync::SyncOptions { prefetch: 0, ..Default::default() };
    crate::ews::sync_folder(&c, &mut st, &store, "a1", "INBOX", &opts).unwrap();
    let inbox = store.load_folder("a1", "INBOX").unwrap();
    let m = inbox.messages.first().expect("synced");
    assert_eq!(m.message_id, "r2@contoso.example");
    assert_eq!(m.in_reply_to, "r1@contoso.example");
    assert_eq!(m.references, ["r0@contoso.example", "r1@contoso.example"]);
}
