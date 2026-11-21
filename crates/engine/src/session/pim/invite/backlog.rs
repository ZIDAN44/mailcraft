//! Invitations already in the Inbox when an account comes (issue #237): its first look at the
//! Inbox takes nothing in as arriving, so meetings waiting for an answer there never reached the
//! calendar. They're found when asked (offered, not done unasked: one the user deleted from the
//! calendar on purpose can't be told apart): meetings still to come, not in the calendar, not
//! called off by a later message, not answered already (the user's answer in Sent), each in its
//! newest version. Those picked go into the calendar unanswered, as an invitation arriving does
//! (issue #20), with no answer sent.

use std::collections::{HashMap, HashSet};

use super::*;

/// How far back the Inbox is looked through, and at most how many of its messages.
const LOOK_BACK: i64 = 180 * 86_400;
const MAX_LOOKED: usize = 2_000;
/// The most messages asked for at once to be read (their text not on this computer yet).
const MAX_FETCH: usize = 200;

/// An invitation waiting in the Inbox, not in the calendar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WaitingInvite {
    pub key: MessageKey,
    pub uid: String,
    pub summary: String,
    pub start: i64,
    pub end: i64,
    pub organizer: String,
}

impl Session {
    /// The invitations waiting in `account`'s Inbox, soonest first, and how many of its messages
    /// are still to be read for them (asked for now, unless they're in `asked`, the messages asked
    /// for already: looked at again once they're here).
    pub fn waiting_invitations(&mut self, account: &str, asked: &mut HashSet<MessageKey>) -> (Vec<WaitingInvite>, usize) {
        let Some(inbox) = self.account(account).and_then(|a| a.folder(FolderRole::Inbox)).map(|f| f.path.clone()) else {
            return (Vec::new(), 0);
        };
        let sent = self.account(account).and_then(|a| a.folder(FolderRole::Sent)).map(|f| f.path.clone());
        let since = self.clock.now.saturating_sub(LOOK_BACK);
        let recent = |s: &mut Session, folder: &str| -> Vec<MessageKey> {
            let mut m: Vec<(i64, MessageKey)> = s
                .folder_state(account, folder)
                .messages
                .iter()
                .filter(|m| m.when() >= since && !m.flags.deleted)
                .map(|m| (m.when(), m.key.clone()))
                .collect();
            m.sort_by_key(|(when, _)| std::cmp::Reverse(*when));
            m.into_iter().take(MAX_LOOKED).map(|(_, k)| k).collect()
        };
        // The meetings the user answered already, by their answers in Sent (those on this
        // computer: Sent isn't fetched for this).
        let mut answered: HashSet<String> = HashSet::new();
        if let Some(sent) = sent {
            for key in recent(self, &sent) {
                if let Some((ics, _)) = self.message_ics(&key, None) {
                    let cal = ical::parse_in(&ics, self.clock.local_zone());
                    if cal.method.as_deref() == Some("REPLY") {
                        answered.extend(cal.events.into_iter().map(|e| e.uid));
                    }
                }
            }
        }
        let mine = self.my_addresses();
        let now = self.clock.now;
        let (mut requests, mut cancels): (HashMap<String, (CalEvent, MessageKey)>, HashMap<String, u32>) = (HashMap::new(), HashMap::new());
        let (mut fetch, mut reading) = (Vec::new(), 0usize);
        for key in recent(self, &inbox) {
            if self.readable_raw(&key).is_none() {
                reading += 1;
                if fetch.len() < MAX_FETCH && asked.len() < MAX_LOOKED && asked.insert(key.clone()) {
                    fetch.push(key);
                }
                continue;
            }
            let Some((ics, _)) = self.message_ics(&key, None) else { continue };
            let cal = ical::parse_in(&ics, self.clock.local_zone());
            let Some(e) = cal.events.iter().find(|e| e.recurrence_id.is_none()).or_else(|| cal.events.first()).cloned() else { continue };
            match cal.method.as_deref() {
                Some("CANCEL") if e.recurrence_id.is_none() => {
                    let seq = cancels.entry(e.uid.clone()).or_insert(e.sequence);
                    *seq = (*seq).max(e.sequence);
                }
                Some("REQUEST") => {
                    let newer = requests.get(&e.uid).is_none_or(|(o, _)| (e.sequence, e.dtstamp) > (o.sequence, o.dtstamp));
                    if newer {
                        requests.insert(e.uid.clone(), (e, key));
                    }
                }
                _ => {}
            }
        }
        for key in fetch {
            self.jobs.push(Job::FetchBody { key });
        }
        let mut found: Vec<WaitingInvite> = requests
            .into_values()
            .filter(|(e, _)| {
                let theirs = e.organizer.as_ref().is_some_and(|o| !is_mine(&o.email, &mine));
                let to_come = e.end > now || e.rrule.is_some() || !e.rdates.is_empty();
                let called_off = cancels.get(&e.uid).is_some_and(|c| *c >= e.sequence);
                theirs && to_come && !called_off && !answered.contains(&e.uid) && self.stored_parts(&e.uid).is_none()
            })
            .map(|(e, key)| WaitingInvite {
                key,
                uid: e.uid.clone(),
                summary: e.summary.clone(),
                start: e.start,
                end: e.end,
                organizer: e.organizer.as_ref().map(|o| if o.name.is_empty() { o.email.clone() } else { o.name.clone() }).unwrap_or_default(),
            })
            .collect();
        found.sort_by(|a, b| a.start.cmp(&b.start).then_with(|| a.uid.cmp(&b.uid)));
        (found, reading)
    }

    /// Put the invitations of these messages into the calendar, unanswered and with no answer
    /// sent: how many went in.
    pub fn place_waiting_invitations(&mut self, keys: &[MessageKey]) -> usize {
        let mut placed = 0;
        for key in keys.iter().take(MAX_LOOKED) {
            if let Some((ics, sender)) = self.message_ics(key, None)
                && self.take_calendar(&key.account, &ics, &sender)
            {
                placed += 1;
            }
        }
        placed
    }
}
