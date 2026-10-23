//! What MailCraft is started with besides its options: `mailto:` links and mail files.
//!
//! Chosen as the system's mail app, MailCraft is started with the link clicked
//! (`mailcraft.exe "mailto:…"` on Windows, `mailcraft mailto:…` from the desktop entry's `%U` on
//! Linux and the BSDs); opened with it, with the file's path, or on Linux a `file://` link to it.
//! The first MailCraft opens them once its window is up; a later launch hands them to the one
//! running (`single_instance.rs`), with each path made absolute, since that one doesn't share
//! this launch's working folder. On macOS, files come as Apple events instead (`apple_events.rs`).

use std::path::Path;

/// What `--help` prints.
pub const USAGE: &str = "MailCraft: mail, calendar, contacts and notes.

Usage: mailcraft [options] [mailto:LINK | FILE]…

  --sample          open the sample mailbox (its own data, nothing of yours)
  --data DIR        keep everything in DIR instead of the usual folders
  --light, --dark   start in the light or dark theme
  --cmd ID[=JSON]   run a command once the window opens (repeatable), e.g.
                    --cmd 'list.open={\"key\":\"sample:1:1:INBOX\"}' --cmd mail.reply
  --log             also write the log to the terminal
  --version, -V     print the version
  --help, -h        print this

A mailto: link starts a new message; a .eml file is shown, and a .ics or .vcf
file's events or people are added. If MailCraft is running, they open there.

A portable copy keeps its data beside the app when portable.txt is there.
The command-line client is mailcraft-cli (mailcraft-cli --help).";

/// `--version`: the version, plus the commit and date of release builds (packaging/common.sh).
pub fn version_line() -> String {
    let v = format!("mailcraft {}", env!("CARGO_PKG_VERSION"));
    match (option_env!("MAILCRAFT_BUILD_SHA"), option_env!("MAILCRAFT_BUILD_DATE")) {
        (Some(sha), Some(date)) if !sha.is_empty() => format!("{v} ({}, {date})", sha.get(..9).unwrap_or(sha)),
        _ => v,
    }
}

/// Options followed by a value of their own.
const WITH_VALUE: [&str; 2] = ["--data", "--cmd"];
/// Most links and files taken from one launch.
const MAX_HANDED: usize = 64;

/// The links and files among `args` (everything that isn't an option or an option's value), in
/// order: links as they are, paths absolute. Junk is kept for the window to say it can't open.
pub fn handed(args: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut skip = false;
    for a in args {
        if std::mem::take(&mut skip) {
            continue;
        }
        if a.starts_with('-') {
            // `--data DIR`, `--cmd ID`; also `-psn_…` (old macOS) and any other option.
            skip = WITH_VALUE.contains(&a.as_str());
            continue;
        }
        if out.len() >= MAX_HANDED {
            break;
        }
        if let Some(h) = one(a) {
            out.push(h);
        }
    }
    out
}

/// One argument as handed on: a `mailto:` link as it is; a `file:` link or a path as an absolute
/// path; nothing for an empty one or a link MailCraft doesn't open (another scheme, a `file:` link
/// to another computer).
fn one(arg: &str) -> Option<String> {
    let arg = arg.trim();
    if arg.is_empty() || arg == "/?" {
        return None;
    }
    if mailcraft_engine::model::mailto::is_mailto(arg) {
        let mut end = arg.len().min(mailcraft_engine::model::mailto::MAX_LINK);
        while !arg.is_char_boundary(end) {
            end = end.saturating_sub(1);
        }
        return arg.get(..end).map(str::to_string);
    }
    let path = match scheme(arg) {
        Some(s) if s.eq_ignore_ascii_case("file") => from_file_link(arg)?,
        Some(_) => return None,
        None => arg.to_string(),
    };
    Some(std::path::absolute(Path::new(&path)).map_or(path, |p| p.to_string_lossy().into_owned()))
}

/// A link's scheme (`https` in `https://…`): letters, then digits, `+`, `-` or `.`, before a `:`.
/// A Windows drive (`C:\…`) has a one-letter one and is a path.
fn scheme(arg: &str) -> Option<&str> {
    let name = arg.get(..arg.find(':')?)?;
    let starts_with_letter = name.bytes().next().is_some_and(|b| b.is_ascii_alphabetic());
    let allowed = |b: u8| b.is_ascii_alphanumeric() || b"+-.".contains(&b);
    let is_drive_letter = name.len() == 1;
    (starts_with_letter && !is_drive_letter && name.bytes().all(allowed)).then_some(name)
}

