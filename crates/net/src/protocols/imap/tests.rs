use super::*;

/// A canned server: reads are served from `input`; writes are recorded.
pub(crate) struct Script {
    input: io::Cursor<Vec<u8>>,
    pub written: Vec<u8>,
}

impl Script {
    fn new(s: &[u8]) -> Self {
        Script { input: io::Cursor::new(s.to_vec()), written: Vec::new() }
    }
}

impl Read for Script {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        // Deliver a few bytes at a time to exercise framing across reads.
        let n = buf.len().min(7);
        self.input.read(&mut buf[..n])
    }
}

impl Write for Script {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.written.extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn sets_and_dates() {
    assert_eq!(uid_set(&[5, 1, 2, 3, 9, 10, 3, 0]), "1:3,5,9:10");
    assert_eq!(uid_set(&[]), "");
    assert_eq!(uid_set(&[u32::MAX, u32::MAX - 1]), format!("{}:{}", u32::MAX - 1, u32::MAX));
    assert_eq!(parse_internal_date("14-Oct-2026 09:30:00 +0200"), Some(1_791_963_000));
    assert_eq!(parse_internal_date(" 4-Jan-1970 00:00:00 -0100"), Some(3 * 86_400 + 3600));
    assert_eq!(parse_internal_date("junk"), None);
    assert_eq!(parse_internal_date("99-Oct-2026 09:30:00 +0200"), None);
    assert_eq!(base64(b"\0user\0pass"), "AHVzZXIAcGFzcw==");
    assert_eq!(base64(b"ab"), "YWI=");
    assert_eq!(quoted("a\"b\\c").as_deref(), Some("\"a\\\"b\\\\c\""));
    assert_eq!(quoted("pässword"), None);
}

#[test]
fn literals() {
    assert_eq!(literal_len(b"* 1 FETCH (BODY[] {12}"), Some(12));
    assert_eq!(literal_len(b"A1 APPEND x {3+}"), Some(3));
    assert_eq!(literal_len(b"* OK {abc}"), None);
    assert_eq!(literal_len(b"* OK done"), None);
    assert_eq!(literal_len(b"{99999999999999999}"), None);
}

#[test]
fn greeting_login_fetch_with_literals() {
    let server = b"* OK [CAPABILITY IMAP4rev1 MOVE UIDPLUS] ready\r\n\
M1 OK [CAPABILITY IMAP4rev1 MOVE UIDPLUS CONDSTORE] logged in\r\n\
* 1 FETCH (UID 7 FLAGS (\\Seen) RFC822.SIZE 12 BODY[HEADER] {12}\r\nSubject: x\r\n)\r\n\
* 2 FETCH (FLAGS (\\Flagged))\r\n\
M2 OK done\r\n";
    let mut c = Client::from_stream(Script::new(server)).unwrap();
    assert!(c.has("move") && !c.has("CONDSTORE"));
    c.login("zoe@x", "pässword").unwrap_err(); // literal path needs a '+' the script lacks
}

#[test]
fn login_quoted_then_fetch() {
    let server = b"* OK hi\r\n\
M1 OK [CAPABILITY IMAP4rev1 MOVE UIDPLUS] logged in\r\n\
* 1 FETCH (UID 7 FLAGS (\\Seen) RFC822.SIZE 12 BODY[HEADER] {12}\r\nSubject: x\r\n)\r\n\
* 2 FETCH (FLAGS (\\Flagged))\r\n\
M2 OK done\r\n";
    let mut c = Client::from_stream(Script::new(server)).unwrap();
    c.login("zoe@x", "pw").unwrap();
    assert!(c.has("UIDPLUS"));
    let f = c.uid_fetch("7", "UID FLAGS RFC822.SIZE BODY.PEEK[HEADER]", "").unwrap();
    assert_eq!(f.len(), 1);
    assert_eq!(f[0].uid, 7);
    assert_eq!(f[0].header.as_deref(), Some(&b"Subject: x\r\n"[..]));
    assert_eq!(f[0].flags.as_deref(), Some(&["\\Seen".to_string()][..]));
    let written = String::from_utf8(c.conn.into_inner().written).unwrap();
    assert!(written.starts_with("M1 LOGIN \"zoe@x\" \"pw\"\r\nM2 UID FETCH 7 ("), "{written}");
}

#[test]
fn errors_and_hostile_input() {
    let mut c = Client::from_stream(Script::new(b"* OK hi\r\nM1 NO [AUTHENTICATIONFAILED] bad password\r\n")).unwrap();
    assert!(matches!(c.login("u", "p"), Err(NetError::Auth(m)) if m.contains("bad password")));
    assert!(matches!(Client::from_stream(Script::new(b"* BYE go away\r\n")), Err(NetError::Closed(_))));
    assert!(Client::from_stream(Script::new(b"")).is_err());
    assert!(Client::from_stream(Script::new(b"garbage\r\n")).is_err());
    let mut c = Client::from_stream(Script::new(b"* OK hi\r\n* 1 FETCH (BODY[] {999999999999}\r\n")).unwrap();
    assert!(matches!(c.uid_fetch("1", "BODY.PEEK[]", ""), Err(NetError::Protocol(_))));
    let mut c = Client::from_stream(Script::new(b"* OK hi\r\n* WEIRD stuff\r\nM1 OK fine\r\n")).unwrap();
    assert!(c.noop().is_ok());
    // Bare LF line ends are tolerated.
    let mut c = Client::from_stream(Script::new(b"* OK hi\n* SEARCH 3 1 2\nM1 OK done\n")).unwrap();
    assert_eq!(c.uid_search("ALL").unwrap(), vec![1, 2, 3]);
}

/// A response code followed straight by a non-ASCII character (found by `fuzz/`): imap-proto
/// sliced inside the character and panicked, so a server could crash the client.
#[test]
fn a_code_right_before_non_ascii_text_never_panics() {
    let c = Client::from_stream(Script::new(b"* OK [CAPABILITY IMAP4rev1]\xd7\n")).unwrap();
    assert!(c.has("IMAP4REV1"));
    let mut c = Client::from_stream(Script::new("* OK hi\r\nM1 NO [ALERT]é quota full\r\n".as_bytes())).unwrap();
    assert!(matches!(c.noop(), Err(NetError::No(m)) if m.contains("é quota full")), "the text is kept whole");
    let mut c = Client::from_stream(Script::new("* OK hi\r\n+ [X]ü\r\nM1 OK [APPENDUID 7 9]ö\r\n".as_bytes())).unwrap();
    assert_eq!(c.append("Drafts", "", b"x").unwrap(), Some(9));
    let mut c = Client::from_stream(Script::new(b"* OK hi\r\n+[UIDNEXT 9]\xff\nM1 OK done\r\n")).unwrap();
    assert!(c.append("Drafts", "", b"x").is_ok(), "a continuation without its space");
    // Inside a quoted string the bracket isn't a code: left as it was.
    assert_eq!(super::conn::space_after_code(b"* 1 FETCH (X \"[a]\xc3\xa9\")\r\n".to_vec()), b"* 1 FETCH (X \"[a]\xc3\xa9\")\r\n");
}
