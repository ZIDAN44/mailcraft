//! Check Names (issue #211), as Outlook's: the names typed in the message's To, Cc and Bcc that
//! aren't addresses, looked up among the people the user writes with and has saved, and in the
//! organisation's directory (Exchange's). One person is put in; several are offered to pick from
//! ([`Session::pick_name`]); none is said; a directory still being asked is said too, and the
//! window asks again once it answers.

use std::collections::HashMap;

use mailcraft_model::addr::check_names::{self, MAX_PEOPLE};
use serde_json::{Value, json};

use super::*;

/// The recipient boxes, as Check Names says them.
const FIELDS: [&str; 3] = ["to", "cc", "bcc"];

impl Session {
    /// Check the names typed in the message being written: how many were put in, those with
    /// several people to pick from, those found nowhere, and those a directory is still asked
    /// about.
    pub fn check_names(&mut self) -> CmdResult {
        let c = self.composing_mut()?;
        if c.sending {
            return Err(CmdError::Disabled("the message is being sent".into()));
        }
        let texts = [c.to.clone(), c.cc.clone(), c.bcc.clone()];
        // Each name not an address yet, asked of the directories first (a short one too: it's
        // asked for; one they answered lately isn't asked again), then looked up as the boxes'
        // suggestions are.
        let mut typed: Vec<String> = Vec::new();
        for t in &texts {
            for u in check_names::check_names(t, |_| Vec::new()).unresolved {
                if !typed.contains(&u.typed) {
                    typed.push(u.typed);
                }
            }
        }
        self.ensure_contacts();
        let now = self.clock.now;
        let mut found: HashMap<String, Vec<EmailAddress>> = HashMap::new();
        for t in &typed {
            self.ask_directory(t, true, false);
            let mut people: Vec<EmailAddress> =
                self.contacts.as_ref().map(|b| b.suggest(t, MAX_PEOPLE, now)).unwrap_or_default().into_iter().map(|c| c.address).collect();
            people.extend(self.directory_people(t));
            found.insert(t.clone(), people);
        }
        let (mut resolved, mut several, mut none, mut pending) = (0usize, Vec::new(), Vec::new(), Vec::new());
        let mut after: Vec<Option<String>> = Vec::new();
        for (field, text) in FIELDS.iter().zip(texts.iter()) {
            let checked = check_names::check_names_waiting(text, |t| found.get(t).cloned().unwrap_or_default(), |t| self.directory_pending(t));
            resolved = resolved.saturating_add(checked.resolved);
            for u in checked.unresolved {
                if self.directory_pending(&u.typed) {
                    pending.push(json!({"field": field, "typed": u.typed}));
                } else if !u.people.is_empty() {
                    let people: Vec<Value> = u.people.iter().map(|p| json!({"name": p.name, "email": p.email})).collect();
                    several.push(json!({"field": field, "typed": u.typed, "people": people}));
                } else {
                    none.push(json!({"field": field, "typed": u.typed}));
                }
            }
            after.push((checked.resolved > 0).then_some(checked.field));
        }
        let c = self.composing_mut()?;
        // Only a box with someone put in changes: the others stay as typed.
        for (field, text) in [&mut c.to, &mut c.cc, &mut c.bcc].into_iter().zip(after) {
            if let Some(text) = text {
                *field = text;
            }
        }
        Ok(json!({"resolved": resolved, "several": several, "none": none, "pending": pending}))
    }

    /// The person picked for a name Check Names offered several for: put in where `typed` was in
    /// the box `field` (`to`, `cc` or `bcc`).
    pub fn pick_name(&mut self, field: &str, typed: &str, person: &EmailAddress) -> CmdResult {
        if !person.is_valid() {
            return Err(CmdError::Params(format!("`{}` isn't an address", person.email)));
        }
        let c = self.composing_mut()?;
        let text = match field {
            "to" => &mut c.to,
            "cc" => &mut c.cc,
            "bcc" => &mut c.bcc,
            other => return Err(CmdError::Params(format!("`{other}` isn't to, cc or bcc"))),
        };
        let picked = check_names::pick(text, typed, person).ok_or_else(|| CmdError::Disabled(format!("“{typed}” isn't in the box any more")))?;
        *text = picked;
        Ok(json!({"field": field, "text": text.clone()}))
    }
}
