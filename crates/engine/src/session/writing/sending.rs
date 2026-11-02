//! The session: whether a message can be sent from an account. Only an account with a server
//! to send through can: with none set up, or from an Outlook data file or a mailbox kept only on
//! this computer, nothing would go, so nothing says it went (a message written with only a data
//! file open was "sent" into the file's Sent Items). The sample mailbox, a demonstration, files
//! what it sends in its Sent folder.

use super::*;

/// Why mail can't be sent from an account.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SendBlock {
    /// There is no such account (none is set up, or it was removed).
    NoAccount,
    /// An Outlook data file opened here: it holds mail, it has no server to send through.
    DataFile,
    /// A mailbox kept on this computer only, with no server.
    NoServer,
    /// An account that sends over SMTP, without an outgoing server.
    NoOutgoingServer,
}

impl SendBlock {
    /// What the error says (the window says it in the user's language).
    pub fn reason(self) -> &'static str {
        match self {
            SendBlock::NoAccount => "add an account to send mail",
            SendBlock::DataFile => "mail can't be sent from an Outlook data file: add an account to send mail",
            SendBlock::NoServer => "mail can't be sent from a mailbox kept only on this computer: add an account to send mail",
            SendBlock::NoOutgoingServer => "the account has no outgoing (SMTP) server: add one in its settings to send mail",
        }
    }
}

impl Session {
    /// Why mail can't be sent from `account`, or `None` when it can.
    pub fn send_block(&self, account: &str) -> Option<SendBlock> {
        let Some(a) = self.account(account) else { return Some(SendBlock::NoAccount) };
        if a.is_data_file() {
            Some(SendBlock::DataFile)
        } else if a.is_local() {
            (a.config.id != crate::sample::SAMPLE_ACCOUNT).then_some(SendBlock::NoServer)
        } else if !a.config.is_api() && a.config.smtp.host.trim().is_empty() {
            Some(SendBlock::NoOutgoingServer)
        } else {
            None
        }
    }

    /// Why the message being written can't be sent from its account (none written: none).
    pub fn compose_send_block(&self) -> Option<SendBlock> {
        self.compose.as_ref().and_then(|c| self.send_block(&c.account))
    }

    /// `account` when mail can be sent from it; else the first account that can send, as Outlook
    /// writes from the default account in a data file's folders; else `account` still (its
    /// Send says why it can't).
    pub(crate) fn sending_account(&self, account: String) -> String {
        if self.send_block(&account).is_none() {
            return account;
        }
        self.accounts.iter().map(|a| &a.config.id).find(|id| self.send_block(id).is_none()).cloned().unwrap_or(account)
    }

    /// An error unless mail can be sent from `account`.
    pub(crate) fn can_send(&self, account: &str) -> Result<(), CmdError> {
        match self.send_block(account) {
            Some(b) => Err(CmdError::Disabled(b.reason().into())),
            None => Ok(()),
        }
    }
}
