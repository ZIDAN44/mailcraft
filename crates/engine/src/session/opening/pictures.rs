//! The session: pictures from the web, loaded only when asked.

use super::*;

/// Most web pictures kept loaded before those no longer shown are let go.
const PICTURES_KEPT: usize = 400;

/// Whether a picture's address is one on the web (the only ones ever fetched).
fn on_web(url: &str) -> bool {
    url.starts_with("https://") || url.starts_with("http://")
}

/// The web pictures of a message that may load: tracking pictures stay unloaded even when the
/// others are shown.
fn web_pictures_of(p: &Parsed) -> impl Iterator<Item = &String> {
    p.images.iter().filter(|u| on_web(u) && !p.trackers.pictures.contains(u)).take(MAX_PICTURES)
}

impl Session {
    /// Web pictures of the open message (none for plain-text mail).
    pub fn remote_pictures(&self) -> Vec<String> {
        // Encrypted mail never reaches the web: a picture address could carry what it says out
        // (EFAIL).
        if self.reading_encrypted() {
            return Vec::new();
        }
        self.reading.as_ref().and_then(|r| r.parsed.as_ref()).map(|p| web_pictures_of(p).cloned().collect()).unwrap_or_default()
    }

    /// Whether the open message's web pictures may load: the user asked, for this message, for
    /// everything from its sender, or for every message that doesn't look suspicious.
    pub fn pictures_allowed(&self) -> bool {
        let Some(r) = &self.reading else { return false };
        // A message file opened to be inspected (issue #56): only when asked for this one. Who
        // it says it's from may be anyone, so no sender's standing and no setting loads them.
        if r.key.account == OPENED_FILE {
            return self.pictures_for.contains(&r.key);
        }
        let sender = r.parsed.as_ref().map(|p| p.from.email.trim().to_ascii_lowercase()).unwrap_or_default();
        // Locked off by the organisation (issue #245): only when asked for this one message.
        if self.pictures_locked_off() {
            return self.pictures_for.contains(&r.key);
        }
        self.pictures_for.contains(&r.key)
            || (!sender.is_empty() && (self.prefs.trusted_senders.contains(&sender) || self.sender_safe(&sender)))
            || (self.prefs.auto_pictures && self.pictures_load_alone())
    }

    /// Whether the organisation locked pictures off (issue #245): no sender's, safe or trusted,
    /// load by themselves; only a message's own, when asked for it.
    pub fn pictures_locked_off(&self) -> bool {
        !self.prefs.auto_pictures && self.setting_locked("autoPictures")
    }

    /// Whether the open message doesn't look suspicious, for the setting that loads its pictures
    /// by themselves: the mail server confirmed who sent it, it isn't in Junk, and its sender
    /// isn't blocked. A message that can't be told is suspicious here, and still asks.
    pub fn pictures_load_alone(&self) -> bool {
        let Some(r) = &self.reading else { return false };
        let Some(p) = r.parsed.as_ref() else { return false };
        let junk = self.folder(&r.key.account, &r.key.folder).is_none_or(|f| f.role == FolderRole::Junk);
        p.sender_check == mime::SenderCheck::Confirmed && !junk && !self.sender_blocked(&p.from.email.trim().to_ascii_lowercase())
    }

    /// Ask the network layer for the open message's pictures, when allowed.
    pub(crate) fn request_pictures(&mut self) {
        if !self.pictures_allowed() {
            return;
        }
        let urls = self.remote_pictures();
        self.request_urls(&urls);
    }

    /// Ask the network layer for the pictures at `urls` not loaded or loading yet. Past
    /// [`PICTURES_KEPT`], those loaded are let go first, except the ones on screen: of the open
    /// message, a message's own window and the signature of the message being written, and
    /// those just asked for. Nothing but `urls` is fetched.
    fn request_urls(&mut self, urls: &[String]) {
        if self.pictures.len() > PICTURES_KEPT {
            let mut shown: HashSet<String> = urls.iter().cloned().collect();
            for r in self.reading.iter().chain(&self.windows) {
                if let Some(p) = &r.parsed {
                    shown.extend(web_pictures_of(p).cloned());
                }
            }
            shown.extend(self.signature_web_pictures().unwrap_or_default());
            self.pictures.retain(|url, p| matches!(p, Picture::Loading) || shown.contains(url));
        }
        for url in urls {
            if !self.pictures.contains_key(url) {
                self.pictures.insert(url.clone(), Picture::Loading);
                self.jobs.push(Job::FetchUrl { url: url.clone() });
            }
        }
    }

