//! The drafts being filed on the server, by every worker of the pool: an account's connection
//! started again (a password or setting changed, Work Offline turned off and on) while the old
//! one is still filing a draft, the new one leaves that draft kept on this computer and sets it
//! aside until the old filing ends (#53). It finds it filed then, and kept no more, or still kept
//! and its own to file (found in review: both filed it, and Drafts had it twice). Meanwhile it
//! goes on with everything else (found in review: it waited for the old filing to end, a minute
//! on a link that hung, and the account with it).

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

/// How often a wait for new mail looks whether a draft set aside can be filed now, as IDLE looks
/// for jobs.
pub(crate) const LOOK_ASIDE_EVERY: Duration = Duration::from_millis(500);

/// The drafts being filed now ([`Filing::begin`]), shared by the pool's workers.
#[derive(Default)]
pub(crate) struct Filing {
    now: Mutex<Vec<OnItsWay>>,
}

/// A draft being filed: its account, and the draft (its source's hash, its Message-ID).
#[derive(Clone, PartialEq)]
struct OnItsWay {
    account: String,
    hash: u64,
    message_id: String,
}

impl OnItsWay {
    fn new(account: &str, hash: u64, message_id: &str) -> OnItsWay {
        OnItsWay { account: account.to_string(), hash, message_id: message_id.to_string() }
    }

    /// Whether `other` is a version of the same draft: the same source, or the same Message-ID.
    fn same(&self, other: &OnItsWay) -> bool {
        self.account == other.account && (self.hash == other.hash || (!self.message_id.is_empty() && self.message_id == other.message_id))
    }
}

/// A draft being filed by this worker, until it's dropped: once filed, after it's kept on this
/// computer no more ([`Filing::begin`]).
pub(crate) struct Filed {
    filing: Arc<Filing>,
    draft: OnItsWay,
}

impl Filing {
    /// A draft (`hash`, its source's; `message_id`) about to be filed for `account`: it's being
    /// filed until what this gives is dropped. `None`, without waiting, when another worker is
    /// filing a version of it.
    pub(crate) fn begin(filing: &Arc<Filing>, account: &str, hash: u64, message_id: &str) -> Option<Filed> {
        let draft = OnItsWay::new(account, hash, message_id);
        let mut now = filing.now.lock().unwrap_or_else(PoisonError::into_inner);
        if now.iter().any(|d| d.same(&draft)) {
            return None;
        }
        now.push(draft.clone());
        Some(Filed { filing: filing.clone(), draft })
    }

    /// Whether a worker is filing a version of `draft`.
    fn busy(&self, draft: &OnItsWay) -> bool {
        self.now.lock().unwrap_or_else(PoisonError::into_inner).iter().any(|d| d.same(draft))
    }
}

impl Drop for Filed {
    fn drop(&mut self) {
        let mut now = self.filing.now.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(i) = now.iter().position(|d| *d == self.draft) {
            now.remove(i);
        }
    }
}

/// The drafts kept on this computer that a worker set aside, another worker filing a version of
/// each ([`Filing::begin`]): the kept drafts are read again once one of those filings ends.
#[derive(Default)]
pub(crate) struct SetAside(Vec<OnItsWay>);

/// Drafts set aside remembered, at most: any others are read again with them.
const MAX_SET_ASIDE: usize = 64;

impl SetAside {
    pub(crate) fn push(&mut self, account: &str, hash: u64, message_id: &str) {
        if self.0.len() < MAX_SET_ASIDE {
            self.0.push(OnItsWay::new(account, hash, message_id));
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub(crate) fn clear(&mut self) {
        self.0.clear();
    }

    /// Whether the other worker's filing of one of them has ended.
    pub(crate) fn ended(&self, filing: &Filing) -> bool {
        self.0.iter().any(|d| !filing.busy(d))
    }
}
