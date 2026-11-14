//! How new mail comes in the browser: by push, the session's event stream (RFC 8620 §7.3) read
//! through `fetch` (an `EventSource` can't send the `Authorization` header), with polling as the
//! fallback.

use mailcraft_engine::model::FolderRole;

use super::{Rcs, Task, client, enqueue, sleep};
use crate::fetch::events;

/// How often the Inbox is checked for new mail, and every how many checks everything is.
const POLL_MS: i32 = 120_000;
const FULL_EVERY: u32 = 8;
/// How long before a push stream that ended is opened again (and how often a stream waits for
/// its account to connect).
const PUSH_RETRY_MS: i32 = 30_000;
const PUSH_WAIT_MS: i32 = 3_000;

/// Check the Inbox every few minutes (and everything now and then), reconnecting when needed.
pub(super) async fn poll(sh: Rcs, id: String, generation: u32) {
    let mut tick = 0u32;
    loop {
        sleep(POLL_MS).await;
        let state = sh.borrow().accounts.get(&id).map(|a| (a.generation, a.client.is_some(), a.password.is_some()));
        let Some((g, connected, password)) = state else { return };
        if g != generation {
            return;
        }
        tick = tick.wrapping_add(1);
        let inbox = sh.borrow().accounts.get(&id).and_then(|a| a.folders.iter().find(|f| f.role == FolderRole::Inbox).map(|f| f.path.clone()));
        let pushing = sh.borrow().accounts.get(&id).is_some_and(|a| a.pushing);
        match (connected, inbox) {
            (false, _) if password => enqueue(&sh, &id, Task::Connect),
            (false, _) => {}
            (true, _) if tick.is_multiple_of(FULL_EVERY) => enqueue(&sh, &id, Task::SyncAll),
            (true, Some(f)) if !pushing => enqueue(&sh, &id, Task::SyncFolder(f)),
            (true, _) => {}
        }
    }
}

fn inbox(sh: &Rcs, id: &str) -> Option<String> {
    sh.borrow().accounts.get(id).and_then(|a| a.folders.iter().find(|f| f.role == FolderRole::Inbox).map(|f| f.path.clone()))
}

fn current(sh: &Rcs, id: &str, generation: u32) -> bool {
    sh.borrow().accounts.get(id).is_some_and(|a| a.generation == generation)
}

fn set_pushing(sh: &Rcs, id: &str, on: bool) {
    if let Some(a) = sh.borrow_mut().accounts.get_mut(id) {
        a.pushing = on;
    }
}

/// Hold the account's push stream open while the account is (RFC 8620 §7.3): a `state` event
/// means something changed, and the Inbox syncs.
pub(super) async fn push(sh: Rcs, id: String, generation: u32) {
    let mut wait = 0;
    loop {
        if wait > 0 {
            sleep(wait).await;
        }
        if !current(&sh, &id, generation) {
            return;
        }
        let Some(c) = client(&sh, &id) else {
            wait = PUSH_WAIT_MS;
            continue;
        };
        // No push from this server: polling it is.
        let Some(url) = c.events_url() else { return };
        let mut opened = false;
        let mut on_event = |e: mailcraft_jmap::sse::Sse| {
            if !current(&sh, &id, generation) {
                return false;
            }
            if !std::mem::replace(&mut opened, true) {
                set_pushing(&sh, &id, true);
            }
            // Mail that came while the stream was closed sends no event: catch up once it's open.
            if (e.event == "state" || e.event == "open")
                && let Some(f) = inbox(&sh, &id)
            {
                enqueue(&sh, &id, Task::SyncFolder(f));
            }
            true
        };
        let r = events(&url, c.authorization(), &mut on_event).await;
        set_pushing(&sh, &id, false);
        if let Err(e) = r {
            log::info!("{id}: push: {e}");
        }
        wait = PUSH_RETRY_MS;
    }
}
