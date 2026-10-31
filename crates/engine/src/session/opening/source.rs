//! The session: a message's source as it came, headers and all, each in a window of its own
//! (View message source, issue #27).
//!
//! The source is shown as text a line at a time: CR LF ends a line, a byte that isn't UTF-8
//! shows as U+FFFD, and what could hide or reorder text (control characters, the marks and
//! overrides that turn text right to left) is shown by name. Only the first [`SOURCE_SHOWN`]
//! bytes are laid out; Save as (.eml) has the rest.

use super::*;

/// How much of a source is shown (a message of 30 MB is mostly its attachment, in base64).
pub const SOURCE_SHOWN: usize = 2 << 20;

/// The most lines shown (a source of nothing but line ends).
const LINES_MAX: usize = 100_000;

/// A line longer than this is shown broken into lines this long (a base64 attachment on one
/// line of megabytes).
const LINE_CHARS: usize = 2_000;

/// The most sources open at once; the oldest closes first.
const SOURCES_MAX: usize = MAX_WINDOWS;

/// A message's source, shown in a window of its own.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SourceView {
    pub key: MessageKey,
    pub subject: String,
    /// The source as lines to show (empty until it's here).
    pub lines: Vec<String>,
    /// How many of `lines` are the header: those before the blank line that ends it.
    pub header_lines: usize,
    /// The whole source's size, in bytes.
    pub size: usize,
    /// Only the start of it is in `lines`.
    pub truncated: bool,
    /// Still downloading.
    pub pending: bool,
    /// Why it couldn't be had.
    pub error: Option<String>,
}

impl SourceView {
    /// The lines shown, as one text (for copying): the header only, or all.
    pub fn text(&self, header_only: bool) -> String {
        let n = if header_only { self.header_lines } else { self.lines.len() };
        let mut out = String::with_capacity(self.lines.iter().take(n).map(|l| l.len() + 1).sum());
        for l in self.lines.iter().take(n) {
            out.push_str(l);
            out.push('\n');
        }
        out
    }

    fn fill(&mut self, raw: &[u8]) {
        let (lines, header_lines, truncated) = source_lines(raw);
        self.lines = lines;
        self.header_lines = header_lines;
        self.truncated = truncated;
        self.size = raw.len();
        self.pending = false;
        self.error = None;
    }
}

/// `raw` as lines to show, how many of them are the header, and whether some was left out.
pub(crate) fn source_lines(raw: &[u8]) -> (Vec<String>, usize, bool) {
    // Cut where a character starts, not inside one.
    let mut cut = SOURCE_SHOWN.min(raw.len());
    for _ in 0..3 {
        match raw.get(cut) {
            Some(b) if (0x80..0xc0).contains(b) && cut > 0 => cut -= 1,
            _ => break,
        }
    }
    let shown = raw.get(..cut).unwrap_or(raw);
    let mut truncated = shown.len() < raw.len();
    let mut lines = Vec::new();
    let mut header = None;
    let mut charset = String::from("windows-1252");
    // CR LF, a lone LF or a lone CR each end a line.
    let mut rest = shown;
    while !rest.is_empty() || lines.is_empty() {
        let end = rest.iter().position(|&b| b == b'\n' || b == b'\r').unwrap_or(rest.len());
        let (line, after) = rest.split_at(end);
        let skip = match after {
            [b'\r', b'\n', ..] => 2,
            [_, ..] => 1,
            [] => 0,
        };
        rest = after.get(skip..).unwrap_or_default();
        if line.is_empty() && header.is_none() {
            header = Some(lines.len());
        }
        // A part's character set, as its header says, reads its text that isn't UTF-8 (8-bit
        // mail); none said: Windows' Western one, which most such mail is in.
        if let Some(c) = charset_of(line) {
            charset = c;
        }
        let text = match std::str::from_utf8(line) {
            Ok(t) => shown_text(t),
            Err(_) => shown_text(&mime::decode_in(&charset, line).unwrap_or_else(|| String::from_utf8_lossy(line).into_owned())),
        };
        let mut chunk = String::new();
        let mut n = 0;
        for c in text.chars() {
            if n == LINE_CHARS {
                lines.push(std::mem::take(&mut chunk));
                n = 0;
            }
            chunk.push(c);
            n += 1;
        }
        lines.push(chunk);
        if lines.len() >= LINES_MAX {
            lines.truncate(LINES_MAX);
            truncated |= !rest.is_empty();
            break;
        }
        if skip == 0 {
            break;
        }
    }
    let header_lines = header.unwrap_or(lines.len()).min(lines.len());
    (lines, header_lines, truncated)
}

