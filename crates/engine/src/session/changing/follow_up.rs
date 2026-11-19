//! The session: flagging mail for follow-up (Follow Up › a day, Mark Complete, Clear Flag),
//! with the days kept on the server that keeps them or here.

use mailcraft_model::FOLLOW_UP_COMPLETE;
use mailcraft_model::organize::MAX_FOLLOW_UP;

use super::*;

impl Session {
    /// Flag `sel` (or clear the flag): one marked complete no longer is, a fresh flag has no due
    /// day, and a cleared one neither. The changes that put the flags back, by account.
    ///
    /// A server that keeps one status for both (Microsoft Graph's flag) gets the
    /// [`FOLLOW_UP_COMPLETE`] keyword taken off before the flag is set, and put on after it.
    fn set_flagged(&mut self, sel: &[SharedSummary], on: bool) -> Vec<(String, Vec<Op>)> {
        let by_account = keys_by_account(sel.iter().map(|m| m.key.clone()));
        let changed = keys_by_account(sel.iter().filter(|m| m.flags.flagged != on).map(|m| m.key.clone()));
        // Carrying the keyword: complete, or flagged again elsewhere with it left behind.
        let completed = keys_by_account(sel.iter().filter(|m| m.flags.has(FOLLOW_UP_COMPLETE)).map(|m| m.key.clone()));
        for (a, keys) in &completed {
            self.apply_op(a, Op::SetKeyword { keys: keys.clone(), keyword: FOLLOW_UP_COMPLETE.into(), on: false });
        }
        // Where the server keeps a flag's days (gap 9): a flag cleared takes them off, as
        // Outlook's Clear Flag does, while it's still flagged (Graph flags one given days); a
        // completed one flagged again starts without them, once flagged.
        let cleared: Vec<SharedSummary> = sel.iter().filter(|m| !on && m.flags.flagged && m.flags.follow_up_due.is_some()).cloned().collect();
        let restore_cleared = self.send_follow_up(&cleared, None);
        for (a, keys) in by_account {
            self.apply_op(&a, Op::SetFlag { keys, flag: Flag::Flagged, on });
        }
        let fresh: Vec<SharedSummary> =
            sel.iter().filter(|m| on && m.flags.follow_up_complete() && m.flags.follow_up_due.is_some()).cloned().collect();
        let restore_fresh = self.send_follow_up(&fresh, None);
        for m in sel {
            let id = m.follow_up_id();
            // Unflagged: no longer due. Flagged again once complete: a fresh flag, not the one
            // completed (one still flagged keeps its day).
            if !on || m.flags.follow_up_complete() {
                self.prefs.follow_up.remove(&id);
            }
            self.prefs.follow_up_done.remove(&id);
        }
        let mut undo: BTreeMap<String, Vec<Op>> = BTreeMap::new();
        for (a, keys) in changed {
            undo.entry(a).or_default().push(Op::SetFlag { keys, flag: Flag::Flagged, on: !on });
        }
        for (a, keys) in completed {
            undo.entry(a).or_default().push(Op::SetKeyword { keys, keyword: FOLLOW_UP_COMPLETE.into(), on: true });
        }
        // The days back: those a clear took off once flagged again, those a fresh flag took off
        // while it's still flagged.
        then(then(restore_fresh, undo.into_iter().collect()), restore_cleared)
    }

    /// Whether `account`'s server keeps a flag's days on the message (gap 9), as Outlook sets
    /// them: Exchange through EWS, Exchange Online through Microsoft Graph. IMAP and JMAP keep
    /// none: MailCraft keeps those itself.
    pub(crate) fn server_keeps_follow_up(&self, account: &str) -> bool {
        self.account(account).is_some_and(|a| !a.is_local() && (a.config.ews.is_some() || a.config.graph.is_some()))
    }

