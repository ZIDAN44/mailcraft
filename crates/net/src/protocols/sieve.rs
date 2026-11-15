//! Automatic replies on IMAP servers: a small ManageSieve client (RFC 5804) that keeps a Sieve
//! script (RFC 5228) named `mailcraft` with a `vacation` action (RFC 5230), limited to a date
//! range with `currentdate` (RFC 5260) when one is set.
//!
//! The script is MailCraft's own: the settings it was made from are kept in its first line, so
//! reading them back never means parsing Sieve. A script the user already runs stays in charge
//! of their other rules: ours `include`s it (RFC 6609) and becomes the active one; turning the
//! replies off makes theirs active again.
//!
//! Server input is hostile: lines and literals are capped.

use std::io::{BufReader, Read, Write};

use mailcraft_model::job::{Vacation, is_day};
use mailcraft_model::{Security, ServerConfig};

use crate::stream::{self, Stream};
use crate::{NetError, Result};

/// ManageSieve's registered port.
pub const PORT: u16 = 4190;
/// Our script's name.
pub const SCRIPT: &str = "mailcraft";
const MARK: &str = "# mailcraft-vacation ";
const MAX_LINE: usize = 64 << 10;
const MAX_LITERAL: usize = 1 << 20;

/// One server response: data lines before it, and the final `OK`/`NO`/`BYE` with its text.
struct Reply {
    data: Vec<Vec<String>>,
    ok: bool,
    text: String,
}

pub struct Client<S: Read + Write> {
    io: BufReader<S>,
    caps: Vec<(String, String)>,
}

/// A token of a response line: an atom, a string (quoted or literal), or either inside the
/// parentheses of a response code (`NO (QUOTA/MAXSIZE) "…"`, RFC 5804 §1.3).
#[derive(Debug, PartialEq, Eq)]
enum Tok {
    Atom(String),
    Str(String),
    Code(String),
}

impl Tok {
    fn text(self) -> String {
        match self {
            Tok::Atom(s) | Tok::Str(s) | Tok::Code(s) => s,
        }
    }
}

/// A response line's tokens, the literals it announces read too.
fn tokens<S: Read + Write>(io: &mut BufReader<S>, first: &str) -> Result<Vec<Tok>> {
    let mut out = Vec::new();
    let mut line = first.to_string();
    // How deep in a response code's parentheses (they may hold strings, and a literal).
    let mut depth = 0usize;
    let tok = |depth: usize, s: String, quoted: bool| {
        if depth > 0 {
            Tok::Code(s)
        } else if quoted {
            Tok::Str(s)
        } else {
            Tok::Atom(s)
        }
    };
    loop {
        let mut chars = line.char_indices().peekable();
        let mut literal: Option<usize> = None;
        while let Some((i, c)) = chars.next() {
            match c {
                ' ' | '\t' | '\r' | '\n' => {}
                '"' => {
                    let mut s = String::new();
                    loop {
                        match chars.next() {
                            Some((_, '\\')) => {
                                if let Some((_, e)) = chars.next() {
                                    s.push(e);
                                }
                            }
                            Some((_, '"')) | None => break,
                            Some((_, c)) => s.push(c),
                        }
                    }
                    out.push(tok(depth, s, true));
                }
                '{' => {
                    let rest = line.get(i + 1..).unwrap_or("");
                    let n: String = rest.chars().take_while(char::is_ascii_digit).collect();
                    literal = Some(n.parse().map_err(|_| NetError::Protocol("ManageSieve: bad literal".into()))?);
                    break;
                }
                '(' => depth = depth.saturating_add(1),
                ')' => depth = depth.saturating_sub(1),
                _ => {
                    let mut atom = String::from(c);
                    while let Some(&(_, c)) = chars.peek() {
                        if c == ' ' || c == '\r' || c == '\n' || c == '(' || c == ')' {
                            break;
                        }
                        atom.push(c);
                        chars.next();
                    }
                    out.push(tok(depth, atom, false));
                }
            }
        }
        let Some(n) = literal else { return Ok(out) };
        if n > MAX_LITERAL {
            return Err(NetError::Protocol("ManageSieve: literal too large".into()));
        }
        let mut buf = vec![0u8; n];
        io.read_exact(&mut buf)?;
        out.push(tok(depth, String::from_utf8_lossy(&buf).into_owned(), true));
        line = read_line(io)?;
    }
}