/// The character set a line names (`Content-Type: text/plain; charset="iso-8859-1"`).
fn charset_of(line: &[u8]) -> Option<String> {
    let at = line.windows(8).position(|w| w.eq_ignore_ascii_case(b"charset="))?;
    let rest = line.get(at + 8..)?;
    let name: String = rest
        .iter()
        .skip_while(|&&b| b == b'"' || b == b'\'')
        .take_while(|&&b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b':' | b'.'))
        .map(|&b| char::from(b))
        .collect();
    (!name.is_empty() && name.len() <= 40).then_some(name)
}

/// A line with what would hide or reorder text shown by name: control characters as their
/// pictures (␀, ␛), the marks and overrides of text direction as `<U+202E>`. Tabs stay.
fn shown_text(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    for c in line.chars() {
        match c {
            '\t' => out.push(c),
            '\u{0}'..='\u{1f}' => out.push(char::from_u32(0x2400 + u32::from(c)).unwrap_or('\u{fffd}')),
            '\u{7f}' => out.push('\u{2421}'),
            '\u{80}'..='\u{9f}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{feff}' => {
                out.push_str(&format!("<U+{:04X}>", u32::from(c)));
            }
            c => out.push(c),
        }
    }
    out
}

impl Session {
    /// The source of a message, if it's here: cached on disk, or waiting in the Outbox.
    pub(super) fn cached_source(&self, key: &MessageKey) -> Option<Vec<u8>> {
        if key.folder == OUTBOX {
            self.outbox_entry(key).and_then(|(a, i)| self.store.as_ref().and_then(|s| s.outbox_raw(&a, i.id)))
        } else {
            self.store.as_ref().and_then(|s| s.read_body(key))
        }
    }

    /// Whether a message is somewhere it can be named from: the list, the reading pane, a window.
    pub fn knows_message(&self, key: &MessageKey) -> bool {
        self.summary(key).is_some() || self.reading.as_ref().is_some_and(|r| &r.key == key) || self.windows.iter().any(|w| &w.key == key)
    }

    /// Show a message's source in a window of its own; one already open stays as it is. Not
    /// downloaded yet: it's asked for, and comes when it's here.
    pub fn open_source(&mut self, key: &MessageKey) -> Option<&SourceView> {
        if let Some(i) = self.sources.iter().position(|v| &v.key == key) {
            // Shown already: but one that failed is tried again.
            if self.sources.get(i).is_some_and(|v| v.error.is_some()) {
                self.sources.remove(i);
            } else {
                return self.sources.get(i);
            }
        }
        if self.sources.len() >= SOURCES_MAX {
            self.sources.remove(0);
        }
        let subject = self
            .summary(key)
            .map(|m| m.subject.clone())
            .or_else(|| {
                self.windows.iter().chain(self.reading.as_ref()).find(|r| &r.key == key).and_then(|r| r.parsed.as_ref()).map(|p| p.subject.clone())
            })
            .unwrap_or_default();
        let mut view = SourceView { key: key.clone(), subject, ..SourceView::default() };
        // Exchange's text first (issue #24) is MailCraft's, not the message's: the source waits.
        match self.cached_source(key).filter(|raw| !mime::is_text_first(raw)) {
            Some(raw) => view.fill(&raw),
            None => {
                view.pending = true;
                // Asked for already by the reading pane or a window: not twice.
                let asked =
                    self.reading.iter().chain(&self.windows).any(|r| &r.key == key && r.error.is_none() && (r.parsed.is_none() || r.text_first));
                if !asked {
                    self.jobs.push(Job::FetchBody { key: key.clone() });
                }
            }
        }
        self.sources.push(view);
        self.sources.last()
    }

    pub fn close_source(&mut self, key: &MessageKey) {
        self.sources.retain(|v| &v.key != key);
    }

    /// A message's source came: shown where it's waited for. Its text first isn't it.
    pub(in crate::session) fn source_arrived(&mut self, key: &MessageKey, raw: &[u8]) {
        if mime::is_text_first(raw) {
            return;
        }
        for v in self.sources.iter_mut().filter(|v| &v.key == key) {
            v.fill(raw);
        }
    }

    /// It couldn't be had: said where it's waited for.
    pub(in crate::session) fn source_failed(&mut self, key: &MessageKey, error: &str) {
        for v in self.sources.iter_mut().filter(|v| &v.key == key && v.pending) {
            v.pending = false;
            v.error = Some(error.to_string());
        }
    }
}
