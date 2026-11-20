//! The changes a user makes to the mailbox, kept until the server has them.

use serde::{Deserialize, Serialize};

use crate::{Flag, MessageKey};

/// A change to the mailbox the user makes, applied to the local cache at once and replayed on
/// the server when it can be reached (RFC 4549's "pending actions").
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum Op {
    SetFlag {
        keys: Vec<MessageKey>,
        flag: Flag,
        on: bool,
    },
    /// A follow-up's due day, on a server that keeps it on the message (Exchange, Microsoft
    /// 365), as Outlook sets it; `None`: no day. Other servers keep none, and leave it be.
    SetFollowUp {
        keys: Vec<MessageKey>,
        due: Option<FollowUpDay>,
    },
    /// A flag's reminder (Unix seconds), on a server that keeps it on the message (Exchange,
    /// Microsoft 365), as Outlook sets it; `None`: none (dismissed). Other servers keep none,
    /// and leave it be.
    SetReminder {
        keys: Vec<MessageKey>,
        at: Option<i64>,
    },
    /// Add or remove a keyword (a category, `$Pinned`).
    SetKeyword {
        keys: Vec<MessageKey>,
        keyword: String,
        on: bool,
    },
    /// Move to another folder of the same account.
    Move {
        keys: Vec<MessageKey>,
        to: String,
    },
    /// Delete permanently (`\Deleted` + `UID EXPUNGE`).
    Expunge {
        keys: Vec<MessageKey>,
    },
    /// Create a folder (`CREATE`, then `SUBSCRIBE`). `path` is the server's name for it.
    CreateFolder {
        path: String,
    },
    /// Rename or move a folder; folders inside it move with it (`RENAME`).
    RenameFolder {
        from: String,
        to: String,
    },
    /// Delete a folder and the mail in it (`DELETE`). The engine deletes subfolders first.
    DeleteFolder {
        path: String,
    },
    /// Delete every message in a folder for good (Deleted Items, Junk Email), including those
    /// not downloaded.
    EmptyFolder {
        path: String,
    },
    /// Mark every message in a folder read, including those not downloaded (issue #197: the
    /// unread mail on the server past what's kept here stayed unread).
    MarkFolderRead {
        path: String,
    },
}

/// A follow-up's due day: the day (days since the epoch) and when it starts where the user is
/// (its local midnight, Unix seconds). Outlook keeps both: the day as a date, and that instant
/// (MS-OXOFLAG: `PidLidTaskDueDate` and `PidLidCommonEnd`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FollowUpDay {
    pub day: i64,
    pub at: i64,
}

impl Op {
    /// The messages a change touches (none for folder changes).
    pub fn keys(&self) -> &[MessageKey] {
        match self {
            Op::SetFlag { keys, .. }
            | Op::SetFollowUp { keys, .. }
            | Op::SetReminder { keys, .. }
            | Op::SetKeyword { keys, .. }
            | Op::Move { keys, .. }
            | Op::Expunge { keys } => keys,
            Op::CreateFolder { .. } | Op::RenameFolder { .. } | Op::DeleteFolder { .. } | Op::EmptyFolder { .. } | Op::MarkFolderRead { .. } => &[],
        }
    }

    /// The folder a change makes to all of its mail ([`Op::EmptyFolder`], [`Op::MarkFolderRead`]),
    /// when it's one of those.
    pub fn whole_folder(&self) -> Option<&str> {
        match self {
            Op::EmptyFolder { path } | Op::MarkFolderRead { path } => Some(path),
            _ => None,
        }
    }

    /// Whether this changes the folder list rather than messages.
    pub fn is_folder_op(&self) -> bool {
        matches!(self, Op::CreateFolder { .. } | Op::RenameFolder { .. } | Op::DeleteFolder { .. })
    }
}
