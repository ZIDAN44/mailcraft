//! An account's worker: the mailbox's rules on its server, read and changed: Exchange's Inbox
//! rules (issue #139), and on an IMAP server MailCraft's rules written as Sieve through
//! ManageSieve (issue #243); their folders told by their paths here.

use mailcraft_model::organize::{NO_SERVER_RULES, ServerRuleOp, ServerRules};

use super::*;

/// The role of a folder Exchange names by itself in a rule (`DistinguishedFolderId`).
fn role_of(distinguished: &str) -> Option<FolderRole> {
    Some(match distinguished {
        "inbox" => FolderRole::Inbox,
        "drafts" => FolderRole::Drafts,
        "sentitems" => FolderRole::Sent,
        "deleteditems" => FolderRole::Trash,
        "junkemail" => FolderRole::Junk,
        "archive" => FolderRole::Archive,
        _ => return None,
    })
}

impl Worker {
    /// Read the rules, or change them (`set`: the changes, and whether Outlook's own copy of the
    /// rules goes) and read them again; said in [`Event::ServerRules`], with why a change failed
    /// beside the rules as the server still has them.
    /// `keywords`: each category's keyword by its name (rules written as Sieve set them).
    pub(crate) fn server_rules(&mut self, set: Option<(Vec<ServerRuleOp>, bool)>, keywords: &[(String, String)]) {
        let r = self.connected_for_job().map_err(NetError::No).and_then(|()| self.rules_on_server(set.as_ref(), keywords));
        let event = match r {
            Ok(rules) => Event::ServerRules { account: self.id(), rules: Some(rules), error: None },
            Err(e) if set.is_some() => {
                log::info!("{}: changing the rules on the server: {e}", self.acc.id);
                let back = self.rules_on_server(None, keywords).ok();
                Event::ServerRules { account: self.id(), rules: back, error: Some(e.to_string()) }
            }
            Err(e) => {
                log::info!("{}: the rules on the server: {e}", self.acc.id);
                Event::ServerRules { account: self.id(), rules: None, error: Some(e.to_string()) }
            }
        };
        self.sink.emit(event);
    }

    fn rules_on_server(&mut self, set: Option<&(Vec<ServerRuleOp>, bool)>, keywords: &[(String, String)]) -> Result<ServerRules> {
        if self.acc.is_imap() {
            return self.sieve_rules(set.map(|(ops, _)| ops.as_slice()), keywords);
        }
        let Some(Conn::Ews(c)) = &self.conn else {
            return Err(NetError::No("only an Exchange account keeps rules on its server that MailCraft can show".into()));
        };
        let (jstate, folders) = (&self.jstate, &self.folders);
        // A folder a rule names, by its path here (its change key aside: only its id says
        // which folder it is).
        let path = |f: crate::ews::RuleFolder<'_>| -> Option<String> {
            match f {
                crate::ews::RuleFolder::Id(id) => jstate.mailboxes.iter().find(|(_, v)| v.as_str() == id).map(|(p, _)| p.clone()),
                crate::ews::RuleFolder::Distinguished(name) => {
                    let role = role_of(name)?;
                    folders.iter().find(|f| f.role == role).map(|f| f.path.clone())
                }
            }
        };
        let id = |p: &str| jstate.mailboxes.get(p).cloned();
        if let Some((ops, drop_outlook_list)) = set {
            c.update_inbox_rules(ops, *drop_outlook_list, &path, &id)?;
        }
        c.inbox_rules(&path)
    }

    /// MailCraft's rules in its Sieve script (issue #243), changed first by `ops`: written with
    /// the account's folders by role and its categories' keywords. A server without ManageSieve
    /// keeps none ([`NO_SERVER_RULES`]: the account has no rules on the server to show).
    fn sieve_rules(&mut self, ops: Option<&[ServerRuleOp]>, keywords: &[(String, String)]) -> Result<ServerRules> {
        // The automatic replies' changes wait (one script holds both).
        let lock = self.sieve_lock.clone();
        let _turn = lock.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut s = self.sieve().map_err(|e| match e {
            NetError::Io(_) => NetError::No(NO_SERVER_RULES.into()),
            e => e,
        })?;
        if let Some(ops) = ops {
            let cx = crate::sieve::RulesContext {
                roles: self.folders.iter().map(|f| (f.role, f.path.clone())).collect(),
                keywords: keywords.to_vec(),
                extensions: Vec::new(),
            };
            if let Err(e) = s.set_server_rules(ops, &cx) {
                s.logout();
                return Err(e);
            }
        }
        let rules = s.server_rules();
        s.logout();
        rules
    }
}
