//! What an import kept of what a `.pst` says of its mail, counted from the sources and the list it
//! left (`pst_bench::import` with `MC_VERIFY`, its `kept:` line): numbers only, to set beside
//! what the file holds (the fidelity design's table) without reading the file again.

use mailcraft_model::Summary;
use serde::Serialize;

/// Attached messages are counted this deep (the import brings in up to 32 levels; no real file
/// measured nests past three).
const LEVELS: usize = 4;

/// The counts, over every message read back.
#[derive(Debug, Default, PartialEq, Eq, Serialize)]
pub(super) struct Kept {
    /// Messages with no text at all, plain or HTML, and those whose text holds U+FFFD (a
    /// character that couldn't be read, by the import or by the sender).
    pub no_text: u64,
    pub replacement: u64,
    /// Files listed with the message (not pictures its text shows), those of no bytes, and those
    /// keeping a Content-ID.
    pub files: u64,
    pub empty_files: u64,
    pub files_with_cid: u64,
    /// Pictures the text shows, and those without a name of their own (one MailCraft made up).
    pub inline: u64,
    pub inline_unnamed: u64,
    /// Attached messages by level (0: attached to the message itself), and the files inside them.
    pub attached: [u64; LEVELS],
    pub inside: u64,
    /// Reports (`multipart/report`: bounces and receipts) kept as reports.
    pub reports: u64,
    /// Messages with a Message-ID the import made up, as for sent mail that had none.
    pub made_up_ids: u64,
    /// From the list: forwarded, high importance, received more than an hour from their date,
    /// and with a Thread-Index of their own.
    pub forwarded: u64,
    pub high: u64,
    pub received_apart: u64,
    pub threaded: u64,
}

impl Kept {
    /// Count the message `m`, whose source is `raw`.
    pub(super) fn add(&mut self, m: &Summary, raw: &[u8]) {
        self.forwarded += u64::from(m.flags.forwarded);
        self.high += u64::from(m.importance > 0);
        self.received_apart += u64::from(m.received > 0 && m.received.abs_diff(m.date) > 3600);
        self.threaded += u64::from(m.thread_own != 0);
        self.made_up_ids += u64::from(m.message_id.ends_with("@mailcraft.invalid"));
        self.reports += u64::from(is_report(raw));
        let Some(p) = mailcraft_mime::parse(raw) else { return };
        self.no_text += u64::from(p.text.trim().is_empty());
        self.replacement += u64::from(p.text.contains('\u{FFFD}'));
        for a in &p.attachments {
            if a.mime.eq_ignore_ascii_case("message/rfc822") {
                self.attached[0] += 1;
                if let Some(inner) = mailcraft_mime::attachment_data(raw, a.index) {
                    self.inside_of(&inner, 1);
                }
            } else if a.inline {
                self.inline += 1;
                self.inline_unnamed += u64::from(a.name.starts_with(&format!("attachment-{}", a.index + 1)));
            } else {
                self.files += 1;
                self.empty_files += u64::from(a.size == 0);
                self.files_with_cid += u64::from(a.content_id.is_some());
            }
        }
    }

    /// The attached message `raw`, `level` down: the messages attached to it, and its other files.
    fn inside_of(&mut self, raw: &[u8], level: usize) {
        let Some(p) = mailcraft_mime::parse(raw) else { return };
        for a in &p.attachments {
            if !a.mime.eq_ignore_ascii_case("message/rfc822") {
                self.inside += 1;
                continue;
            }
            let Some(n) = self.attached.get_mut(level) else { continue };
            *n += 1;
            if let Some(inner) = mailcraft_mime::attachment_data(raw, a.index) {
                self.inside_of(&inner, level + 1);
            }
        }
    }
}

/// Whether the source's own header says it is a report (the type may be folded onto a second line).
fn is_report(raw: &[u8]) -> bool {
    let head = raw.windows(4).position(|w| w == b"\r\n\r\n").map_or(raw, |end| raw.get(..end).unwrap_or(raw));
    let head = String::from_utf8_lossy(head).to_ascii_lowercase().replace("\r\n ", " ").replace("\r\n\t", " ");
    head.lines().any(|l| l.strip_prefix("content-type:").is_some_and(|v| v.trim_start().starts_with("multipart/report")))
}

