//! Junk Email lists: blocked senders, whose mail goes to Junk as it arrives, and safe senders,
//! whose mail is taken back out of Junk (and whose pictures load). An entry is an address or a
//! whole domain (`@example.com`).

use super::*;

/// The most entries a list keeps.
pub const JUNK_LIST_MAX: usize = 10_000;

/// Which list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JunkList {
    Blocked,
    Safe,
}

/// An entry as it's kept: a lower-case address, or `@domain`; `None` when it's neither.
pub fn junk_entry(raw: &str) -> Option<String> {
    let v = raw.trim().trim_matches(|c| c == '<' || c == '>').to_ascii_lowercase();
    if v.len() > 320 {
        return None;
    }
    match v.split_once('@') {
        Some(("", domain)) => is_domain(domain).then(|| format!("@{domain}")),
        Some((local, domain)) => {
            (!local.is_empty() && !local.contains(char::is_whitespace) && is_domain(domain) && mailcraft_model::addr::is_valid_email(&v)).then_some(v)
        }
        None => is_domain(&v).then(|| format!("@{v}")),
    }
}

/// `example.com`, `mail.example.co.uk`: dotted labels of letters, digits and hyphens.
fn is_domain(d: &str) -> bool {
    let labels: Vec<&str> = d.split('.').collect();
    labels.len() >= 2
        && d.len() <= 253
        && labels.iter().all(|l| {
            !l.is_empty() && l.len() <= 63 && !l.starts_with('-') && !l.ends_with('-') && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        })
}

/// Whether an address is on a list: the address itself, or its domain.
pub fn junk_listed(list: &BTreeSet<String>, email: &str) -> bool {
    if list.is_empty() {
        return false;
    }
    let email = email.trim().to_ascii_lowercase();
    let Some((_, domain)) = email.rsplit_once('@') else { return false };
    !domain.is_empty() && (list.contains(&email) || list.contains(&format!("@{domain}")))
}

impl Session {
    fn junk_list_mut(&mut self, which: JunkList) -> &mut BTreeSet<String> {
        match which {
            JunkList::Blocked => &mut self.prefs.blocked_senders,
            JunkList::Safe => &mut self.prefs.safe_senders,
        }
    }

    /// Put an entry on a list (off the other one), or take it off. The entry as it's kept.
    pub fn set_junk_entry(&mut self, which: JunkList, raw: &str, on: bool) -> Result<String, CmdError> {
        let entry = junk_entry(raw).ok_or_else(|| CmdError::Params(format!("`{}` is not an address or a domain", raw.trim())))?;
        if on {
            // Refused before anything changes: a full list left the entry off the other one too.
            let list = self.junk_list_mut(which);
            if list.len() >= JUNK_LIST_MAX && !list.contains(&entry) {
                return Err(CmdError::Failed(format!("the list is full ({JUNK_LIST_MAX} entries)")));
            }
            list.insert(entry.clone());
            let other = if which == JunkList::Blocked { JunkList::Safe } else { JunkList::Blocked };
            self.junk_list_mut(other).remove(&entry);
        } else {
            self.junk_list_mut(which).remove(&entry);
        }
        Ok(entry)
    }

    /// Whether mail from this address goes to Junk: blocked, and not also safe.
    pub fn sender_blocked(&self, email: &str) -> bool {
        junk_listed(&self.prefs.blocked_senders, email) && !junk_listed(&self.prefs.safe_senders, email)
    }

    pub fn sender_safe(&self, email: &str) -> bool {
        junk_listed(&self.prefs.safe_senders, email)
    }

