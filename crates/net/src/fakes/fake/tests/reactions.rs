//! Issue #39: a reaction's emoji is known from the folder's sync, whatever is downloaded ahead.

use super::*;

/// With nothing downloaded ahead, a message whose header says it's a reaction has its small
/// body brought by the sync, so its summary says its emoji; a big one, or other mail, isn't.
#[test]
fn a_reaction_is_known_from_the_sync() {
    let st = State::new();
    let reaction = b"From: Ben <ben@studio.example>\r\nTo: me@studio.example\r\nSubject: Re: Plan\r\nMessage-ID: <r@studio.example>\r\n\
In-Reply-To: <plan@studio.example>\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Disposition: reaction\r\n\r\n\xf0\x9f\x8e\x89\r\n";
    let (r_uid, other) = {
        let mut s = st.lock().unwrap();
        (s.add_raw("INBOX", reaction.to_vec()), s.add("INBOX", "Plain", &[]))
    };
    let store = store("reaction-sync");
    let mut c = Client::from_stream(Fake::connect(&st)).unwrap();
    c.login("me", "pw").unwrap();
    sync_folder(&mut c, &store, "a1", "INBOX", &SyncOptions { window: 50, prefetch: 0, ..Default::default() }).unwrap();
    let f = store.load_folder("a1", "INBOX").unwrap();
    let r = f.messages.iter().find(|m| m.key.uid == r_uid).unwrap();
    assert_eq!((r.reaction.as_str(), r.in_reply_to.as_str()), ("🎉", "plan@studio.example"));
    assert!(store.has_body(&r.key));
    let o = f.messages.iter().find(|m| m.key.uid == other).unwrap();
    assert!(o.reaction.is_empty() && !store.has_body(&o.key), "nothing else downloaded");
}
