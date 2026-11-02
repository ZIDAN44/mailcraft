//! Meeting mail out of the Inbox once it's dealt with (issue #31), as Outlook's "Delete meeting
//! requests and notifications from Inbox after responding": once the user answers an
//! invitation (in the message, or in the calendar), the invitation and its earlier versions in
//! the Inbox go to Deleted Items or a folder of the user's; a cancellation once it's taken in,
//! and a guest's answer once it's recorded (when asked for). They're ordinary moves, so other
//! clients see them too.
//!
//! Which messages carry a meeting is only known from their bodies: those not downloaded yet
//! stay where they are.

use super::*;

/// The newest messages of the Inbox looked at for a meeting's mail.
const MAX_LOOKED_AT: usize = 2000;
/// Most of their sources read in one go (a big Inbox's attachments aren't all read through).
const MAX_READ: usize = 64 << 20;
/// A message bigger than this isn't looked through for a calendar.
const MAX_MESSAGE: usize = 8 << 20;

/// What becomes of a meeting invitation's mail once it's answered (issue #31).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AnsweredInvites {
    /// Left where it is.
    #[default]
    Keep,
    /// To Deleted Items (Outlook's way, on at first there).
    Delete,
    /// To the folder chosen for its account (`Prefs::answered_folders`); left where it is in an
    /// account without one.
    Move,
}

impl AnsweredInvites {
    /// Settings saved before the option came keep their mail where it was.
    pub fn kept() -> Self {
        AnsweredInvites::Keep
    }
}

impl Session {
    /// Where an account's answered meeting mail goes, if anywhere: Deleted Items, or the folder
    /// chosen (one that's still there).
    pub fn answered_folder(&self, account: &str) -> Option<String> {
        let a = self.account(account)?;
        match self.prefs.answered_invites {
            AnsweredInvites::Keep => None,
            AnsweredInvites::Delete => a.folder(FolderRole::Trash).map(|f| f.path.clone()),
            AnsweredInvites::Move => {
                let path = self.prefs.answered_folders.get(account)?;
                a.folders.iter().find(|f| &f.path == path && f.selectable).map(|f| f.path.clone())
            }
        }
    }

    /// Move a meeting's mail out of the way once it's dealt with: the message it was dealt with
    /// in (`also`), and the account's Inbox messages carrying the same meeting (`uid`) with one
    /// of `methods`, which name only the occurrences in `only` (`None`: any) and are no newer
    /// than `upto`, the version dealt with (none: any version): an update not taken in yet (held, or with tentative
    /// invitations off) stays, to be seen and answered. Returns how many moved.
    pub(super) fn tidy_meeting_mail(
        &mut self,
        account: &str,
        uid: &str,
        methods: &[&str],
        only: Option<&[i64]>,
        upto: &[CalEvent],
        also: Option<&MessageKey>,
    ) -> usize {
        let Some(to) = self.answered_folder(account) else { return 0 };
        let mut keys: Vec<MessageKey> = Vec::new();
        // The message answered, wherever it is (not out of Sent Items, Drafts or the Outbox).
        if let Some(k) = also {
            let role = self.folder(&k.account, &k.folder).map(|f| f.role);
            if k.folder != to && !matches!(role, Some(FolderRole::Trash | FolderRole::Sent | FolderRole::Drafts)) && k.folder != OUTBOX {
                keys.push(k.clone());
            }
        }
        let inbox = self.account(account).and_then(|a| a.folder(FolderRole::Inbox)).map(|f| f.path.clone());
        if let Some(inbox) = inbox.filter(|i| *i != to) {
            // Its read marks still being written are waited for (not taken for an empty Inbox).
            if let Err(e) = self.folders_ready(&[(account.to_string(), inbox.clone())]) {
                log::warn!("{account}: meeting mail isn't tidied: {e}");
            }
            let mut candidates: Vec<MessageKey> = self.folder_state(account, &inbox).messages.iter().map(|m| m.key.clone()).collect();
            candidates.sort_by_key(|a| std::cmp::Reverse(a.uid));
            candidates.truncate(MAX_LOOKED_AT);
            let mut budget = MAX_READ;
            for k in candidates {
                if budget == 0 {
                    break;
                }
                if !keys.contains(&k) && self.carries_meeting(&k, uid, methods, only, upto, &mut budget) {
                    keys.push(k);
                }
            }
        }
        self.move_meeting_mail(keys, &to)
    }

