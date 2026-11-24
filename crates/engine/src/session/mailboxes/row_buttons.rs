//! The buttons a message in the list shows under the pointer (issue #274: Archive among them,
//! as the user chooses, as Outlook's quick actions).

use serde::{Deserialize, Serialize};

/// What a list row's button does to that message.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RowAction {
    Delete,
    Archive,
    Flag,
    /// Mark read, or unread when it's read.
    Read,
    Pin,
}

/// As many as a row has room for beside its subject.
pub const ROW_ACTIONS_MAX: usize = 4;

impl RowAction {
    /// Every one, in the order a row shows them from its end.
    pub const ALL: [RowAction; 5] = [RowAction::Delete, RowAction::Archive, RowAction::Flag, RowAction::Read, RowAction::Pin];
    /// A row's buttons before any are chosen (as they were before the choice came).
    pub const DEFAULT: [RowAction; 3] = [RowAction::Delete, RowAction::Flag, RowAction::Read];

    pub fn code(self) -> &'static str {
        match self {
            RowAction::Delete => "delete",
            RowAction::Archive => "archive",
            RowAction::Flag => "flag",
            RowAction::Read => "read",
            RowAction::Pin => "pin",
        }
    }

    /// As File › Options lists it.
    pub fn label(self) -> &'static str {
        match self {
            RowAction::Delete => "Delete",
            RowAction::Archive => "Archive",
            RowAction::Flag => "Flag",
            RowAction::Read => "Mark read or unread",
            RowAction::Pin => "Pin",
        }
    }

    pub fn from_code(code: &str) -> Option<RowAction> {
        RowAction::ALL.into_iter().find(|a| a.code() == code)
    }

    /// A choice as it's kept: each once, in [`RowAction::ALL`]'s order, no more than a row
    /// holds (a settings file edited by hand may have anything).
    pub fn tidy(mut chosen: Vec<RowAction>) -> Vec<RowAction> {
        chosen.sort();
        chosen.dedup();
        chosen.truncate(ROW_ACTIONS_MAX);
        chosen
    }
}