fn read_line<S: Read + Write>(io: &mut BufReader<S>) -> Result<String> {
    match super::capped_line(io, MAX_LINE)? {
        super::Capped::Line(buf) => Ok(String::from_utf8_lossy(&buf).into_owned()),
        super::Capped::Closed => Err(NetError::Closed("the ManageSieve server closed the connection".into())),
        super::Capped::TooLong => Err(NetError::Protocol("ManageSieve: line too long".into())),
    }
}

fn reply<S: Read + Write>(io: &mut BufReader<S>) -> Result<Reply> {
    let mut data = Vec::new();
    loop {
        let line = read_line(io)?;
        let t = tokens(io, &line)?;
        // Only an atom ends the reply: a string is data (a script named "ok").
        let head = match t.first() {
            Some(Tok::Atom(a)) => a.to_ascii_uppercase(),
            _ => String::new(),
        };
        match head.as_str() {
            "OK" | "NO" | "BYE" => {
                // The response code left out: the rest is the human text, upper case or not.
                let text = t.into_iter().skip(1).filter(|x| !matches!(x, Tok::Code(_))).map(Tok::text).collect::<Vec<_>>().join(" ");
                if head == "BYE" {
                    return Err(NetError::Closed(text));
                }
                return Ok(Reply { data, ok: head == "OK", text });
            }
            _ => {
                if data.len() > 10_000 {
                    return Err(NetError::Protocol("ManageSieve: too many lines".into()));
                }
                data.push(t.into_iter().map(Tok::text).collect());
            }
        }
    }
}

// A ManageSieve quoted string.
use super::quote as q;

impl<S: Read + Write> Client<S> {
    /// Read the greeting (the server's capabilities).
    pub fn from_stream(s: S) -> Result<Client<S>> {
        let mut c = Client { io: BufReader::new(s), caps: Vec::new() };
        c.read_caps()?;
        Ok(c)
    }

    fn read_caps(&mut self) -> Result<()> {
        let r = reply(&mut self.io)?;
        if !r.ok {
            return Err(NetError::No(r.text));
        }
        self.caps = r.data.into_iter().filter_map(|t| Some((t.first()?.to_ascii_uppercase(), t.get(1).cloned().unwrap_or_default()))).collect();
        Ok(())
    }

