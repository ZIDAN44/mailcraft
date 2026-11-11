//! JMAP `EventSource` push: holding the stream open and tracking `StateChange` snapshots.

use super::*;

/// A push stream that closes this soon after opening (a proxy buffering or closing SSE responses)
/// turns push off so the worker polls instead of reopening in a tight loop.
const SHORT_PUSH: Duration = Duration::from_secs(5);

impl Worker {
    /// JMAP push: hold the server's event stream open (as IMAP holds IDLE), checking for jobs
    /// between short reads; a `state` event whose `StateChange.changed` map advances means
    /// something changed. `false` when the server can't push (the caller polls instead).
    pub(super) fn wait_for_push(&mut self, rx: &Receiver<Job>) -> bool {
        let mut opened = false;
        if self.jmap_events.is_none() {
            let Some(Conn::Jmap(c)) = &self.conn else { return false };
            let mut es = match c.events() {
                Ok(Some(es)) => es,
                Ok(None) => {
                    self.no_push = true;
                    return false;
                }
                Err(e @ NetError::Auth(_)) => {
                    self.failed(e);
                    return true;
                }
                Err(e) => {
                    // Push not working (a proxy, an old server): poll from now on.
                    log::info!("{}: no JMAP push: {e}", self.acc.id);
                    self.no_push = true;
                    return false;
                }
            };
            self.jmap_push_state.clear();
            // RFC 8620 §7.3 servers emit an initial `state` snapshot immediately on connect:
            // record that baseline before the catch-up sync so it doesn't trigger a loop.
            match es.next(Duration::from_millis(150)) {
                Ok(Some(e)) if e.event == "state" => {
                    let _ = record_jmap_state(&mut self.jmap_push_state, &e.data);
                }
                Ok(_) => {}
                Err(e) => {
                    log::info!("{}: JMAP push closed right after opening ({e}); polling instead", self.acc.id);
                    self.no_push = true;
                    return false;
                }
            }
            self.jmap_events = Some(es);
            opened = true;
        }
        // Mail that came between the last sync and this stream opening sends no event (unless
        // the server replays it, RFC 8620 §7.3): check once when a stream was newly opened.
        if opened
            && !std::mem::take(&mut self.pushed)
            && let Some(f) = self.inbox()
        {
            self.sync_one(&f);
        }
        if opened {
            self.jmap_push_opened = Some(Instant::now());
        }
        self.working(false);
        let end = Instant::now() + IDLE_FOR;
        let mut changed = false;
        let mut dav_changed = false;
        let mut ended = false;
        while Instant::now() < end && !changed && !dav_changed {
            self.drain(rx);
            if self.closed || !self.queue.is_empty() || self.kept.aside.ended(&self.filing) {
                return true;
            }
            let next = match self.jmap_events.as_mut() {
                Some(es) => es.next(Duration::from_millis(500)),
                None => break,
            };
            match next {
                Ok(Some(e)) if e.event == "state" => {
                    let (m, d) = record_jmap_state(&mut self.jmap_push_state, &e.data);
                    changed = m;
                    dav_changed = d;
                }
                Ok(Some(_) | None) => {}
                Err(e) => {
                    // Dropped: reopened on the next wait unless it closed right after opening.
                    log::info!("{}: JMAP push ended: {e}", self.acc.id);
                    ended = true;
                    break;
                }
            }
        }
        if ended {
            let short = self.jmap_push_opened.is_some_and(|t| t.elapsed() < SHORT_PUSH);
            self.drop_jmap_push();
            if short {
                log::info!("{}: JMAP push closed right after opening; polling instead", self.acc.id);
                self.no_push = true;
            }
        }
        if dav_changed {
            let account = self.id();
            push_job(&mut self.queue, Job::SyncDav { account });
        }
        if changed && let Some(f) = self.inbox() {
            self.sync_one(&f);
            self.pushed = true;
        }
        !self.no_push
    }
}

/// Merge an RFC 8620 §7.3 `StateChange` JSON payload into `seen` and report `(mail_changed, dav_changed)`.
pub(super) fn record_jmap_state(seen: &mut serde_json::Map<String, serde_json::Value>, data: &str) -> (bool, bool) {
    let Ok(val) = serde_json::from_str::<serde_json::Value>(data) else { return (true, false) };
    let Some(changed) = val.get("changed").and_then(serde_json::Value::as_object) else { return (true, false) };
    let mut mail = false;
    let mut dav = false;
    for (acct, types) in changed {
        let Some(types_obj) = types.as_object() else {
            if seen.insert(acct.clone(), types.clone()).as_ref() != Some(types) {
                mail = true;
            }
            continue;
        };
        if !matches!(seen.get(acct), Some(serde_json::Value::Object(_))) {
            seen.insert(acct.clone(), serde_json::Value::Object(serde_json::Map::new()));
        }
        let Some(serde_json::Value::Object(acct_seen)) = seen.get_mut(acct) else { continue };
        for (kind, token) in types_obj {
            if acct_seen.insert(kind.clone(), token.clone()).as_ref() != Some(token) {
                match kind.as_str() {
                    "Email" | "Mailbox" | "Thread" | "EmailDelivery" => mail = true,
                    "Calendar" | "CalendarEvent" | "AddressBook" | "ContactCard" | "Task" | "Todo" | "Note" | "Journal" => dav = true,
                    _ => {}
                }
            }
        }
    }
    (mail, dav)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jmap_push_state_ignores_duplicate_snapshots_and_detects_real_changes() {
        let mut seen = serde_json::Map::new();
        let initial = r#"{"@type":"StateChange","changed":{"a1":{"Email":"e1","Mailbox":"m1","Calendar":"c1"}}}"#;
        assert_eq!(record_jmap_state(&mut seen, initial), (true, true));
        assert_eq!(record_jmap_state(&mut seen, initial), (false, false));
        let cal_only = r#"{"@type":"StateChange","changed":{"a1":{"Calendar":"c2"}}}"#;
        assert_eq!(record_jmap_state(&mut seen, cal_only), (false, true));
        let mail_next = r#"{"@type":"StateChange","changed":{"a1":{"Email":"e2","Mailbox":"m1"}}}"#;
        assert_eq!(record_jmap_state(&mut seen, mail_next), (true, false));
    }
}