    /// The selection's senders put on a list (`domain`: their domains), as Outlook's Junk menu.
    pub fn junk_list_selection(&mut self, which: JunkList, domain: bool) -> CmdResult {
        let sel = self.selected_shared();
        if sel.is_empty() {
            return Err(CmdError::Disabled("no message selected".into()));
        }
        let (mut added, mut refused) = (Vec::new(), None);
        for m in &sel {
            let email = m.from.email.trim().to_ascii_lowercase();
            let raw = if domain { email.rsplit_once('@').map(|(_, d)| d.to_string()).unwrap_or_default() } else { email };
            match self.set_junk_entry(which, &raw, true) {
                Ok(e) if !added.contains(&e) => added.push(e),
                Ok(_) => {}
                // The list is full: said, rather than that there's no sender.
                Err(e @ CmdError::Failed(_)) => refused = Some(e),
                // No address to put on it.
                Err(_) => {}
            }
        }
        if added.is_empty() {
            return Err(refused.unwrap_or_else(|| CmdError::Failed("the selected mail has no sender address to add".into())));
        }
        let what = if added.len() == 1 { added.first().cloned().unwrap_or_default() } else { format!("{} senders", added.len()) };
        let moved = match which {
            JunkList::Blocked => {
                self.notice(format!("Mail from {what} now goes to Junk Email"), false);
                Some(self.move_selection(MoveTarget::Role(FolderRole::Junk)))
            }
            JunkList::Safe => {
                self.notice(format!("Mail from {what} is never treated as junk"), false);
                // Out of Junk, as Outlook does when the sender is trusted from there.
                let in_junk = sel.iter().all(|m| self.folder(&m.key.account, &m.key.folder).is_some_and(|f| f.role == FolderRole::Junk));
                in_junk.then(|| self.move_selection(MoveTarget::Role(FolderRole::Inbox)))
            }
        };
        let mut answer = match moved {
            // The lists changed all the same: the mail that couldn't go stays, and that's said.
            Some(Err(e)) => {
                self.notice(format!("The selected mail stays where it is: {e}"), true);
                serde_json::json!({ "added": added, "moved": 0, "notMoved": e.to_string() })
            }
            Some(Ok(v)) => serde_json::json!({ "added": added, "moved": v.get("moved").cloned() }),
            None => serde_json::json!({ "added": added, "moved": null }),
        };
        // Some went on and the list was full for the rest: said too, not only when none did.
        if let Some(e) = refused {
            self.notice(format!("Not every sender was added: {e}"), true);
            if let Some(a) = answer.as_object_mut() {
                a.insert("notAdded".into(), e.to_string().into());
            }
        }
        Ok(answer)
    }

    /// Mail just arrived in an Inbox from blocked senders goes to Junk; returns those keys.
    pub(in crate::session) fn block_arrived(&mut self, msgs: &[Summary]) -> Vec<MessageKey> {
        if self.prefs.blocked_senders.is_empty() {
            return Vec::new();
        }
        let mut gone = Vec::new();
        let mut changes = Batch::default();
        for m in msgs {
            if !self.sender_blocked(&m.from.email) {
                continue;
            }
            let a = &m.key.account;
            if let Some(junk) = self.account(a).and_then(|x| x.folder(FolderRole::Junk)).map(|f| f.path.clone())
                && junk != m.key.folder
            {
                changes.add(a, Op::Move { keys: vec![m.key.clone()], to: junk });
                gone.push(m.key.clone());
            }
        }
        self.apply_changes(changes);
        gone
    }

    /// Mail just arrived in Junk from safe senders goes back to the Inbox.
    pub(in crate::session) fn rescue_arrived(&mut self, msgs: &[Summary]) {
        if self.prefs.safe_senders.is_empty() {
            return;
        }
        let mut changes = Batch::default();
        let mut back = Vec::new();
        for m in msgs {
            if !self.sender_safe(&m.from.email) {
                continue;
            }
            let a = &m.key.account;
            if let Some(inbox) = self.account(a).and_then(|x| x.folder(FolderRole::Inbox)).map(|f| f.path.clone())
                && inbox != m.key.folder
            {
                changes.add(a, Op::Move { keys: vec![m.key.clone()], to: inbox.clone() });
                back.push((a.clone(), inbox, m.message_id.clone()));
            }
        }
        self.apply_changes(changes);
        // Seen arriving in the Inbox: it's the server's filing that's put right.
        for k in back {
            self.moved_in.remove(&k);
        }
    }
}