    fn cap(&self, name: &str) -> Option<&str> {
        self.caps.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    /// Whether the server's Sieve has an extension.
    pub fn has_extension(&self, ext: &str) -> bool {
        self.cap("SIEVE").is_some_and(|v| v.split_whitespace().any(|e| e.eq_ignore_ascii_case(ext)))
    }

    fn run(&mut self, cmd: &str) -> Result<Reply> {
        let w = self.io.get_mut();
        w.write_all(cmd.as_bytes())?;
        w.write_all(b"\r\n")?;
        w.flush()?;
        reply(&mut self.io)
    }

    fn run_ok(&mut self, cmd: &str) -> Result<Reply> {
        let r = self.run(cmd)?;
        if r.ok { Ok(r) } else { Err(NetError::No(r.text)) }
    }

    pub fn login(&mut self, user: &str, pass: &str) -> Result<()> {
        let token = mailcraft_model::base64::encode(format!("\0{user}\0{pass}").as_bytes());
        let r = self.run(&format!("AUTHENTICATE \"PLAIN\" {}", q(&token)))?;
        if r.ok { Ok(()) } else { Err(NetError::Auth(if r.text.is_empty() { "the Sieve server refused the password".into() } else { r.text })) }
    }

    /// Sign in with an OAuth token: `xoauth2` is the initial response in base64 already, sent as
    /// IMAP sends it.
    pub fn login_xoauth2(&mut self, user: &str, token: &str) -> Result<()> {
        let r = self.run(&format!("AUTHENTICATE \"XOAUTH2\" {}", q(&crate::oauth::xoauth2(user, token))))?;
        if r.ok { Ok(()) } else { Err(NetError::Auth(r.text)) }
    }

    /// Script names, and which one is active.
    pub fn list(&mut self) -> Result<(Vec<String>, Option<String>)> {
        let r = self.run_ok("LISTSCRIPTS")?;
        let mut names = Vec::new();
        let mut active = None;
        for t in r.data {
            let Some(n) = t.first() else { continue };
            if t.get(1).is_some_and(|a| a.eq_ignore_ascii_case("ACTIVE")) {
                active = Some(n.clone());
            }
            names.push(n.clone());
        }
        Ok((names, active))
    }

    pub fn get(&mut self, name: &str) -> Result<String> {
        let r = self.run_ok(&format!("GETSCRIPT {}", q(name)))?;
        Ok(r.data.into_iter().next().and_then(|t| t.into_iter().next()).unwrap_or_default())
    }

    pub fn put(&mut self, name: &str, script: &str) -> Result<()> {
        let cmd = format!("PUTSCRIPT {} {{{}+}}\r\n{script}", q(name), script.len());
        self.run_ok(&cmd).map(|_| ())
    }

    /// Make a script the active one (`""`: none).
    pub fn activate(&mut self, name: &str) -> Result<()> {
        self.run_ok(&format!("SETACTIVE {}", q(name))).map(|_| ())
    }

    pub fn logout(&mut self) {
        let _ = self.run("LOGOUT");
    }

    /// The automatic replies this script holds (the default when never set up here).
    pub fn vacation(&mut self) -> Result<Vacation> {
        let (names, active) = self.list()?;
        if !names.iter().any(|n| n == SCRIPT) {
            return Ok(Vacation::default());
        }
        let mut v = read_script(&self.get(SCRIPT)?).unwrap_or_default();
        // Someone switched our script off elsewhere: the replies don't run.
        v.enabled &= active.as_deref() == Some(SCRIPT);
        Ok(v)
    }

    /// Store the automatic replies and switch them on or off.
    pub fn set_vacation(&mut self, v: &Vacation) -> Result<()> {
        if v.enabled && !self.has_extension("vacation") {
            return Err(NetError::No("this server's Sieve has no automatic replies (vacation)".into()));
        }
        let (names, active) = self.list()?;
        // The script the user had before ours (kept when we last wrote ours).
        let previous = if names.iter().any(|n| n == SCRIPT) { read_previous(&self.get(SCRIPT)?) } else { None };
        let theirs = match active.as_deref() {
            Some(a) if a != SCRIPT => Some(a.to_string()),
            _ => previous.filter(|p| names.contains(p)),
        };
        if theirs.is_some() && v.enabled && !self.has_extension("include") {
            return Err(NetError::No(
                "another Sieve script runs your filters and this server can't combine scripts (include); turn it off first".into(),
            ));
        }
        let dated = v.from.is_some() || v.until.is_some();
        if v.enabled && dated && !self.has_extension("date") {
            return Err(NetError::No("this server can't limit automatic replies to dates".into()));
        }
        self.put(SCRIPT, &script(v, theirs.as_deref()))?;
        if v.enabled {
            self.activate(SCRIPT)
        } else {
            // Off: the user's own script runs again (or none).
            self.activate(theirs.as_deref().unwrap_or(""))
        }
    }
}

impl Client<Stream> {
    /// Connect to the account's Sieve server: the IMAP host on port 4190, encrypted with
    /// STARTTLS unless the account is plain (local test servers). `trusted` is what the account
    /// trusts besides the usual authorities ([`stream::tls_trusting`]).
    pub fn connect_trusting(imap: &ServerConfig, port: u16, trusted: stream::Trusted<'_>) -> Result<Client<Stream>> {
        let sock = stream::tcp(&imap.host, port)?;
        let mut c = Client::from_stream(Stream::Plain(sock))?;
        if imap.security == Security::None {
            return Ok(c);
        }
        if c.cap("STARTTLS").is_none() {
            return Err(NetError::Tls("the Sieve server does not offer STARTTLS".into()));
        }
        c.run_ok("STARTTLS")?;
        let unread = c.io.buffer().len();
        // The server repeats its capabilities over TLS (RFC 5804 §2.2); the earlier ones are void.
        Client::from_stream(stream::starttls(c.io.into_inner(), unread, &imap.host, trusted)?)
    }
}

/// A Sieve string: quoted, with `\` and `"` escaped (line breaks are allowed inside).
fn sieve_str(s: &str) -> String {
    q(&s.replace('\r', ""))
}

/// Our script for these settings. The settings themselves (and the user's own script, which it
/// includes) are kept in the first lines.
pub fn script(v: &Vacation, include: Option<&str>) -> String {
    let settings = serde_json::to_string(v).unwrap_or_default().replace(['\r', '\n'], " ");
    let mut out = format!("{MARK}{settings}\r\n");
    if let Some(p) = include {
        out.push_str(&format!("# mailcraft-previous {}\r\n", p.replace(['\r', '\n'], " ")));
    }
    let from = v.from.as_deref().filter(|d| is_day(d));
    let until = v.until.as_deref().filter(|d| is_day(d));
    let mut req = vec!["vacation"];
    if include.is_some() {
        req.push("include");
    }
    if from.is_some() || until.is_some() {
        req.extend(["date", "relational"]);
    }
    if v.enabled {
        out.push_str(&format!("require [{}];\r\n", req.iter().map(|r| sieve_str(r)).collect::<Vec<_>>().join(", ")));
    } else if include.is_some() {
        out.push_str("require [\"include\"];\r\n");
    }
    if let Some(p) = include {
        out.push_str(&format!("include :personal {};\r\n", sieve_str(p)));
    }
    if v.enabled {
        let mut tests = Vec::new();
        if let Some(d) = from {
            tests.push(format!("currentdate :value \"ge\" \"date\" {}", sieve_str(d)));
        }
        if let Some(d) = until {
            tests.push(format!("currentdate :value \"le\" \"date\" {}", sieve_str(d)));
        }
        let subject = if v.subject.trim().is_empty() { String::new() } else { format!(":subject {} ", sieve_str(v.subject.trim())) };
        let action = format!("vacation :days 1 {subject}{};\r\n", sieve_str(&v.text));
        if tests.is_empty() {
            out.push_str(&action);
        } else {
            out.push_str(&format!("if allof({}) {{\r\n    {action}}}\r\n", tests.join(", ")));
        }
    }
    out
}

/// The settings our script was made from.
pub fn read_script(s: &str) -> Option<Vacation> {
    let first = s.lines().next()?;
    serde_json::from_str(first.strip_prefix(MARK)?.trim()).ok()
}

fn read_previous(s: &str) -> Option<String> {
    s.lines().take(3).find_map(|l| l.strip_prefix("# mailcraft-previous ").map(|p| p.trim().to_string())).filter(|p| !p.is_empty())
}

#[cfg(test)]
pub(crate) mod tests {
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    use super::*;

