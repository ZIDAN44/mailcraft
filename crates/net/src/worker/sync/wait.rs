//! Completion tracking for platform-triggered full syncs.

use super::{Job, Pool};
use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Default)]
pub(crate) struct SyncCompletion(Arc<(Mutex<HashMap<u64, SyncProgress>>, Condvar)>);

#[derive(Default)]
struct SyncProgress {
    remaining: usize,
    success: bool,
}

/// Waits for one tracked full sync requested from an Android background worker.
#[derive(Clone, Default)]
pub struct SyncWaiter(SyncCompletion);

impl SyncWaiter {
    /// Wait for the request's account passes. `None` means the time limit elapsed.
    pub fn wait(&self, request_id: u64, timeout: Duration) -> Option<bool> {
        self.wait_while(request_id, timeout, || false)
    }

    /// Wait unless the platform worker was stopped (for example, because network constraints changed).
    pub fn wait_while(&self, request_id: u64, timeout: Duration, mut is_stopped: impl FnMut() -> bool) -> Option<bool> {
        let deadline = Instant::now().checked_add(timeout)?;
        let (progresses, ready) = &*self.0.0;
        let mut progresses = progresses.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        loop {
            if is_stopped() {
                progresses.remove(&request_id);
                return None;
            }
            let progress = progresses.get(&request_id)?;
            if progress.remaining == 0 {
                let success = progress.success;
                progresses.remove(&request_id);
                return Some(success);
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                progresses.remove(&request_id);
                return None;
            }
            progresses = match ready.wait_timeout(progresses, remaining.min(Duration::from_millis(500))) {
                Ok((progresses, _)) => progresses,
                Err(error) => error.into_inner().0,
            };
        }
    }
}

impl SyncCompletion {
    pub(super) fn begin(&self, request_id: u64, accounts: usize) {
        let (progresses, ready) = &*self.0;
        progresses.lock().unwrap_or_else(std::sync::PoisonError::into_inner).insert(request_id, SyncProgress { remaining: accounts, success: true });
        ready.notify_all();
    }

    pub(crate) fn finish(&self, request_id: u64, success: bool) {
        let (progresses, ready) = &*self.0;
        let mut progresses = progresses.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(progress) = progresses.get_mut(&request_id)
            && progress.remaining > 0
        {
            progress.remaining -= 1;
            progress.success &= success;
            ready.notify_all();
        }
    }

    pub(crate) fn cancel_all(&self) {
        let (progresses, ready) = &*self.0;
        let mut progresses = progresses.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        for progress in progresses.values_mut() {
            progress.remaining = 0;
            progress.success = false;
        }
        ready.notify_all();
    }
}

impl Pool {
    /// Queue one full sync for each live account worker and return a waiter for all of them.
    pub fn request_full_sync(&mut self, request_id: u64) -> SyncWaiter {
        let accounts: Vec<String> = self.workers.keys().cloned().collect();
        self.sync_completion.begin(request_id, accounts.len());
        for account in accounts {
            self.submit(Job::SyncAllTracked { account, request_id });
        }
        SyncWaiter(self.sync_completion.clone())
    }
}
