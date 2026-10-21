use super::*;

#[test]
fn session_urls_dates_and_keywords() {
    assert_eq!(session_url("jmap.example.com"), "https://jmap.example.com/.well-known/jmap");
    assert_eq!(session_url("https://api.example.com/"), "https://api.example.com/.well-known/jmap");
    assert_eq!(session_url("https://api.example.com/jmap/session"), "https://api.example.com/jmap/session");
    assert_eq!(utc_date("1970-01-02T00:00:00Z"), Some(86_400));
    assert_eq!(utc_date("2024-05-01T09:30:00.123Z"), utc_date("2024-05-01T09:30:00Z"));
    assert_eq!(utc_date("2024-05-01T11:30:00+02:00"), utc_date("2024-05-01T09:30:00Z"));
    for bad in ["", "2024-13-01T00:00:00Z", "yesterday", "2024-05-01"] {
        assert_eq!(utc_date(bad), None, "{bad}");
    }
    let f = keywords(Some(&json!({"$seen": true, "$Flagged": true, "$label1": true, "$draft": false, "bad word": true})));
    assert!(f.seen && f.flagged && !f.draft && f.has("$label1"));
    assert_eq!(flag_keyword(Flag::Deleted), None);
    let base = Url::parse("https://h.example/.well-known/jmap").unwrap();
    assert_eq!(
        template_url(&base, "/download/{accountId}/{blobId}/{name}?type={type}").unwrap(),
        "https://h.example/download/{accountId}/{blobId}/{name}?type={type}"
    );
    assert_eq!(expand("/d/{blobId}/{name}", &[("blobId", "b 1"), ("name", "m.eml")]), "/d/b%201/m.eml");
    assert!(template_url(&base, "http://h.example/upload/{accountId}/").is_err(), "no downgrade");
}

#[test]
fn mailbox_paths() {
    let mb = |id: &str, name: &str, parent: Option<&str>, role: FolderRole| Mailbox {
        id: id.into(),
        name: name.into(),
        parent: parent.map(str::to_string),
        role,
        total: 0,
        unread: 0,
    };
    let boxes = vec![
        mb("1", "Inbox", None, FolderRole::Inbox),
        mb("2", "Projects", None, FolderRole::Other),
        mb("3", "Café", Some("2"), FolderRole::Other),
        mb("4", "Lists", Some("1"), FolderRole::Other),
        mb("5", "a/b", None, FolderRole::Other),
        mb("6", "Loop", Some("7"), FolderRole::Other),
        mb("7", "Back", Some("6"), FolderRole::Other),
        mb("8", "Projects", None, FolderRole::Other),
    ];
    let f = folders("a1", &boxes);
    let paths: Vec<&str> = f.iter().map(|(f, _)| f.path.as_str()).collect();
    assert_eq!(paths[..4], ["INBOX", "Projects", "Projects/Caf&AOk-", "INBOX/Lists"]);
    assert_eq!(utf7::decode(paths[4]), "a∕b", "a slash in a name isn't a level");
    assert!(paths[5].ends_with("Loop") && paths[6].ends_with("Back"), "a parent loop ends: {paths:?}");
    assert_eq!(paths[7], "Projects~8", "duplicate names stay apart");
    assert_eq!(f[2].0.display_name(), "Café");
    let mut st = State::default();
    let (a, b) = (st.number("M1"), st.number("M2"));
    assert_eq!((a, b, st.number("M1"), st.id(b)), (1, 2, 1, Some("M2")));
    st.prune(&[b].into_iter().collect());
    assert_eq!(st.id(a), None);
    assert_eq!(st.number("M3"), 3, "numbers aren't reused");
}

/// A canned server: the session, then whatever the test hands back per API call.
struct Canned(std::cell::RefCell<Vec<Response>>);

impl Http for Canned {
    async fn send(&self, _req: Request) -> Result<Response> {
        let mut q = self.0.borrow_mut();
        if q.is_empty() { Err(Error::Io("no more answers".into())) } else { Ok(q.remove(0)) }
    }
}

#[test]
fn connect_checks_the_session() {
    let session = json!({
        "capabilities": {"urn:ietf:params:jmap:core": {"maxObjectsInGet": 50}, "urn:ietf:params:jmap:mail": {}},
        "primaryAccounts": {"urn:ietf:params:jmap:mail": "A1"},
        "apiUrl": "/api/", "downloadUrl": "/d/{accountId}/{blobId}", "uploadUrl": "/u/{accountId}/",
    });
    let ok = |v: &Value| Response { status: 200, body: v.to_string().into_bytes() };
    let c = block_on(Client::connect(Canned(vec![ok(&session)].into()), "jmap.example.com", "me", "pw", false)).unwrap();
    assert_eq!(c.api, "https://jmap.example.com/api/");
    assert_eq!(c.authorization(), "Basic bWU6cHc=");
    assert_eq!(c.events_url(), None);
    let refused = block_on(Client::connect(Canned(vec![Response { status: 401, body: Vec::new() }].into()), "h", "me", "x", false));
    assert!(matches!(refused, Err(Error::Auth(_))));
    let no_mail = json!({"capabilities": {}, "apiUrl": "/a"});
    assert!(matches!(block_on(Client::connect(Canned(vec![ok(&no_mail)].into()), "h", "me", "x", true)), Err(Error::Protocol(_))));
    let downgrade = json!({"capabilities": {"urn:ietf:params:jmap:mail": {}}, "primaryAccounts": {"urn:ietf:params:jmap:mail": "A"}, "apiUrl": "http://h/api", "downloadUrl": "/d", "uploadUrl": "/u"});
    assert!(matches!(block_on(Client::connect(Canned(vec![ok(&downgrade)].into()), "h", "me", "x", true)), Err(Error::Tls(_))));
}
