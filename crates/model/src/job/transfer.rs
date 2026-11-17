//! Send / Receive, as Outlook's progress window shows it: each POP or IMAP account's mail going
//! out (its Outbox) and coming in (a POP account's new mail downloaded, an IMAP account's folders
//! updated), how far each has come, and how it ended. Accounts that sync on their own all the
//! time (JMAP, Microsoft Graph, Exchange) have no part in it, as Exchange accounts have none in
//! Outlook's.

use serde::{Deserialize, Serialize};

/// Which way an account's mail goes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TransferKind {
    /// The Outbox going out.
    Sending,
    /// New mail coming in.
    #[default]
    Receiving,
}

/// Where a task is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TransferState {
    /// Asked for (Send / Receive), not begun yet.
    Waiting,
    /// Under way.
    #[default]
    Working,
    /// Ended, all of it done.
    Done,
    /// Ended with an error: what came before it is kept.
    Failed,
    /// Stopped as asked (Cancel All, Cancel Task): what came before is kept.
    Cancelled,
}

impl TransferState {
    /// Waiting or under way.
    pub fn is_open(self) -> bool {
        matches!(self, TransferState::Waiting | TransferState::Working)
    }
}

/// How an account's sending or receiving is going (the newest replaces the ones before).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Transfer {
    pub kind: TransferKind,
    pub state: TransferState,
    /// Messages done of those to do (POP: downloaded; sending: sent). `total` 0 when it isn't
    /// counted in messages (an IMAP account's folders), or there were none.
    pub done: u32,
    pub total: u32,
    /// Their bytes, when the server said each one's size (POP's `LIST`; a message to send).
    pub bytes_done: u64,
    pub bytes_total: u64,
    /// The folder being updated (IMAP), and how many of the account's are done.
    pub folder: String,
    pub folders_done: u32,
    pub folders_total: u32,
    /// Why it failed.
    pub error: Option<String>,
}

impl Transfer {
    /// A task of `kind` under way, nothing done yet.
    pub fn working(kind: TransferKind) -> Transfer {
        Transfer { kind, state: TransferState::Working, ..Transfer::default() }
    }

    /// How far it has come in thousandths, when that can be told: by bytes, else messages, else
    /// folders.
    pub fn permille(&self) -> Option<u16> {
        let part = |done: u64, total: u64| (total > 0).then(|| u16::try_from(done.min(total).saturating_mul(1000) / total).unwrap_or(1000));
        part(self.bytes_done, self.bytes_total)
            .or_else(|| part(u64::from(self.done), u64::from(self.total)))
            .or_else(|| part(u64::from(self.folders_done), u64::from(self.folders_total)))
    }
}
