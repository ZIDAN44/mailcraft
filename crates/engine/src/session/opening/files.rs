//! Message files opened to be looked at (issue #56): each in a message window of its own, as a
//! message double-clicked in the list is, but in no account and no folder. Nothing in it loads
//! from the web, nothing is added anywhere, and closing it leaves no trace.

use super::*;
use mime::layout::Block;
use serde_json::json;

/// A message file open in a window: its name, and its bytes, kept in memory while the window is.
pub(crate) struct OpenedFile {
    pub(crate) key: MessageKey,
    pub(crate) name: String,
    pub(crate) raw: Arc<[u8]>,
}

/// The longest a link's address is shown beside its words.
const ADDRESS_SHOWN: usize = 200;

impl Session {
    /// Open the message `raw`, read from the file `name`, in a window of its own: `file.open`'s
    /// answer, with the window's `key`.
    pub(crate) fn open_message_file(&mut self, name: &str, raw: &[u8]) -> CmdResult {
        let mut answer = crate::cmd::message_preview(self, name, raw)?;
        let uid = u32::try_from(self.fresh_id() & u64::from(u32::MAX)).unwrap_or_default();
        let key = MessageKey { account: OPENED_FILE.into(), folder: OPENED_FILE.into(), validity: 0, uid };
        // Signed or encrypted: what's inside is shown, its protection said beside it.
        let (shown, security) = self.unwrap(&key, raw);
        let parsed = mime::parse(&shown).map(|p| self.for_reading(&key, p));
        let warning = parsed.as_ref().and_then(|p| self.message_warning(&key, &p.from, p.bulk));
        let error = parsed.is_none().then(|| "this message could not be read".to_string());
        if self.windows.len() >= MAX_WINDOWS {
            self.windows.remove(0);
        }
        self.windows.push(Reading { key: key.clone(), parsed, error, thread: Vec::new(), warning, security, text_first: false });
        self.opened_files.push(OpenedFile { key: key.clone(), name: name.to_string(), raw: Arc::from(raw) });
        self.forget_closed_files();
        if let Some(a) = answer.as_object_mut() {
            a.insert("key".into(), json!(key.to_id()));
        }
        Ok(answer)
    }

    /// The name of the file message `key` was opened from, when it's an opened file.
    pub fn opened_file_name(&self, key: &MessageKey) -> Option<&str> {
        self.opened_files.iter().find(|f| &f.key == key).map(|f| f.name.as_str())
    }

    /// The bytes of the file message `key` was opened from, when it's an opened file.
    pub fn opened_file_source(&self, key: &MessageKey) -> Option<Arc<[u8]>> {
        self.opened_files.iter().find(|f| &f.key == key).map(|f| f.raw.clone())
    }

    /// The files whose windows closed are let go of, with what was decrypted of them.
    pub(super) fn forget_closed_files(&mut self) {
        let windows = &self.windows;
        let (kept, gone): (Vec<OpenedFile>, Vec<OpenedFile>) =
            std::mem::take(&mut self.opened_files).into_iter().partition(|f| windows.iter().any(|w| w.key == f.key));
        self.opened_files = kept;
        for f in gone {
            self.opened.remove(&f.key);
        }
    }

    /// A message's bytes as kept: an opened file's, an Outbox message's, or the cache's.
    pub(crate) fn body_raw(&self, key: &MessageKey) -> Option<Vec<u8>> {
        if key.account == OPENED_FILE {
            self.opened_files.iter().find(|f| &f.key == key).map(|f| f.raw.to_vec())
        } else if key.folder == OUTBOX {
            self.outbox_entry(key).and_then(|(a, i)| self.store.as_ref().and_then(|s| s.outbox_raw(&a, i.id)))
        } else {
            self.store.as_ref().and_then(|s| s.read_body(key))
        }
    }
}

/// Beside each link whose words say something else, where it really goes (` <address>`), in the
/// formatted text and in its laid-out form: a file is opened to be inspected, and a link that
/// hides where it goes is what such a look is for.
pub(super) fn show_link_targets(p: &mut mime::Parsed) {
    p.rich.iter_mut().for_each(line_targets);
    blocks_targets(&mut p.layout, 0);
}

fn blocks_targets(blocks: &mut [Block], depth: usize) {
    // Deeper than any mail nests (the layout caps it already).
    if depth > 64 {
        return;
    }
    for b in blocks {
        match b {
            Block::Text(lines) => lines.iter_mut().for_each(line_targets),
            Block::Table(t) => {
                for cell in t.rows.iter_mut().flatten() {
                    blocks_targets(&mut cell.blocks, depth + 1);
                    for o in cell.overlays.iter_mut() {
                        blocks_targets(&mut o.blocks, depth + 1);
                    }
                }
            }
        }
    }
}

/// One line's links: a link in several spans (a word of it bold) says where it goes once, after
/// its last.
fn line_targets(line: &mut Vec<mime::Span>) {
    if line.iter().all(|s| s.link.is_none()) {
        return;
    }
    let mut out = Vec::with_capacity(line.len() + 1);
    let mut words = String::new();
    let mut spans = std::mem::take(line).into_iter().peekable();
    while let Some(span) = spans.next() {
        let Some(link) = span.link.clone() else {
            out.push(span);
            continue;
        };
        if span.image.is_none() {
            words.push_str(&span.text);
        }
        out.push(span);
        if spans.peek().is_some_and(|next| next.link.as_ref() == Some(&link)) {
            continue;
        }
        if !same_place(&link, &words) {
            let mut shown = link.trim().to_string();
            if shown.len() > ADDRESS_SHOWN {
                let cut = (0..=ADDRESS_SHOWN).rev().find(|i| shown.is_char_boundary(*i)).unwrap_or(0);
                shown.truncate(cut);
                shown.push('…');
            }
            out.push(mime::Span { text: format!(" <{shown}>"), ..Default::default() });
        }
        words.clear();
    }
    *line = out;
}

/// Whether a link's words name where it goes: the same address, give or take its scheme
/// (`https://`, `mailto:`) and a closing slash.
fn same_place(link: &str, words: &str) -> bool {
    fn bare(s: &str) -> String {
        let s = s.trim().to_lowercase();
        let s = ["https://", "http://", "mailto:"].iter().find_map(|p| s.strip_prefix(p)).map_or(s.as_str(), |r| r).to_string();
        s.trim_end_matches('/').to_string()
    }
    let words = bare(words);
    !words.is_empty() && words == bare(link)
}