/// Every count from sources made by hand: files, pictures with and without names, attached
/// messages two deep with files inside, a report, a message with no text and one whose id was
/// made up, and the list's flags.
#[test]
fn what_an_import_kept_is_counted() {
    let picture = "iVBORw0KGgo=";
    let mail = format!(
        "From: a@b.example\r\nMessage-ID: <1@b.example>\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=\"m\"\r\n\r\n\
         --m\r\nContent-Type: multipart/related; boundary=\"r\"\r\n\r\n\
         --r\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<p>Hi \u{FFFD}</p><img src=\"cid:p1\"><img src=\"cid:p2\">\r\n\
         --r\r\nContent-Type: image/png; name=\"logo.png\"\r\nContent-Disposition: inline; filename=\"logo.png\"\r\nContent-ID: <p1>\r\nContent-Transfer-Encoding: base64\r\n\r\n{picture}\r\n\
         --r\r\nContent-Type: image/png\r\nContent-Disposition: inline\r\nContent-ID: <p2>\r\nContent-Transfer-Encoding: base64\r\n\r\n{picture}\r\n\
         --r--\r\n\
         --m\r\nContent-Type: application/pdf; name=\"a.pdf\"\r\nContent-Disposition: attachment; filename=\"a.pdf\"\r\nContent-ID: <f1>\r\nContent-Transfer-Encoding: base64\r\n\r\nJVBERi0=\r\n\
         --m\r\nContent-Type: application/octet-stream; name=\"empty.bin\"\r\nContent-Disposition: attachment; filename=\"empty.bin\"\r\n\r\n\r\n\
         --m\r\nContent-Type: message/rfc822\r\nContent-Disposition: attachment; filename=\"fwd.eml\"\r\n\r\n\
         From: c@d.example\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=\"i\"\r\n\r\n\
         --i\r\nContent-Type: text/plain\r\n\r\nInside.\r\n\
         --i\r\nContent-Type: text/csv; name=\"t.csv\"\r\nContent-Disposition: attachment; filename=\"t.csv\"\r\n\r\na,b\r\n\
         --i\r\nContent-Type: message/rfc822\r\nContent-Disposition: attachment; filename=\"deeper.eml\"\r\n\r\n\
         From: e@f.example\r\nContent-Type: text/plain\r\n\r\nDeepest.\r\n\
         --i--\r\n\
         --m--\r\n"
    );
    let report = "From: postmaster@b.example\r\nMIME-Version: 1.0\r\nContent-Type: multipart/report;\r\n report-type=delivery-status; boundary=\"x\"\r\n\r\n\
         --x\r\nContent-Type: text/plain\r\n\r\nNot delivered.\r\n\
         --x\r\nContent-Type: message/delivery-status\r\n\r\nReporting-MTA: dns; b.example\r\n\r\nFinal-Recipient: rfc822; z@y.example\r\nAction: failed\r\nStatus: 5.0.0\r\n\
         --x--\r\n";
    let empty = "From: a@b.example\r\nContent-Type: text/plain\r\n\r\n\r\n";
    let summary = |id: &str| Summary { message_id: id.to_string(), ..Default::default() };
    let mut kept = Kept::default();
    let mut first = summary("1@b.example");
    first.flags.forwarded = true;
    (first.importance, first.date, first.received, first.thread_own) = (1, 1_700_000_000, 1_700_007_200, 7);
    kept.add(&first, mail.as_bytes());
    kept.add(&summary("2@b.example"), report.as_bytes());
    let mut sent = summary("pst-a1-3@mailcraft.invalid");
    (sent.date, sent.received) = (1_700_000_000, 1_700_000_600);
    kept.add(&sent, empty.as_bytes());
    let want = Kept {
        no_text: 1,
        replacement: 1,
        // The report's status is one.
        files: 3,
        empty_files: 1,
        files_with_cid: 1,
        inline: 2,
        inline_unnamed: 1,
        attached: [1, 1, 0, 0],
        inside: 1,
        reports: 1,
        made_up_ids: 1,
        forwarded: 1,
        high: 1,
        received_apart: 1,
        threaded: 1,
    };
    assert_eq!(kept, want);
}
