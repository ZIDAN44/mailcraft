//! SMTP on the wire: replies read as servers write them (one line or several, words in another
//! character set, too long, cut off), what an `EHLO` offers, and the message's lines as `DATA`
//! sends them, read back as a server reads them.

use proptest::prelude::*;

use super::wire::{Offers, Reply, data_lines, parse_line, read_data, read_reply};
use crate::NetError;

fn read(bytes: &[u8]) -> crate::Result<Reply> {
    read_reply(&mut &bytes[..])
}

#[test]
fn a_reply_of_one_line_or_several_is_read_whole() {
    let r = read(b"250 2.0.0 fine\r\n").unwrap();
    assert_eq!((r.code, r.said()), (250, "250 2.0.0 fine".to_string()));
    let r = read(b"250-mail.example greets you\r\n250-SIZE 35882577\r\n250-8BITMIME\r\n250 SMTPUTF8\r\nleft for later").unwrap();
    assert_eq!(r.lines, ["mail.example greets you", "SIZE 35882577", "8BITMIME", "SMTPUTF8"]);
    // A line end of LF alone, and a code with no words.
    assert_eq!(read(b"354\n").unwrap().said(), "354");
    assert!(read(b"220 ready\r\n").unwrap().done() && !read(b"451 later\r\n").unwrap().done());
}

/// A server set up in Latin-1 (discussion #309): its words are read, what isn't UTF-8 replaced.
#[test]
fn words_that_arent_utf8_are_read() {
    let r = read(b"550 5.1.1 Empf\xE4nger unbekannt\r\n").unwrap();
    assert_eq!(r.code, 550);
    assert!(r.said().starts_with("550 5.1.1 Empf") && r.said().ends_with("nger unbekannt"), "{}", r.said());
}

#[test]
fn what_isnt_an_smtp_reply_is_refused() {
    for bad in [&b"HTTP/1.1 400 Bad Request\r\n"[..], b"25\r\n", b"250x\r\n", b"650 no\r\n", b"099 no\r\n", b"250-one\r\n251 two\r\n"] {
        assert!(matches!(read(bad), Err(NetError::Protocol(_))), "{:?}", String::from_utf8_lossy(bad));
    }
    // Cut off: the connection ended, which isn't the server's word.
    for cut in [&b""[..], b"250-more\r\n", b"250 no line end"] {
        assert!(matches!(read(cut), Err(NetError::Io(_))), "{:?}", String::from_utf8_lossy(cut));
    }
    // Too long a line, or too many.
    let long = [b"250 ".to_vec(), vec![b'x'; 70_000], b"\r\n".to_vec()].concat();
    assert!(matches!(read(&long), Err(NetError::Protocol(_))));
    let many = "250-x\r\n".repeat(2000) + "250 x\r\n";
    assert!(matches!(read(many.as_bytes()), Err(NetError::Protocol(_))));
}

#[test]
fn the_lines_of_a_reply() {
    assert_eq!(parse_line("250 ok"), Some((250, false, "ok")));
    assert_eq!(parse_line("250-ok"), Some((250, true, "ok")));
    assert_eq!(parse_line("221"), Some((221, false, "")));
    assert_eq!(parse_line("2é0 ok"), None);
    assert_eq!(parse_line(""), None);
}

/// What an `EHLO` offers: its extensions, after the line that greets; sign-in methods from
/// `AUTH` and the older `AUTH=`, each once.
#[test]
fn what_ehlo_offers() {
    let r = read(b"250-mail.example\r\n250-STARTTLS\r\n250-AUTH PLAIN LOGIN\r\n250-AUTH=LOGIN xoauth2\r\n250 8bitmime\r\n").unwrap();
    let o = Offers::of(&r);
    assert!(o.has("STARTTLS") && o.has("8BITMIME") && !o.has("SMTPUTF8") && !o.has("MAIL.EXAMPLE"));
    assert_eq!(o.sign_ins(), ["PLAIN", "LOGIN", "XOAUTH2"]);
    assert!(Offers::of(&read(b"250 only greets\r\n").unwrap()).sign_ins().is_empty());
}

/// A message's lines as `DATA` sends them: CR LF line ends, a dot at a line's start doubled, and
/// a lone dot's line at the end.
#[test]
fn a_message_is_sent_with_its_dots_doubled() {
    assert_eq!(data_lines(b"Subject: s\r\n\r\n.hidden\r\n..two\r\nend"), b"Subject: s\r\n\r\n..hidden\r\n...two\r\nend\r\n.\r\n".to_vec());
    // Bare line ends, which servers turn away, become CR LF.
    assert_eq!(data_lines(b"a\nb\rc\r\n"), b"a\r\nb\r\nc\r\n.\r\n".to_vec());
    assert_eq!(data_lines(b""), b".\r\n".to_vec());
    assert_eq!(data_lines(b".\r\n"), b"..\r\n.\r\n".to_vec());
}

proptest! {
    /// Any message with CR LF line ends comes back from the server as it was sent, ending CR LF.
    #[test]
    fn a_message_reaches_the_server_as_it_was(lines in proptest::collection::vec("[ -~]{0,20}", 0..12)) {
        let raw = lines.iter().map(|l| format!("{l}\r\n")).collect::<String>();
        let sent = data_lines(raw.as_bytes());
        prop_assert_eq!(read_data(&sent), Some(raw.into_bytes()));
    }
}
