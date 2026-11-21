//! The session: Send / Receive as Outlook's progress window shows it. Each POP or IMAP account
//! has two tasks, its sending (the Outbox) and its receiving (new mail, its folders), each as it
//! last went ([`Event::Transfer`]): asked for (Send / Receive marks them waiting), under way,
//! done, failed with why, or cancelled. An account's connection failing fails its tasks under
//! way, with the account's words for why. An Exchange (EWS) account has its two as well (issue
//! #231), as Outlook's window has the Exchange account's; JMAP and Microsoft Graph accounts sync on
//! their own all the time and have none.

use mailcraft_model::job::{Transfer, TransferKind, TransferState};

use super::*;

/// An account's sending or receiving, as it last went.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransferRow {
    pub account: String,
    pub transfer: Transfer,
}

/// Send / Receive's tasks, and what the window is asked to show of them.
#[derive(Clone, Debug, Default)]
pub struct SendReceive {
    /// In the accounts' order, each account's sending before its receiving.
    pub rows: Vec<TransferRow>,
    /// A task failed since the window last showed them (the status bar says so until then).
    pub unseen_error: bool,
    /// Send / Receive was asked for by the user (F9): the window shows its progress, unless
    /// told not to ([`Prefs::open_send_receive`]). Taken by the window.
    pub show_asked: bool,
}

impl SendReceive {
    /// The tasks waiting or under way.
    pub fn open(&self) -> impl Iterator<Item = &TransferRow> {
        self.rows.iter().filter(|r| r.transfer.state.is_open())
    }

    /// How many tasks have ended done, and how many there are.
    pub fn completed(&self) -> (usize, usize) {
        (self.rows.iter().filter(|r| r.transfer.state == TransferState::Done).count(), self.rows.len())
    }

    /// The tasks that failed, with why.
    pub fn errors(&self) -> impl Iterator<Item = &TransferRow> {
        self.rows.iter().filter(|r| r.transfer.state == TransferState::Failed)
    }
}

/// Whether an account has a part in Send / Receive: POP, IMAP or Exchange (EWS), with a mail
/// server.
fn takes_part(a: &AccountState) -> bool {
    a.config.has_server() && a.config.sends_and_receives()
}

impl Session {
    /// How an account's sending or receiving goes ([`Event::Transfer`]). One failed without its
    /// own words takes the account's status's.
    pub(crate) fn transfer_went(&mut self, account: &str, mut transfer: Transfer) {
        if !self.account(account).is_some_and(takes_part) {
            return;
        }
        if transfer.state == TransferState::Failed && transfer.error.as_deref().is_none_or(str::is_empty) {
            transfer.error = self.account(account).and_then(|a| status_error(&a.status)).or(transfer.error);
        }
        if transfer.state == TransferState::Failed {
            self.send_receive.unseen_error = true;
        }
        self.put_transfer(account, transfer);
    }

    /// `account`'s task of the transfer's kind, put in its place among the others.
    fn put_transfer(&mut self, account: &str, transfer: Transfer) {
        let kind = transfer.kind;
        let rows = &mut self.send_receive.rows;
        if let Some(r) = rows.iter_mut().find(|r| r.account == account && r.transfer.kind == kind) {
            r.transfer = transfer;
        } else {
            rows.push(TransferRow { account: account.to_string(), transfer });
        }
        // The accounts' order, then sending before receiving.
        let order: Vec<String> = self.accounts.iter().map(|a| a.config.id.clone()).collect();
        let place = |id: &str| order.iter().position(|a| a == id).unwrap_or(usize::MAX);
        self.send_receive.rows.sort_by_key(|r| (place(&r.account), r.transfer.kind));
    }

    /// Send / Receive asked of these accounts: their tasks wait to begin, how far the last ones
    /// came forgotten. Those of accounts not asked stay as they were.
    pub(crate) fn transfers_asked(&mut self, accounts: &[String]) {
        let asked: Vec<String> = accounts.iter().filter(|id| self.account(id).is_some_and(takes_part)).cloned().collect();
        for id in asked {
            for kind in [TransferKind::Sending, TransferKind::Receiving] {
                self.put_transfer(&id, Transfer { kind, state: TransferState::Waiting, ..Transfer::default() });
            }
        }
    }

    /// An account's connection failed, or it went offline: what it had waiting or under way
    /// fails with the status's words, or is cancelled when it went offline as asked.
    pub(crate) fn transfers_stopped(&mut self, account: &str, status: &SyncStatus) {
        let (state, error) = match status {
            SyncStatus::Error { .. } | SyncStatus::NeedsPassword { .. } => (TransferState::Failed, status_error(status)),
            SyncStatus::Offline => (TransferState::Cancelled, None),
            _ => return,
        };
        let mut failed = false;
        for r in self.send_receive.rows.iter_mut().filter(|r| r.account == account && r.transfer.state.is_open()) {
            r.transfer.state = state;
            r.transfer.error.clone_from(&error);
            failed |= state == TransferState::Failed;
        }
        self.send_receive.unseen_error |= failed;
    }

    /// Stop Send / Receive, `account`'s or every account's: what's under way stops where it can
    /// (its end is said by the network side), and what hasn't begun is cancelled now.
    pub fn cancel_transfers(&mut self, account: Option<&str>) -> CmdResult {
        let mut cancelled = 0;
        for r in self.send_receive.rows.iter_mut().filter(|r| account.is_none_or(|a| r.account == a)) {
            if r.transfer.state == TransferState::Waiting {
                r.transfer.state = TransferState::Cancelled;
                cancelled += 1;
            }
        }
        let working = self.send_receive.open().filter(|r| account.is_none_or(|a| r.account == a)).count();
        self.jobs.push(Job::CancelTransfers { account: account.unwrap_or_default().to_string() });
        Ok(serde_json::json!({ "cancelled": cancelled, "stopping": working }))
    }

    /// The window has shown the tasks: a failure is no longer news.
    pub fn transfers_seen(&mut self) {
        self.send_receive.unseen_error = false;
    }

    /// A removed account's tasks go with it.
    pub(crate) fn forget_transfers_of(&mut self, account: &str) {
        self.send_receive.rows.retain(|r| r.account != account);
        let any = self.send_receive.errors().next().is_some();
        self.send_receive.unseen_error &= any;
    }
}

/// Why an account's connection failed, in its status's words.
fn status_error(status: &SyncStatus) -> Option<String> {
    match status {
        SyncStatus::Error { error, .. } | SyncStatus::NeedsPassword { error } if !error.is_empty() => Some(error.clone()),
        _ => None,
    }
}
