//! An account's worker: its Send / Receive as the window shows it, Outlook's progress window and
//! status bar ([`Event::Transfer`]): the Outbox going out and new mail coming in, how far each
//! has come and how it ended; and stopping it when asked (Cancel All, Cancel Task), between two
//! messages or two folders. A POP, IMAP or Exchange (EWS) account's
//! ([`AccountConfig::sends_and_receives`]): JMAP and Microsoft Graph ones sync on their own all
//! the time, and have no part in it.

use mailcraft_model::job::{Transfer, TransferKind, TransferState};

use super::*;

/// A task under way is said at most this often (a download of thousands of small messages said
/// each one).
const SAY_EVERY: Duration = Duration::from_millis(150);

impl Worker {
    /// Whether the account's Send / Receive was asked to stop.
    pub(crate) fn cancel_asked(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    /// Whether this account has a part in Send / Receive.
    pub(crate) fn sends_and_receives(&self) -> bool {
        self.acc.sends_and_receives()
    }

    /// Say how the account's sending or receiving goes.
    pub(crate) fn tell_transfer(&self, transfer: Transfer) {
        if self.sends_and_receives() && !self.stopped() {
            self.sink.emit(Event::Transfer { account: self.id(), transfer });
        }
    }

    /// A task ended: as it came to `last`, done, or cancelled when that was asked and some of it
    /// was left, or failed with `error` (empty: the account's status says why).
    pub(crate) fn transfer_ended(&self, last: Transfer, error: Option<String>) {
        let left = last.done < last.total || last.folders_done < last.folders_total;
        let state = match &error {
            Some(_) => TransferState::Failed,
            None if left && self.cancel_asked() => TransferState::Cancelled,
            None => TransferState::Done,
        };
        self.tell_transfer(Transfer { state, error, ..last });
    }

    /// A POP account's new mail downloaded into its Inbox (`path`), said as it comes, and how it
    /// ended. For a worker that started when its account had been removed `seen` times.
    pub(crate) fn receive_pop(&mut self, store: &Store, path: &str, seen: u64) -> Result<crate::sync::SyncReport> {
        let mut last = Transfer::working(TransferKind::Receiving);
        self.tell_transfer(last.clone());
        let secret = self.secret();
        let mut pace = Pace::default();
        let mut listed = 0;
        let got = (|| {
            let mut c = crate::pop::open(&self.acc, &secret?)?;
            let mut watch = |f: &crate::pop::Fetching| {
                last = receiving(f);
                if pace.due() {
                    self.tell_transfer(last.clone());
                }
                // A batch saved: the Inbox shows it now, not at the end (a big download's Inbox
                // stayed empty while the status bar counted its messages coming).
                if f.saved > listed && !self.stopped() {
                    listed = f.saved;
                    self.sink.emit(Event::FolderChanged { account: self.id(), folder: path.to_string() });
                }
                !self.cancel_asked()
            };
            let keep = crate::pop::Keep::of(&self.acc);
            let got = crate::pop::fetch_new_as(&mut c, store, &self.acc.id, path, &keep, now(), seen, &mut watch)?;
            let _ = c.quit();
            Ok(got)
        })();
        self.transfer_ended(last, got.as_ref().err().map(ToString::to_string));
        got
    }

    /// A message sent on its own (Send, from the window): one of one.
    pub(crate) fn sending_one(&self) -> Transfer {
        let t = Transfer { total: 1, ..Transfer::working(TransferKind::Sending) };
        self.tell_transfer(t.clone());
        t
    }
}

/// How often a task under way is said: the first time, then every [`SAY_EVERY`].
#[derive(Default)]
pub(crate) struct Pace(Option<Instant>);

impl Pace {
    /// Whether it's time to say it again (noting that it's said).
    pub(crate) fn due(&mut self) -> bool {
        let due = self.0.is_none_or(|at| at.elapsed() >= SAY_EVERY);
        if due {
            self.0 = Some(Instant::now());
        }
        due
    }
}

/// A POP download going, as the Send / Receive window shows it.
pub(crate) fn receiving(f: &crate::pop::Fetching) -> Transfer {
    Transfer { done: f.done, total: f.total, bytes_done: f.bytes_done, bytes_total: f.bytes_total, ..Transfer::working(TransferKind::Receiving) }
}