    /// Give `sel`'s messages on such servers a follow-up's day (`None`: none), those that
    /// haven't it already. The changes that put back the days they had, by account.
    fn send_follow_up(&mut self, sel: &[SharedSummary], due: Option<i64>) -> Vec<(String, Vec<Op>)> {
        let clock = self.clock;
        let day = |d: Option<i64>| d.map(|day| mailcraft_model::FollowUpDay { day, at: clock.midnight(day) });
        let mut send: BTreeMap<String, Vec<MessageKey>> = BTreeMap::new();
        let mut back: BTreeMap<(String, Option<i64>), Vec<MessageKey>> = BTreeMap::new();
        for m in sel.iter().filter(|m| m.flags.follow_up_due != due && self.server_keeps_follow_up(&m.key.account)) {
            send.entry(m.key.account.clone()).or_default().push(m.key.clone());
            back.entry((m.key.account.clone(), m.flags.follow_up_due)).or_default().push(m.key.clone());
        }
        for (a, keys) in send {
            self.apply_op(&a, Op::SetFollowUp { keys, due: day(due) });
        }
        back.into_iter().map(|((a, was), keys)| (a, vec![Op::SetFollowUp { keys, due: day(was) }])).collect()
    }

    /// Each of `sel`'s due and done days as they are, for Undo to put back.
    fn follow_up_days_of<S: std::borrow::Borrow<Summary>>(&self, sel: &[S]) -> Vec<(String, Option<i64>, Option<i64>)> {
        sel.iter()
            .map(|m| {
                let id = m.borrow().follow_up_id();
                let (due, done) = (self.prefs.follow_up.get(&id).copied(), self.prefs.follow_up_done.get(&id).copied());
                (id, due, done)
            })
            .collect()
    }

    /// Remember a change to follow-ups for Undo, when it changed anything: `ops` put the flags
    /// back, `was` the days.
    fn remember_follow_up(&mut self, ops: Vec<(String, Vec<Op>)>, was: Vec<(String, Option<i64>, Option<i64>)>) {
        let days_changed =
            was.iter().any(|(id, due, done)| self.prefs.follow_up.get(id).copied() != *due || self.prefs.follow_up_done.get(id).copied() != *done);
        if ops.iter().any(|(_, o)| !o.is_empty()) || days_changed {
            self.remember(Undo::FollowUp { ops, days: was });
        }
    }

    /// Flag the selection for follow-up by a day (`None`: no date), or (`complete`) mark it
    /// done ([`Self::complete_selection`]).
    pub fn follow_up_selection(&mut self, due: Option<i64>, complete: bool) -> CmdResult {
        if complete {
            return self.complete_selection();
        }
        let sel = self.selected_shared();
        self.follow_up_messages(&sel, due)
    }

    /// Flag these messages, or clear their flags, wherever they are (the To-Do Bar's are in any
    /// folder, issue #160), as [`Session::flag_selection`] does the selection's.
    pub(crate) fn flag_messages(&mut self, sel: &[SharedSummary], on: bool) -> CmdResult {
        if sel.is_empty() {
            return Err(CmdError::Disabled("no message selected".into()));
        }
        let was = self.follow_up_days_of(sel);
        let undo = self.set_flagged(sel, on);
        self.remember_follow_up(undo, was);
        Ok(serde_json::json!({ "on": on, "count": sel.len() }))
    }

    /// Flag these messages for follow-up by a day (`None`: no date), wherever they are, as
    /// [`Session::follow_up_selection`] does the selection.
    pub(crate) fn follow_up_messages(&mut self, sel: &[SharedSummary], due: Option<i64>) -> CmdResult {
        if sel.is_empty() {
            return Err(CmdError::Disabled("no message selected".into()));
        }
        let was = self.follow_up_days_of(sel);
        let flag_back = self.set_flagged(sel, true);
        // The day on the server that keeps it (gap 9); put back before the flag goes, while
        // it's still flagged. A completed one flagged again has none now (`set_flagged` took
        // them off): given its day again, it's sent.
        let now: Vec<SharedSummary> = sel
            .iter()
            .map(|m| {
                if m.flags.follow_up_complete() && m.flags.follow_up_due.is_some() {
                    let mut fresh = (**m).clone();
                    fresh.flags.follow_up_due = None;
                    SharedSummary::new(fresh)
                } else {
                    m.clone()
                }
            })
            .collect();
        let undo = then(self.send_follow_up(&now, due), flag_back);
        for m in sel {
            let id = m.follow_up_id();
            match due {
                Some(d) if self.prefs.follow_up.len() < MAX_FOLLOW_UP || self.prefs.follow_up.contains_key(&id) => {
                    self.prefs.follow_up.insert(id, d);
                }
                _ => {
                    self.prefs.follow_up.remove(&id);
                }
            }
        }
        self.remember_follow_up(undo, was);
        Ok(serde_json::json!({ "flagged": true, "due": due.map(mailcraft_model::dates::day_string), "count": sel.len() }))
    }