/// The path of a `file:` link on this computer (`file:///home/zoe/a%20b.eml`,
/// `file://localhost/…`, `file:///C:/…`), or `None` for one to another computer.
fn from_file_link(link: &str) -> Option<String> {
    let rest = link.get(5..)?;
    let path = match rest.strip_prefix("//") {
        Some(r) => {
            let (host, path) = r.find('/').map_or((r, ""), |i| (r.get(..i).unwrap_or(""), r.get(i..).unwrap_or("")));
            if !host.is_empty() && !host.eq_ignore_ascii_case("localhost") {
                return None;
            }
            path
        }
        None => rest,
    };
    let path = mailcraft_engine::model::mailto::percent_decode(path.split(['?', '#']).next().unwrap_or(""));
    // `/C:/Users/…` is `C:/Users/…` on Windows.
    let windows_drive = path.get(1..3).is_some_and(|d| d.ends_with(':') && d.starts_with(|c: char| c.is_ascii_alphabetic()));
    let path = if cfg!(windows) && windows_drive { path.get(1..).unwrap_or_default().to_string() } else { path };
    (!path.is_empty()).then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(args: &[&str]) -> Vec<String> {
        handed(&args.iter().map(|a| a.to_string()).collect::<Vec<_>>())
    }

    /// Options and their values aren't handed over; links and files are, in order.
    #[test]
    fn options_are_left_and_links_and_files_taken() {
        assert!(h(&["--sample", "--light", "--log"]).is_empty());
        assert!(h(&["--data", "D:/Mail", "--cmd", "mail.new", "-psn_0_12345"]).is_empty(), "values of options aren't files");
        let got = h(&["--sample", "mailto:ana@x.example?subject=Hi", "--cmd", "ui.about", "MAILTO:bo@x.example"]);
        assert_eq!(got, ["mailto:ana@x.example?subject=Hi", "MAILTO:bo@x.example"]);
        assert!(h(&["", "  ", "/?", "https://x.example/a.eml", "javascript:alert(1)", "ms-outlook:x"]).is_empty(), "nothing to open");
        assert_eq!(h(&["--data"]), Vec::<String>::new(), "a missing value is fine");
        let many: Vec<String> = (0..200).map(|i| format!("mailto:{i}@x.example")).collect();
        assert_eq!(handed(&many).len(), MAX_HANDED);
    }

    /// Paths are made absolute against this launch's folder; `file:` links are read.
    #[test]
    fn paths_are_absolute() {
        let here = std::env::current_dir().unwrap();
        let got = h(&["invite.ics"]);
        assert_eq!(got, [here.join("invite.ics").to_string_lossy().to_string()]);
        if cfg!(windows) {
            assert_eq!(h(&[r"C:\Users\Zoe\Mail\a.eml"]), [r"C:\Users\Zoe\Mail\a.eml"]);
            assert_eq!(h(&["file:///C:/Users/Zoe/My%20Mail/a.eml"]), [r"C:\Users\Zoe\My Mail\a.eml"]);
        } else {
            assert_eq!(h(&["/home/zoe/a.eml"]), ["/home/zoe/a.eml"]);
            assert_eq!(h(&["file:///home/zoe/My%20Mail/caf%C3%A9.eml"]), ["/home/zoe/My Mail/café.eml"]);
            assert_eq!(h(&["file://localhost/home/zoe/b.vcf"]), ["/home/zoe/b.vcf"]);
            assert_eq!(h(&["file:/home/zoe/c.ics"]), ["/home/zoe/c.ics"]);
        }
        assert!(h(&["file://server.example/share/a.eml", "file://", "file:///"]).iter().all(|p| !p.contains("server")), "not another computer");
    }

    #[test]
    fn schemes_and_drives() {
        assert_eq!(scheme("mailto:x"), Some("mailto"));
        assert_eq!(scheme("svn+ssh://x"), Some("svn+ssh"));
        assert_eq!(scheme(r"C:\x"), None);
        assert_eq!(scheme("a b:c"), None);
        assert_eq!(scheme("1x:y"), None);
        assert_eq!(scheme("plain.eml"), None);
    }

    /// Whatever the system hands over is read without trouble.
    #[test]
    fn junk_is_handled() {
        let huge = format!("mailto:a@b?body={}", "é".repeat(mailcraft_engine::model::mailto::MAX_LINK));
        assert!(h(&[&huge]).first().is_some_and(|l| l.len() <= mailcraft_engine::model::mailto::MAX_LINK));
        for odd in ["file:", "file:%", "file:///%ZZ", "\u{0}", "::::", "mailto:", "é:é", "C:"] {
            let _ = h(&[odd]);
        }
    }
}
