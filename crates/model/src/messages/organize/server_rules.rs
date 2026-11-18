//! An Exchange mailbox's own Inbox rules, kept and run by the server (issue #139), as MailCraft
//! shows and changes them: each one as the server has it, and as MailCraft's own [`Rule`] when
//! everything it does can be said as one, else shown as it is and only turned on or off,
//! renamed, moved or deleted.

use serde::{Deserialize, Serialize};

use super::Rule;

/// One of the mailbox's rules on the server.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerRule {
    /// The server's id for it (changed when Outlook saves the rules again).
    pub id: String,
    pub name: String,
    /// Its place in the order the server runs them, from 1.
    pub priority: u32,
    pub enabled: bool,
    /// The server can't run it as it is (Outlook's "this rule is in error").
    #[serde(default)]
    pub in_error: bool,
    /// Made in a program that can do more than the server can say: it can't be changed here.
    #[serde(default)]
    pub not_supported: bool,
    /// What it does as MailCraft's own rule, when all of it can be said so; `None`: shown, run
    /// and kept as it is.
    #[serde(default)]
    pub rule: Option<Rule>,
    /// Its conditions, exceptions and actions as the server wrote them: sent back as they came
    /// whenever only its name, order or switch changes, and compared to see whether another
    /// program changed it meanwhile.
    #[serde(default)]
    pub raw: String,
}

/// The mailbox's rules on the server, in the order it runs them.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerRules {
    /// Outlook keeps its own copy of the rules beside the server's: changing them here removes
    /// it, and with it Outlook's rules that are turned off or run only on its computer.
    pub outlook_list: bool,
    pub rules: Vec<ServerRule>,
}

/// A change to the mailbox's rules on the server.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum ServerRuleOp {
    /// A new rule, made of MailCraft's own (`now.rule`).
    Create {
        now: ServerRule,
    },
    /// `was` (as it was read) becomes `now`: what MailCraft can say of it changed when `now.rule`
    /// differs from `was.rule`, else only its name, order or switch.
    Set {
        was: ServerRule,
        now: ServerRule,
    },
    Delete {
        was: ServerRule,
    },
}