    /// Mark the selection's follow-up complete (Mark Complete): unflagged, with the
    /// [`FOLLOW_UP_COMPLETE`] keyword, and the day it was done kept (as Outlook keeps it). Its
    /// due day stays; Undo puts the flag and both days back as they were, in one step.
    pub fn complete_selection(&mut self) -> CmdResult {
        let sel = self.selected_shared();
        self.complete_messages(sel)
    }

    /// [`Session::complete_selection`] for these messages, wherever they are (the To-Do Bar's
    /// are in any folder).
    pub(crate) fn complete_messages<S: std::borrow::Borrow<Summary>>(&mut self, sel: Vec<S>) -> CmdResult {
        let sel: Vec<&Summary> = sel.iter().map(std::borrow::Borrow::borrow).collect();
        if sel.is_empty() {
            return Err(CmdError::Disabled("no message selected".into()));
        }
        let was = self.follow_up_days_of(&sel);
        let today = mailcraft_model::dates::today(&self.clock);
        let flagged = keys_by_account(sel.iter().filter(|m| m.flags.flagged).map(|m| m.key.clone()));
        let marked = keys_by_account(sel.iter().filter(|m| !m.flags.has(FOLLOW_UP_COMPLETE)).map(|m| m.key.clone()));
        for m in &sel {
            // Done today, unless it was already done (marked complete again).
            let id = m.follow_up_id();
            if !(m.flags.follow_up_complete() && self.prefs.follow_up_done.contains_key(&id))
                && (self.prefs.follow_up_done.len() < MAX_FOLLOW_UP || self.prefs.follow_up_done.contains_key(&id))
            {
                self.prefs.follow_up_done.insert(id, today);
            }
        }
        // Undo: the keyword off before the flag goes back on (see `set_flagged`).
        let mut undo: BTreeMap<String, Vec<Op>> = BTreeMap::new();
        for (a, keys) in &marked {
            undo.entry(a.clone()).or_default().push(Op::SetKeyword { keys: keys.clone(), keyword: FOLLOW_UP_COMPLETE.into(), on: false });
        }
        for (a, keys) in &flagged {
            undo.entry(a.clone()).or_default().push(Op::SetFlag { keys: keys.clone(), flag: Flag::Flagged, on: true });
        }
        for (a, keys) in flagged {
            self.apply_op(&a, Op::SetFlag { keys, flag: Flag::Flagged, on: false });
        }
        for (a, keys) in marked {
            self.apply_op(&a, Op::SetKeyword { keys, keyword: FOLLOW_UP_COMPLETE.into(), on: true });
        }
        self.remember_follow_up(undo.into_iter().collect(), was);
        Ok(serde_json::json!({ "flagged": false, "complete": true, "count": sel.len() }))
    }

    /// When a flagged message is due, if a day was given.
    pub fn follow_up_due(&self, m: &Summary) -> Option<i64> {
        if !m.flags.flagged {
            return None;
        }
        // The server's, where it keeps one (gap 9): as Outlook set it, or as it was set here.
        if self.server_keeps_follow_up(&m.key.account) {
            return m.flags.follow_up_due;
        }
        self.prefs.follow_up.get(&m.follow_up_id()).copied()
    }

    /// When a message's follow-up was marked complete, if it was and the day is known.
    pub fn follow_up_done(&self, m: &Summary) -> Option<i64> {
        if !m.flags.follow_up_complete() {
            return None;
        }
        self.prefs.follow_up_done.get(&m.follow_up_id()).copied()
    }
}

/// A day put back as it was (`None`: none), within [`MAX_FOLLOW_UP`].
pub(super) fn put_day(days: &mut BTreeMap<String, i64>, id: String, day: Option<i64>) {
    match day {
        Some(d) if days.len() < MAX_FOLLOW_UP || days.contains_key(&id) => {
            days.insert(id, d);
        }
        Some(_) => {}
        None => {
            days.remove(&id);
        }
    }
}

/// Undo's changes by account: `first`'s before `next`'s, for each account.
fn then(first: Vec<(String, Vec<Op>)>, next: Vec<(String, Vec<Op>)>) -> Vec<(String, Vec<Op>)> {
    let mut all: BTreeMap<String, Vec<Op>> = BTreeMap::new();
    for (a, ops) in first.into_iter().chain(next) {
        all.entry(a).or_default().extend(ops);
    }
    all.into_iter().collect()
}