    #[test]
    fn scripts() {
        let v = Vacation {
            enabled: true,
            subject: "Away \"now\"".into(),
            text: "Back on Monday.\nZoe".into(),
            from: Some("2026-10-15".into()),
            until: None,
            ..Default::default()
        };
        let s = script(&v, Some("filters"));
        assert!(s.starts_with(MARK));
        assert!(s.contains(r#"require ["vacation", "include", "date", "relational"];"#), "{s}");
        assert!(s.contains("include :personal \"filters\";"));
        assert!(s.contains(r#"if allof(currentdate :value "ge" "date" "2026-10-15") {"#));
        assert!(
            s.contains(
                r#":subject "Away \"now\"" "Back on Monday.
Zoe";"#
            ),
            "{s}"
        );
        assert_eq!(read_script(&s), Some(v.clone()));
        assert_eq!(read_previous(&s).as_deref(), Some("filters"));
        let off = script(&Vacation { enabled: false, ..v }, None);
        assert!(!off.contains("vacation :days"), "off: no action");
        assert_eq!(read_script("require \"vacation\";"), None);
        // Settings never break out of the comment line.
        let evil = Vacation { enabled: true, subject: "x\r\nrequire \"fileinto\";".into(), text: "t".into(), ..Default::default() };
        assert_eq!(script(&evil, None).lines().next().unwrap().matches("require").count(), 1, "still one comment line");
    }

    /// A server's answers, canned; what's sent to it is dropped.
    struct Canned(std::io::Cursor<Vec<u8>>);

    impl Read for Canned {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.0.read(buf)
        }
    }

    impl Write for Canned {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// Found in review: a reply ended at any line whose first token read `OK`, `NO` or `BYE`,
    /// quoted or not, so scripts named "ok", "no" or "bye" ended LISTSCRIPTS early and the lines
    /// after were taken for the next command's answer; and every upper-case word was dropped
    /// from a reply's text as if it were a response code.
    #[test]
    fn only_an_atom_ends_a_reply_and_only_a_code_is_left_out_of_its_text() {
        let s = b"\"SIEVE\" \"vacation\"\r\nOK\r\n\
\"ok\"\r\n\"no\" ACTIVE\r\n\"bye\"\r\nOK\r\n\
NO (QUOTA/MAXSIZE \"x\") \"LOCKED\"\r\n\
NO {6}\r\nFAILED\r\n";
        let mut c = Client::from_stream(Canned(std::io::Cursor::new(s.to_vec()))).unwrap();
        assert_eq!(c.list().unwrap(), (vec!["ok".to_string(), "no".into(), "bye".into()], Some("no".to_string())));
        assert!(matches!(c.activate("x"), Err(NetError::No(m)) if m == "LOCKED"));
        assert!(matches!(c.activate("x"), Err(NetError::No(m)) if m == "FAILED"));
    }

    /// A ManageSieve server for tests: scripts in memory, one connection at a time.
    pub(crate) struct Fake {
        pub scripts: Vec<(String, String)>,
        pub active: Option<String>,
        pub sieve: &'static str,
        pub log: Vec<String>,
    }

    pub(crate) fn serve(fake: Fake) -> (u16, Arc<Mutex<Fake>>) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let st = Arc::new(Mutex::new(fake));
        let shared = st.clone();
        std::thread::spawn(move || {
            for conn in l.incoming() {
                let Ok(mut s) = conn else { continue };
                let mut r = BufReader::new(s.try_clone().unwrap());
                let caps = |s: &mut std::net::TcpStream, sieve: &str| {
                    let _ = write!(
                        s,
                        "\"IMPLEMENTATION\" \"Fake\"\r\n\"SASL\" \"PLAIN XOAUTH2\"\r\n\"SIEVE\" \"{sieve}\"\r\n\"VERSION\" \"1.0\"\r\nOK \"ready\"\r\n"
                    );
                };
                caps(&mut s, shared.lock().unwrap().sieve);
                loop {
                    let mut line = String::new();
                    if r.read_line(&mut line).unwrap_or(0) == 0 {
                        break;
                    }
                    let mut f = shared.lock().unwrap();
                    f.log.push(line.trim_end().to_string());
                    let arg = |n: usize| line.split('"').nth(n).unwrap_or("").to_string();
                    let up = line.to_ascii_uppercase();
                    if up.starts_with("AUTHENTICATE") {
                        // PLAIN with the password, or XOAUTH2 with the token (its initial
                        // response the SASL string in base64, RFC 5804 §2.1).
                        let good = if up.contains("\"XOAUTH2\"") {
                            format!("\"{}\"", mailcraft_model::base64::encode(b"user=me@studio.example\x01auth=Bearer token\x01\x01"))
                        } else {
                            mailcraft_model::base64::encode(b"\0me@studio.example\0pw")
                        };
                        let _ = s.write_all(if line.contains(&good) { b"OK\r\n" as &[u8] } else { b"NO \"wrong password\"\r\n" });
                    } else if up.starts_with("LISTSCRIPTS") {
                        for (n, _) in &f.scripts {
                            let act = if f.active.as_deref() == Some(n.as_str()) { " ACTIVE" } else { "" };
                            let _ = write!(s, "\"{n}\"{act}\r\n");
                        }
                        let _ = s.write_all(b"OK\r\n");
                    } else if up.starts_with("GETSCRIPT") {
                        let name = arg(1);
                        match f.scripts.iter().find(|(n, _)| *n == name) {
                            Some((_, body)) => {
                                let _ = write!(s, "{{{}}}\r\n{body}\r\nOK\r\n", body.len());
                            }
                            None => {
                                let _ = s.write_all(b"NO \"no such script\"\r\n");
                            }
                        }
                    } else if up.starts_with("PUTSCRIPT") {
                        let name = arg(1);
                        let n: usize = line.rsplit('{').next().unwrap_or("").trim_end_matches(['\r', '\n', '}', '+']).parse().unwrap_or(0);
                        let mut body = vec![0u8; n];
                        r.read_exact(&mut body).unwrap();
                        let mut end = String::new();
                        let _ = r.read_line(&mut end);
                        let body = String::from_utf8(body).unwrap();
                        f.scripts.retain(|(x, _)| *x != name);
                        f.scripts.push((name, body));
                        let _ = s.write_all(b"OK\r\n");
                    } else if up.starts_with("SETACTIVE") {
                        let name = arg(1);
                        f.active = (!name.is_empty()).then_some(name);
                        let _ = s.write_all(b"OK\r\n");
                    } else if up.starts_with("LOGOUT") {
                        let _ = s.write_all(b"OK\r\n");
                        break;
                    } else {
                        let _ = s.write_all(b"NO \"unknown\"\r\n");
                    }
                }
            }
        });
        (port, st)
    }

    fn connect(port: u16) -> Client<std::net::TcpStream> {
        let mut c = Client::from_stream(std::net::TcpStream::connect(("127.0.0.1", port)).unwrap()).unwrap();
        c.login("me@studio.example", "pw").unwrap();
        c
    }

    /// An account signed in with OAuth reaches ManageSieve with XOAUTH2, its initial response
    /// encoded once, as IMAP sends it: it went encoded twice, which no server can read, so its
    /// automatic replies could never be read or set.
    #[test]
    fn oauth_accounts_sign_in_with_xoauth2() {
        let (port, _) = serve(Fake { scripts: Vec::new(), active: None, sieve: "vacation", log: Vec::new() });
        let mut c = Client::from_stream(std::net::TcpStream::connect(("127.0.0.1", port)).unwrap()).unwrap();
        c.login_xoauth2("me@studio.example", "token").unwrap();
        assert_eq!(c.vacation().unwrap(), Vacation::default());
        c.logout();
        let mut stale = Client::from_stream(std::net::TcpStream::connect(("127.0.0.1", port)).unwrap()).unwrap();
        assert!(matches!(stale.login_xoauth2("me@studio.example", "expired"), Err(NetError::Auth(_))));
        stale.logout();
    }

    #[test]
    fn replies_on_and_off_keep_the_users_filters() {
        let (port, st) = serve(Fake {
            scripts: vec![("filters".into(), "require \"fileinto\";\r\n".into())],
            active: Some("filters".into()),
            sieve: "fileinto vacation include date relational",
            log: Vec::new(),
        });
        let mut bad = Client::from_stream(std::net::TcpStream::connect(("127.0.0.1", port)).unwrap()).unwrap();
        assert!(matches!(bad.login("me@studio.example", "nope"), Err(NetError::Auth(_))));
        bad.logout();
        let mut c = connect(port);
        assert_eq!(c.vacation().unwrap(), Vacation::default(), "never set up");
        let v = Vacation { enabled: true, subject: "Away".into(), text: "Back soon".into(), until: Some("2026-10-20".into()), ..Default::default() };
        c.set_vacation(&v).unwrap();
        assert_eq!(c.vacation().unwrap(), v);
        {
            let f = st.lock().unwrap();
            assert_eq!(f.active.as_deref(), Some(SCRIPT));
            let ours = &f.scripts.iter().find(|(n, _)| n == SCRIPT).unwrap().1;
            assert!(ours.contains("include :personal \"filters\";"), "the user's filters still run");
        }
        c.set_vacation(&Vacation { enabled: false, ..v.clone() }).unwrap();
        assert_eq!(st.lock().unwrap().active.as_deref(), Some("filters"), "their script is back");
        let off = c.vacation().unwrap();
        assert!(!off.enabled && off.text == "Back soon", "the text is kept for next time");
        // On again: theirs is still found through our script's note.
        c.set_vacation(&v).unwrap();
        assert!(st.lock().unwrap().scripts.iter().any(|(n, s)| n == SCRIPT && s.contains("include :personal \"filters\"")));
        c.logout();
    }

    #[test]
    fn servers_without_the_extensions_say_so() {
        let (port, _) = serve(Fake { scripts: Vec::new(), active: None, sieve: "fileinto", log: Vec::new() });
        let mut c = connect(port);
        let v = Vacation { enabled: true, text: "x".into(), ..Default::default() };
        assert!(matches!(c.set_vacation(&v), Err(NetError::No(e)) if e.contains("vacation")));
        let (port, _) =
            serve(Fake { scripts: vec![("filters".into(), String::new())], active: Some("filters".into()), sieve: "vacation", log: Vec::new() });
        let mut c = connect(port);
        assert!(matches!(c.set_vacation(&v), Err(NetError::No(e)) if e.contains("include")));
        let dated = Vacation { from: Some("2026-01-01".into()), ..v };
        let (port, _) = serve(Fake { scripts: Vec::new(), active: None, sieve: "vacation", log: Vec::new() });
        assert!(matches!(connect(port).set_vacation(&dated), Err(NetError::No(e)) if e.contains("dates")));
    }
}
