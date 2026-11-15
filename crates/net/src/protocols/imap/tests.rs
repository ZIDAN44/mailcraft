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
    assert_eq!(parse_internal_date("31-Dec-9999 23:59:59 +0000"), Some(253_402_300_799));
    assert_eq!(parse_internal_date("01-Jan-0001 00:00:00 +0000"), Some(-62_135_596_800));
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

/// A password that can't be quoted (not ASCII) goes as a literal: its bytes are sent only once
/// the server asks for them with `+`. A server that answers the command instead has refused the
/// sign-in, and never sees the password.
#[test]
fn a_literal_login_needs_its_continuation() {
    let refused = b"* OK [CAPABILITY IMAP4rev1 MOVE UIDPLUS] ready\r\nM1 NO not like that\r\n";
    let mut c = Client::from_stream(Script::new(refused)).unwrap();
    assert!(c.has("move") && !c.has("CONDSTORE"), "the greeting's capabilities");
    assert!(matches!(c.login("zoe@x", "pässword"), Err(NetError::Auth(m)) if m == "not like that"));
    assert_eq!(c.conn.into_inner().written, b"M1 LOGIN \"zoe@x\" {9}\r\n", "the password held back");

    let server = b"* OK [CAPABILITY IMAP4rev1 MOVE UIDPLUS] ready\r\n+ go on\r\nM1 OK [CAPABILITY IMAP4rev1 MOVE UIDPLUS CONDSTORE] logged in\r\n";
    let mut c = Client::from_stream(Script::new(server)).unwrap();
    c.login("zoe@x", "pässword").unwrap();
    assert!(c.has("CONDSTORE"), "the capabilities said at sign-in");
    assert_eq!(c.conn.into_inner().written, "M1 LOGIN \"zoe@x\" {9}\r\npässword\r\n".as_bytes());
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

/// Found in review: bare LF line ends were mended across the whole response, literals too, so a
/// literal holding bare LFs outgrew its `{n}` and the response was dropped; and a response ending
/// in CRLF was never mended, so a bare LF before its literal stayed.
#[test]
fn bare_lf_line_ends_are_mended_outside_literals_only() {
    let s = b"* OK hi\r\n\
* 1 FETCH (UID 7 BODY[] {6}\r\na\nb\nc\n)\n\
* 2 FETCH (UID 8 BODY[] {3}\nx\r\n)\r\n\
M1 OK done\r\n";
    let mut c = Client::from_stream(Script::new(s)).unwrap();
    let got = c.uid_fetch("7:8", "UID BODY.PEEK[]", "").unwrap();
    let bodies: Vec<(u32, Option<&[u8]>)> = got.iter().map(|f| (f.uid, f.body.as_deref())).collect();
    assert_eq!(bodies, [(7, Some(&b"a\nb\nc\n"[..])), (8, Some(&b"x\r\n"[..]))]);
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

/// An INTERNALDATE with a year far out of range (found in review: the day count overflowed, a
/// panic in debug builds and a garbage date in release) or a negative time is refused.
#[test]
fn a_hostile_internal_date_is_refused() {
    for bad in [
        "01-Jan-9000000000000000000 00:00:00 +0000",
        "01-Jan--9000000000000000000 00:00:00 +0000",
        "01-Jan-10000 00:00:00 +0000",
        "01-Jan-0 00:00:00 +0000",
        "01-Mar-9223372036854775807 00:00:00 +0000",
        "01-Jan-2026 -1:00:00 +0000",
        "01-Jan-2026 10:-5:00 +0000",
        "01-Jan-2026 10:00:-1 +0000",
        "01-Jan-2026 +1:00:00 +0000",
        "01-Jan-2026 10:00:00 +00-5",
    ] {
        assert_eq!(parse_internal_date(bad), None, "{bad}");
    }
    let s = b"* OK hi\r\n* 1 FETCH (UID 5 INTERNALDATE \"01-Jan-9000000000000000000 00:00:00 +0000\")\r\nM1 OK done\r\n";
    let mut c = Client::from_stream(Script::new(s)).unwrap();
    let got = c.uid_fetch("5", "UID INTERNALDATE", "").unwrap();
    assert!(got.iter().all(|f| f.internal_date.is_none()));
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

/// Responses are framed the same however the server's bytes come: each look at the buffer takes
/// up where the last stopped, inside a line, before a literal or inside one.
#[test]
fn responses_are_framed_the_same_however_the_bytes_come() {
    let s = b"* OK hi\r\n\
* 1 FETCH (UID 7 BODY[] {12}\r\nSubject: x\r\n BODY[HEADER] {3}\r\nabc)\r\n\
* SEARCH 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16\r\n\
* 2 FETCH (BODY[] {0}\r\n)\r\n\
M1 OK done\r\n";
    let frames = |step: usize| {
        let mut c = super::conn::Conn::new(Script { step, ..Script::new(s) });
        std::iter::from_fn(|| c.read_raw().ok()).collect::<Vec<_>>()
    };
    let whole = frames(s.len());
    assert_eq!(whole.len(), 5);
    assert_eq!(whole.concat(), s);
    for step in 1..=40 {
        assert_eq!(frames(step), whole, "{step} bytes a read");
    }
}

/// A line whose literal hasn't all come is read once: as the literal's bytes arrive only they are
/// waited for (the line, up to `MAX_LINE` long, was searched from its first byte at every read).
/// Shown by blanking the line in the buffer once it was read: the response is framed all the same.
#[test]
fn a_line_waiting_for_its_literal_is_not_read_again() {
    let line = b"* 1 FETCH (UID 7 BODY[] {10}\r\n";
    let mut c = super::conn::Conn::new(Script { step: 1 << 10, ..Script::new(&[&line[..], b"ab"].concat()) });
    assert!(matches!(c.read_raw(), Err(NetError::Closed(_))), "the literal isn't whole yet");
    c.buf.get_mut(c.pos..c.pos + line.len()).unwrap().fill(b'x');
    c.buf.extend_from_slice(b"\ncdefghi)\r\n* 2 EXPUNGE\r\n");
    assert_eq!(c.read_raw().unwrap().len(), line.len() + 10 + 3);
    assert_eq!(c.read_raw().unwrap(), b"* 2 EXPUNGE\r\n");
    // The command's byte budget is still asked about as the literal comes.
    let mut c = super::conn::Conn::new(Script { step: 1 << 10, ..Script::new(&[&line[..], b"ab"].concat()) });
    assert!(matches!(c.read_raw(), Err(NetError::Closed(_))));
    c.set_limits(super::conn::Limits { bytes: line.len() + 9, ..Default::default() });
    assert!(matches!(c.read_raw(), Err(NetError::Protocol(_))));
}

/// How long the longest answer `MAX_LINE` is for takes to frame: a folder's `UID SEARCH ALL` of
/// three million messages on one line, read as a socket hands it over, 32 KB at a time.
#[test]
#[ignore = "a measurement"]
fn measure_a_huge_search_line() {
    let mut s = b"* SEARCH".to_vec();
    for u in 1..=3_000_000u32 {
        s.extend(format!(" {u}").as_bytes());
    }
    s.extend(b"\r\n");
    let len = s.len();
    let mut c = super::conn::Conn::new(Script { step: 32 << 10, ..Script::new(&s) });
    let began = std::time::Instant::now();
    let raw = c.read_raw().unwrap();
    eprintln!("MEASURE a {} MB search line framed in {:?}", len >> 20, began.elapsed());
    assert_eq!(raw.len(), len);
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

/// Found against a real server: a folder it lists as `"Back\\slash"` or `"Say \"hi\""` was
/// shown, and asked for, with its backslashes still in (imap-proto hands quoted strings back
/// unread), so it couldn't be opened or filed into. The delimiter stays as the parser wants it.
#[test]
fn a_quoted_name_is_read_without_its_escapes() {
    let server = br#"* OK hi
* LIST (\HasNoChildren) "\\" "Back\\slash"
* LIST (\HasNoChildren) "/" "Say \"hi\""
* LIST (\HasNoChildren) "/" "\\"
* LIST (\HasNoChildren) "/" "Plain name"
* LIST (\Noselect) NIL "Flat\\name"
* LIST (\HasNoChildren) "/" {5}
Lit\"
M1 OK done
"#;
    let mut c = Client::from_stream(Script::new(server)).unwrap();
    let folders = c.list("a").unwrap();
    let names: Vec<&str> = folders.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(names, [r"Back\slash", r#"Say "hi""#, r"\", "Plain name", r"Flat\name", r#"Lit\""#]);
    assert_eq!(folders[0].delimiter, Some('\\'));
    // Asked for by the name it has, escaped once.
    let mut c = Client::from_stream(Script::new(b"* OK hi\r\nM1 OK done\r\n")).unwrap();
    c.create(r"Back\slash").unwrap();
    let written = String::from_utf8_lossy(&c.conn_mut().get_mut().written).to_string();
    assert!(written.contains(r#"CREATE "Back\\slash""#), "{written}");

    use super::quotes::plain_quotes;
    let same = |raw: &[u8]| assert_eq!(plain_quotes(raw.to_vec()), raw, "{}", String::from_utf8_lossy(raw));
    // A status response's text isn't strings; nor is a quote that never closes; a literal's
    // bytes are the message's own; a delimiter outside a listing is one character.
    same(br#"M1 NO "a\\b" isn't there"#);
    same(br#"+ "a\\b""#);
    same(br#"* 1 FETCH (X "a\\b)"#);
    same(b"* 1 FETCH (BODY[] {8}\r\n\"a\\\\b\" x)\r\n");
    let mixed = [&br#"* 1 FETCH (X ("a \"b\".pdf" "c") BODY[] {3}"#[..], b"\r\n", br#""\\ Y "d\\e")"#, b"\r\n"].concat();
    let fixed = [&b"* 1 FETCH (X ({9}\r\n"[..], br#"a "b".pdf "c") BODY[] {3}"#, b"\r\n", br#""\\ Y {3}"#, b"\r\n", br"d\e)", b"\r\n"].concat();
    assert_eq!(plain_quotes(mixed), fixed);
    // Junk never panics.
    for junk in [&br"\"[..], br#""\"#, br#"* LIST "\""#, b"* X \"\\\r\n", br#""\"\"\""#, b"* X {99999999999}\r\n\"\\\\\""] {
        let _ = plain_quotes(junk.to_vec());
    }
}

/// A server that answers in its own language: a completion in words outside ASCII is the
/// command done all the same, and what its codes say is read.
#[test]
fn a_completion_in_another_language_is_still_done() {
    let mut c = Client::from_stream(Script::new("* OK hi\r\nM1 OK Terminé avec succès\r\n".as_bytes())).unwrap();
    assert!(c.noop().is_ok());
    let mut c = Client::from_stream(Script::new("* OK hi\r\n+ go\r\nM1 OK [APPENDUID 7 9] gespeichert für später\r\n".as_bytes())).unwrap();
    assert_eq!(c.append("Drafts", "", b"x").unwrap(), Some(9));
    let server =
        "* OK hi\r\n* 3 EXISTS\r\n* OK [UIDVALIDITY 5] gültig\r\n* OK [UIDNEXT 9] nächste\r\nM1 OK [READ-WRITE] ausgewählt für Änderungen\r\n";
    let mut c = Client::from_stream(Script::new(server.as_bytes())).unwrap();
    let info = c.select("INBOX", false).unwrap();
    assert_eq!((info.exists, info.validity, info.uid_next), (3, 5, 9));
}