    /// The web pictures the HTML code of the signature of the message being written names
    /// (none for a message with no such signature).
    fn signature_web_pictures(&self) -> Option<Vec<String>> {
        let c = self.compose.as_ref()?;
        c.signature_card()?;
        let (html, _) = c.signature_card_parts();
        let blocks = mime::layout::html_layout(&html);
        let mut urls: Vec<String> = Vec::new();
        for (lines, _) in mime::layout::text_blocks(&blocks) {
            for src in lines.iter().flatten().filter_map(|span| span.image.as_deref()) {
                if on_web(src) && !urls.iter().any(|u| u == src) && urls.len() < MAX_PICTURES {
                    urls.push(src.to_string());
                }
            }
        }
        Some(urls)
    }

    /// Show the open message's pictures (`always`: and everything from its sender from now on).
    pub fn show_pictures(&mut self, always: bool) -> CmdResult {
        let r = self.reading.clone().ok_or_else(|| CmdError::Disabled("open a message first".into()))?;
        self.pictures_for.insert(r.key.clone());
        let sender = r.parsed.as_ref().map(|p| p.from.email.trim().to_ascii_lowercase()).unwrap_or_default();
        // A message file's sender is never trusted on the file's word.
        let always = always && r.key.account != OPENED_FILE && !self.pictures_locked_off();
        if always && !sender.is_empty() && self.prefs.trusted_senders.len() < 10_000 {
            self.prefs.trusted_senders.insert(sender.clone());
        }
        self.request_pictures();
        Ok(serde_json::json!({ "pictures": self.remote_pictures().len(), "trusted": always.then_some(sender) }))
    }

    /// The pictures on the web that the signature of the message being written names in its
    /// HTML code: asked for, to show the signature as it's sent (issue #90). Nothing is asked
    /// for until the user says so, here as for a message that's read; the signature is the
    /// user's own, and the addresses are the ones they put in it.
    pub fn show_signature_pictures(&mut self) -> CmdResult {
        self.composing()?;
        let urls = self.signature_web_pictures().ok_or_else(|| CmdError::Disabled("the message has no signature written as HTML code".into()))?;
        self.request_urls(&urls);
        Ok(serde_json::json!({ "pictures": urls.len() }))
    }

    /// Stop loading pictures from the open message's sender automatically.
    pub fn block_pictures(&mut self) -> CmdResult {
        let r = self.reading.clone().ok_or_else(|| CmdError::Disabled("open a message first".into()))?;
        self.pictures_for.remove(&r.key);
        let sender = r.parsed.as_ref().map(|p| p.from.email.trim().to_ascii_lowercase()).unwrap_or_default();
        let was = self.prefs.trusted_senders.remove(&sender);
        Ok(serde_json::json!({ "untrusted": was }))
    }
}

impl Session {
    /// A picture a message open here shows (issue #110): one it carries (by `cid:` or the name
    /// it was sent under), or one from the web that's been loaded. `None` for one not here.
    pub fn picture_bytes(&self, key: &MessageKey, src: &str) -> Option<Vec<u8>> {
        if src.starts_with("http://") || src.starts_with("https://") {
            return match self.pictures.get(src) {
                Some(crate::Picture::Ready(b)) => Some(b.to_vec()),
                _ => None,
            };
        }
        let open = self.reading.iter().chain(self.windows.iter()).find(|r| &r.key == key)?;
        let index = mime::picture_attachment(open.parsed.as_ref()?, src)?;
        mime::attachment_data(&self.readable_raw(key)?, index)
    }

    /// The name a picture is saved under: its file's, else `picture`.
    pub fn picture_name(&self, key: &MessageKey, src: &str) -> String {
        let named = if src.starts_with("http://") || src.starts_with("https://") {
            src.split(['?', '#']).next().and_then(|u| u.rsplit('/').next()).map(str::to_string)
        } else {
            let open = self.reading.iter().chain(self.windows.iter()).find(|r| &r.key == key);
            open.and_then(|r| r.parsed.as_ref()).and_then(|p| {
                let i = mime::picture_attachment(p, src)?;
                p.attachments.iter().find(|a| a.index == i).map(|a| a.name.clone())
            })
        };
        let name = mime::sanitize_filename(named.as_deref().unwrap_or("").trim());
        if name.is_empty() { "picture".into() } else { name }
    }
}
