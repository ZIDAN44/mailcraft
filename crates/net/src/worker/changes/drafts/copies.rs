//! MailCraft's own copies of a draft on an IMAP server. Each copy it files carries a random mark
//! of its own ([`COPY_HEADER`]), remembered with the draft's Message-ID before the copy goes, so
//! a later filing of the draft (kept on this computer as MailCraft closed while its save got there
//! too) replaces exactly that copy, even kept at another size than sent (a server that adds
//! headers). Another program's copy under the same Message-ID, built anew without the mark, stays.

use super::*;

use mailcraft_mime::COPY_HEADER;

/// The drafts MailCraft filed on an account's IMAP server, by Message-ID and the mark on the copy
/// ([`Worker::copies_of`]).
const FILED_DRAFTS: &str = "drafts-filed.json";
/// Drafts filed remembered, at most (the oldest forgotten).
const MAX_FILED: usize = 256;
/// How much of a copy's header is read for its mark, at most.
const MAX_HEADER: usize = 64 * 1024;

#[derive(serde::Serialize, serde::Deserialize, PartialEq)]
struct FiledDraft {
    message_id: String,
    /// The mark on its copy; none matches no copy.
    #[serde(default)]
    mark: String,
}

/// A new mark for a copy of a draft: 128 bits of the system's randomness, as hex; none without it.
pub(super) fn new_mark() -> Option<String> {
    let mut bits = [0u8; 16];
    ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut bits).ok()?;
    Some(bits.iter().map(|b| format!("{b:02x}")).collect())
}

/// `raw` with `mark` on it, first in its header.
pub(super) fn marked(raw: &[u8], mark: &str) -> Vec<u8> {
    let line = format!("{COPY_HEADER}: {mark}\r\n");
    let mut out = Vec::with_capacity(raw.len().saturating_add(line.len()));
    out.extend_from_slice(line.as_bytes());
    out.extend_from_slice(raw);
    out
}

/// Whether a copy's header (`head`) carries one of `marks`.
fn has_mark(head: &[u8], marks: &[String]) -> bool {
    let end = head.windows(4).position(|w| w == b"\r\n\r\n").unwrap_or(head.len()).min(MAX_HEADER);
    let head = String::from_utf8_lossy(head.get(..end).unwrap_or_default()).replace("\r\n ", " ").replace("\r\n\t", " ");
    head.split("\r\n")
        .filter_map(|line| line.split_once(':'))
        .any(|(name, value)| name.trim().eq_ignore_ascii_case(COPY_HEADER) && marks.iter().any(|m| !m.is_empty() && m == value.trim()))
}

impl Worker {
    /// A draft filed on the server, its copy marked `mark`, remembered as MailCraft's own
    /// ([`Self::copies_of`]); not for an account removed since the worker started.
    pub(super) fn filed_draft(&self, message_id: &str, mark: &str) {
        if message_id.is_empty() || mark.is_empty() {
            return;
        }
        let kept = self.store.while_kept(&self.acc.id, self.removals, || {
            let _one_at_a_time = pending_file();
            let mut list: Vec<FiledDraft> = self.store.load_account_doc(&self.acc.id, FILED_DRAFTS).unwrap_or_default();
            if list.len() >= MAX_FILED {
                list.remove(0);
            }
            list.push(FiledDraft { message_id: message_id.to_string(), mark: mark.to_string() });
            self.store.save_account_doc(&self.acc.id, FILED_DRAFTS, &list)
        });
        if let Some(Err(e)) = kept {
            log::info!("{}: a draft filed wasn't remembered: {e}", self.acc.id);
        }
    }

    /// MailCraft's own copies of a draft (its Message-ID) in `folder` on an IMAP server, as it
    /// filed them, but the copy the filing replaces (`but`), which goes as any replaced copy does:
    /// found by the header, then each one's Message-ID and mark read to be sure (a server's search
    /// may match the words of it, and another program may have saved the draft again with the
    /// same Message-ID, which stays). None where they can't be asked for (the folder gone, another
    /// kind of server).
    pub(super) fn copies_of(&mut self, folder: &str, message_id: &str, but: Option<&MessageKey>) -> Vec<MessageKey> {
        if message_id.is_empty() || !matches!(self.conn, Some(Conn::Imap(_))) {
            return Vec::new();
        }
        let marks: Vec<String> = self
            .store
            .load_account_doc::<Vec<FiledDraft>>(&self.acc.id, FILED_DRAFTS)
            .unwrap_or_default()
            .into_iter()
            .filter(|d| d.message_id == message_id && !d.mark.is_empty())
            .map(|d| d.mark)
            .collect();
        if marks.is_empty() {
            return Vec::new();
        }
        let account = self.id();
        let found = (|| -> Result<Vec<MessageKey>> {
            let c = self.client()?;
            let info = c.select(folder, false)?;
            let id = format!("<{message_id}>");
            let args =
                [crate::sync::SearchArg::Atom("HEADER".into()), crate::sync::SearchArg::Atom("Message-ID".into()), crate::sync::SearchArg::Str(id)];
            let uids: Vec<u32> = c.uid_search_args(&args)?.into_iter().take(16).collect();
            let fetched = c.uid_fetch(&crate::imap::uid_set(&uids), "BODY.PEEK[HEADER]", "")?;
            Ok(fetched
                .into_iter()
                .filter(|f| uids.contains(&f.uid) && f.header.as_deref().is_some_and(|h| written_message_id(h) == message_id && has_mark(h, &marks)))
                .map(|f| MessageKey { account: account.clone(), folder: folder.to_string(), validity: info.validity, uid: f.uid })
                .filter(|k| Some(k) != but)
                .collect())
        })();
        found.unwrap_or_else(|e| {
            log::info!("{account}: copies of a draft in {folder} weren't looked for: {e}");
            Vec::new()
        })
    }

    /// The copy a draft filed replaces, and MailCraft's own earlier copies of it (`own`), gone
    /// from the folder ([`old_copies_gone`]). Only the former's failure is the filing's: an
    /// earlier copy left is a copy too many, and filing the draft again would only add another.
    pub(super) fn remove_copies(&mut self, own: &[MessageKey], replaces: Option<MessageKey>, trash: Option<&str>) -> Result<()> {
        let account = self.id();
        let c = self.client()?;
        if !own.is_empty()
            && let Err(e) = old_copies_gone(c, own, trash)
        {
            log::info!("{account}: an earlier copy of a draft stays: {e}");
        }
        match replaces {
            Some(old) => old_copies_gone(c, &[old], trash),
            None => Ok(()),
        }
    }
}

/// Earlier copies of a draft gone from their folder, and nothing else: removed exactly with
/// UIDPLUS's `UID EXPUNGE`; without it only marked deleted, which MailCraft doesn't show, for the
/// folder's next expunge. A plain `EXPUNGE` removes whatever else is marked deleted there, another
/// program's mail too, and moving each to Trash fills it with every version saved (found in
/// review: every save without UIDPLUS did the one, then the other).
fn old_copies_gone<S: std::io::Read + std::io::Write>(c: &mut Client<S>, keys: &[MessageKey], trash: Option<&str>) -> Result<()> {
    if c.has("UIDPLUS") {
        return expunge_keys(c, keys, trash);
    }
    by_folder(c, keys, |c, _, set| c.uid_store(set, true, Flag::Deleted.imap()))
}
