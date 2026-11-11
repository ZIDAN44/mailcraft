use super::*;

/// A canned server: reads are served from `input`; writes are recorded.
pub(crate) struct Script {
    input: io::Cursor<Vec<u8>>,
    pub written: Vec<u8>,
    /// The most bytes one read gives.
    step: usize,
}

impl Script {
    fn new(s: &[u8]) -> Self {
        // A few bytes at a time, to exercise framing across reads.
        Script { input: io::Cursor::new(s.to_vec()), written: Vec::new(), step: 7 }
    }
}

impl Read for Script {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = buf.len().min(self.step);
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

/// A response imap-proto panics on (`INTERNALDATE NIL`, found in review: it unwraps the date)
/// is skipped like any other it doesn't understand, and the command carries on.
#[test]
fn a_response_the_parser_panics_on_is_skipped() {
    let s = b"* OK hi\r\n* 1 FETCH (UID 5 FLAGS (\\Seen) INTERNALDATE NIL)\r\n* 2 FETCH (UID 6 FLAGS (\\Seen))\r\nM1 OK done\r\n";
    let mut c = Client::from_stream(Script::new(s)).unwrap();
    let got = c.uid_fetch("5:6", "UID FLAGS INTERNALDATE", "").unwrap();
    assert_eq!(got.iter().map(|f| f.uid).collect::<Vec<_>>(), [6]);
}

/// A signed or encrypted message never comes as its text first (only its whole source shows it
/// and checks it); nor one of a single part. Neither asks for more than its structure.
#[test]
fn signed_and_single_part_mail_comes_whole() {
    for structure in [
        r#"(("TEXT" "PLAIN" NIL NIL NIL "7BIT" 10 1)("APPLICATION" "PKCS7-SIGNATURE" NIL NIL NIL "BASE64" 900000) "SIGNED" NIL NIL NIL)"#,
        r#"("APPLICATION" "PKCS7-MIME" ("SMIME-TYPE" "enveloped-data") NIL NIL "BASE64" 900000)"#,
        r#"(("TEXT" "PLAIN" NIL NIL NIL "7BIT" 10 1)("APPLICATION" "PKCS7-MIME" NIL NIL NIL "BASE64" 900000) "MIXED" NIL NIL NIL)"#,
        r#"("TEXT" "PLAIN" NIL NIL NIL "7BIT" 900000 1)"#,
    ] {
        let s = format!("* OK ready\r\n* 1 FETCH (UID 1 BODYSTRUCTURE {structure})\r\nM1 OK done\r\n");
        let mut c = Client::from_stream(Script::new(s.as_bytes())).unwrap();
        assert_eq!(c.text_first(1).unwrap(), None, "{structure}");
        let sent = String::from_utf8_lossy(&c.conn.into_inner().written).into_owned();
        assert!(!sent.contains("BODY.PEEK"), "{sent}");
    }
}

/// A hostile server's lists nested a hundred thousand deep don't overflow the stack (they did,
/// in imap-proto's parser, from a thousand), even on a thread with Windows' 1 MB main stack;
/// real nesting parses.
#[test]
fn deeply_nested_lists_dont_overflow_the_stack() {
    let h = std::thread::Builder::new().stack_size(1 << 20).spawn(|| {
        let mut out = Vec::new();
        for depth in [30usize, conn::MAX_NESTING - 1, conn::MAX_NESTING + 1, 1000, 100_000] {
            let mut s = b"* OK ready\r\n* 1 FETCH (UID 1 BODYSTRUCTURE ".to_vec();
            s.extend(std::iter::repeat_n(b'(', depth));
            s.extend(b"\"TEXT\" \"PLAIN\" NIL NIL NIL \"7BIT\" 1 1");
            s.extend(std::iter::repeat_n(b')', depth));
            s.extend(b")\r\nM1 OK done\r\n");
            let mut c = Client::from_stream(Script::new(&s)).unwrap();
            out.push((depth, c.uid_fetch("1", "UID BODYSTRUCTURE", "").is_ok()));
        }
        out
    });
    let out = h.unwrap().join().expect("no overflow");
    assert!(out.iter().all(|(_, ok)| *ok), "{out:?}");
    // The count skips quoted strings and literals.
    assert_eq!(conn::nesting(b"* 1 FETCH (UID 1 BODY[] {5}\r\n(((((\r\n FLAGS (\"((\" \\Seen))"), 2);
}

/// A folder's `UID SEARCH ALL` is answered in one line: past 1 MB (some 130,000 messages) it
/// was refused, the connection dropped, and the account signed in again without end.
#[test]
fn a_big_folders_search_line_is_read() {
    let mut s = b"* OK hi\r\n* SEARCH".to_vec();
    for u in 1..=200_000u32 {
        s.extend(format!(" {u}").as_bytes());
    }
    s.extend(b"\r\nM1 OK done\r\n");
    assert!(s.len() > 1 << 20);
    let mut c = Client::from_stream(Script { step: 16 << 10, ..Script::new(&s) }).unwrap();
    assert_eq!(c.uid_search("ALL").unwrap().len(), 200_000);
}

/// A refused SELECT leaves no folder selected (RFC 3501 §6.3.1): the one before was still
/// taken for selected, so IDLE waited on nothing and the Inbox's messages couldn't be opened.
#[test]
fn a_refused_select_leaves_nothing_selected() {
    let mut c = Client::from_stream(Script::new(b"* OK hi\r\n* 3 EXISTS\r\nM1 OK [READ-WRITE] done\r\nM2 NO no such mailbox\r\n")).unwrap();
    c.select("INBOX", false).unwrap();
    assert_eq!(c.selected(), Some("INBOX"));
    assert!(c.select("Gone", false).is_err());
    assert_eq!(c.selected(), None);
}

/// `VANISHED` (QRESYNC's EXPUNGE) lowers the count of messages, so the next one's `EXISTS`
/// is taken for new mail.
#[test]
fn mail_after_a_vanished_message_is_new() {
    let s = b"* OK hi\r\n* 10 EXISTS\r\nM1 OK [READ-WRITE] done\r\n* VANISHED 57,60:61\r\nM2 OK done\r\n* 8 EXISTS\r\nM3 OK done\r\n";
    let mut c = Client::from_stream(Script::new(s)).unwrap();
    c.select("INBOX", false).unwrap();
    c.take_news();
    c.noop().unwrap();
    assert!(!c.take_news());
    c.noop().unwrap();
    assert!(c.take_news(), "7 left, then 8");
}

/// A mailbox a server lists in UTF-8 is named whole (`Büro` went as `Bro`, another mailbox),
/// and never with a line end in it.
#[test]
fn a_utf8_mailbox_name_is_sent_whole() {
    let mut c = Client::from_stream(Script::new(b"* OK hi\r\nM1 NO x\r\nM2 NO x\r\n")).unwrap();
    let _ = c.select("B\u{fc}ro \"x\"", false);
    let _ = c.select("a\r\nM9 DELETE b\u{fc}", false);
    let sent = String::from_utf8_lossy(&c.conn.into_inner().written).into_owned();
    assert_eq!(sent, "M1 SELECT \"B\u{fc}ro \\\"x\\\"\"\r\nM2 SELECT \"aM9 DELETE b\u{fc}\"\r\n");
}

/// Without UIDPLUS, EXPUNGE takes every message marked `\Deleted`: the ones another program
/// marked and left are unmarked while it runs, so only what was asked for goes.
#[test]
fn a_plain_expunge_spares_what_others_marked() {
    let s = b"* OK hi\r\n* SEARCH 4 9\r\nM1 OK done\r\nM2 OK done\r\nM3 OK done\r\nM4 OK done\r\n";
    let mut c = Client::from_stream(Script::new(s)).unwrap();
    c.expunge("7").unwrap();
    let sent = String::from_utf8_lossy(&c.conn.into_inner().written).into_owned();
    let want = "M1 UID SEARCH DELETED NOT UID 7\r\nM2 UID STORE 4,9 -FLAGS.SILENT (\\Deleted)\r\nM3 EXPUNGE\r\nM4 UID STORE 4,9 +FLAGS.SILENT (\\Deleted)\r\n";
    assert_eq!(sent, want);
}

/// A flag change the server tells of while headers are fetched (with QRESYNC it names the
/// UID) is no header: it made a message with no subject or sender, and that message's real
/// header, in a later answer, was skipped.
#[test]
fn a_flag_change_told_in_passing_makes_no_message() {
    let s = b"* OK hi\r\n* 1 FETCH (UID 5 FLAGS () RFC822.SIZE 20 BODY[HEADER] {15}\r\nSubject: five\r\n)\r\n* 9 FETCH (UID 300 FLAGS (\\Seen) MODSEQ (991))\r\nM1 OK done\r\n";
    let mut c = Client::from_stream(Script::new(s)).unwrap();
    let mut st = mailcraft_store::FolderState { validity: 1, ..Default::default() };
    crate::sync::fetch_headers(&mut c, &mut st, "a", "INBOX", &[5], &mut Default::default()).unwrap();
    assert_eq!(st.messages.iter().map(|m| (m.key.uid, m.subject.clone())).collect::<Vec<_>>(), [(5, "five".to_string())]);
}