    /// Move these messages (of one account) to a folder, each from its own. The one being read
    /// among them: the list goes on to the next, as after a delete, so the reading pane, the
    /// focus and the anchor of a Shift selection (issue #26) aren't lost with it (they were: Down
    /// went back to the top of the list).
    fn move_meeting_mail(&mut self, keys: Vec<MessageKey>, to: &str) -> usize {
        let n = keys.len();
        let read = self.reading.as_ref().map(|r| r.key.clone()).filter(|k| keys.contains(k));
        // The one after it in the list (else before it), not one of those going: an earlier
        // version further down doesn't take the list there.
        let next = read.as_ref().and_then(|k| {
            let at = self.index_of(k)?;
            let stays = |m: &&Summary| !keys.contains(&m.key);
            self.list.iter().skip(at + 1).find(stays).or_else(|| self.list.iter().take(at).rev().find(stays)).map(|m| m.key.clone())
        });
        let mut by_folder: Vec<(String, Vec<MessageKey>)> = Vec::new();
        for k in keys {
            match by_folder.iter_mut().find(|(a, f)| *a == k.account && f.first().is_some_and(|x| x.folder == k.folder)) {
                Some((_, list)) => list.push(k),
                None => by_folder.push((k.account.clone(), vec![k])),
            }
        }
        for (account, keys) in by_folder {
            self.apply_op(&account, Op::Move { keys, to: to.to_string() });
        }
        if read.is_some() {
            self.after_removal(next);
        }
        n
    }

    /// A guest's answer recorded in the user's meeting: out of the way too, when asked for.
    pub(super) fn tidy_reply_mail(&mut self, key: &MessageKey) {
        if !self.prefs.answered_replies {
            return;
        }
        if let Some(to) = self.answered_folder(&key.account).filter(|to| *to != key.folder) {
            self.move_meeting_mail(vec![key.clone()], &to);
        }
    }

    /// Whether a downloaded message carries this meeting with one of these methods, naming only
    /// the occurrences in `only` (when given), in no newer version than `upto`.
    fn carries_meeting(&self, key: &MessageKey, uid: &str, methods: &[&str], only: Option<&[i64]>, upto: &[CalEvent], budget: &mut usize) -> bool {
        // Most mail has no calendar: told from its source without parsing it.
        let Some(raw) = self.store.as_ref().and_then(|s| s.read_body(key)) else { return false };
        *budget = budget.saturating_sub(raw.len());
        if raw.len() > MAX_MESSAGE || (!contains_ignore_case(&raw, b"text/calendar") && !contains_ignore_case(&raw, b".ics")) {
            return false;
        }
        let Some((cal, _)) = self.message_invitation(key) else { return false };
        if !cal.method.as_deref().is_some_and(|m| methods.iter().any(|x| m.eq_ignore_ascii_case(x))) {
            return false;
        }
        let parts: Vec<&CalEvent> = cal.events.iter().filter(|e| e.uid == uid).collect();
        !parts.is_empty()
            && only.is_none_or(|only| parts.iter().all(|e| e.recurrence_id.is_some_and(|r| only.contains(&r))))
            && !parts.iter().any(|e| newer_than(e, upto))
    }
}

/// Whether `hay` holds `needle`, ASCII case aside.
fn contains_ignore_case(hay: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty() && hay.windows(needle.len()).any(|w| w.eq_ignore_ascii_case(needle))
}
