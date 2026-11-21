//! Check Names (issue #211) in the message being written: what `compose.checkNames` answers
//! shown. Names that could be several people are offered to pick from, one at a time; names found
//! nowhere are said; names a directory is still asked about are said, and checked again once it
//! answers.

use crate::dialogs::{CheckNamesForm, NameChoice};
use crate::*;

impl MailApp {
    /// Check the names typed in the message acted on. `again`: the check a directory's answer
    /// brought on, which doesn't say again that it's still asked.
    pub(super) fn check_names(&mut self, again: bool) {
        let r = match self.session.run("compose.checkNames", &json!({})) {
            Ok(r) => r,
            Err(e) => return self.session.notice(e.to_string(), true),
        };
        let Some(id) = self.session.compose.as_ref().map(|c| c.id) else { return };
        let listed = |key: &str| r.get(key).and_then(Value::as_array).cloned().unwrap_or_default();
        let named =
            |v: &[Value]| v.iter().filter_map(|x| x.get("typed").and_then(Value::as_str)).map(|t| format!("“{t}”")).collect::<Vec<_>>().join(", ");
        let (none, pending) = (listed("none"), listed("pending"));
        if !none.is_empty() {
            self.session.notice(tf!("No one found for {names}", names = named(&none)), true);
        }
        if pending.is_empty() {
            self.ui.names_pending = self.ui.names_pending.filter(|(c, _)| *c != id);
        } else {
            if !again {
                self.session.notice(tf!("Still looking up {names} in the directory", names = named(&pending)), true);
            }
            self.ui.names_pending = Some((id, self.session.directory.rev));
        }
        let asks: Vec<NameChoice> = listed("several").iter().filter_map(NameChoice::from_json).collect();
        // One being picked from already stays: what's asked is added after it.
        if let Some(Dialog::CheckNames(f)) = self.ui.dialog.as_mut() {
            for a in asks {
                if !f.asks.iter().any(|b| b.field == a.field && b.typed == a.typed) {
                    f.asks.push(a);
                }
            }
        } else if !asks.is_empty() && self.ui.dialog.is_none() {
            self.ui.dialog = Some(Dialog::CheckNames(Box::new(CheckNamesForm { asks, picked: None })));
            self.ui.dialog_for = Some(id);
        }
    }

    /// Check Names waiting on a directory: checked again once it answered (`end_frame`).
    pub(crate) fn recheck_names(&mut self, acts: &mut Vec<Action>) {
        if let Some((id, rev)) = self.ui.names_pending
            && rev != self.session.directory.rev
        {
            // Asked once: the check says so again if it's still waiting (a message closed
            // meanwhile isn't checked).
            self.ui.names_pending = None;
            acts.push(Action::ForCompose(id, Box::new(Action::RecheckNames)));
        }
    }
}
