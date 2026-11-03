//! Reactions (RFC 9078, issue #39): an emoji sent in answer to a message, and those that answer
//! the message shown, gathered under it.

use super::*;

/// One emoji's reactions to a message: who sent it, and whether the user is among them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Reactions {
    pub emoji: String,
    pub people: Vec<String>,
    pub mine: bool,
}

/// Reactions read for one message at most (a list's flood of them stops there).
const MAX_REACTIONS: usize = 500;

impl Session {
    /// The reactions answering message `key`: those in the list shown and in the account's Sent
    /// Items as cached, each emoji once with who sent it, the most sent first.
    pub fn reactions(&self, key: &MessageKey) -> Vec<Reactions> {
        let Some(id) = self.summary(key).map(|s| s.message_id.clone()).filter(|id| !id.is_empty()) else { return Vec::new() };
        let me = self.my_addresses();
        let sent = self.account(&key.account).and_then(|a| a.folder(FolderRole::Sent)).map(|f| f.path.clone());
        let cached_sent = sent.and_then(|f| self.cache.get(&(key.account.clone(), f)));
        let answers = self
            .list
            .iter()
            .map(|s| &**s)
            .chain(cached_sent.into_iter().flat_map(|f| f.messages.iter().map(|m| &**m)))
            .filter(|s| s.in_reply_to == id && mime::is_reaction_text(&s.reaction))
            .take(MAX_REACTIONS);
        let mut out: Vec<Reactions> = Vec::new();
        let mut seen: Vec<(String, String)> = Vec::new();
        for s in answers {
            let who = s.from.email.to_ascii_lowercase();
            // One message seen twice (the list shows Sent Items too), or sent again: once.
            if seen.iter().any(|(w, e)| *w == who && *e == s.reaction) {
                continue;
            }
            seen.push((who.clone(), s.reaction.clone()));
            let name = if s.from.name.trim().is_empty() { s.from.email.clone() } else { s.from.name.clone() };
            let mine = me.contains(&who);
            match out.iter_mut().find(|r| r.emoji == s.reaction) {
                Some(r) => {
                    r.people.push(name);
                    r.mine |= mine;
                }
                None => out.push(Reactions { emoji: s.reaction.clone(), people: vec![name], mine }),
            }
        }
        out.sort_by_key(|r| std::cmp::Reverse(r.people.len()));
        out
    }

    /// React to the message shown (or `key`) with `emoji`: a reaction sent to its sender and
    /// everyone else it went to, from the account it came to.
    pub fn react(&mut self, key: Option<MessageKey>, emoji: &str) -> CmdResult {
        if !mime::is_reaction_text(emoji) {
            return Err(CmdError::Params("a reaction is one emoji or a few".into()));
        }
        let shown = self.reading.as_ref().filter(|r| key.as_ref().is_none_or(|k| *k == r.key));
        let Some(r) = shown else { return Err(CmdError::Params("open the message to react to".into())) };
        let Some(original) = r.parsed.clone() else { return Err(CmdError::Disabled("the message is still being read".into())) };
        // A reaction goes unencrypted, its subject with it; and a list's message would have it
        // mailed to the whole list.
        if r.security.as_ref().is_some_and(|sec| sec.lock.is_some()) {
            return Err(CmdError::Disabled("a reaction to encrypted mail would go unencrypted".into()));
        }
        if original.bulk {
            return Err(CmdError::Disabled("reactions aren't sent to lists and newsletters".into()));
        }
        let account = r.key.account.clone();
        if let Some(b) = self.send_block(&account) {
            return Err(CmdError::Disabled(b.reason().to_string()));
        }
        let acc = self.account(&account).ok_or_else(|| CmdError::Failed("the account is gone".into()))?;
        let from = EmailAddress::new(acc.config.name.clone(), acc.config.email.clone());
        let local = acc.is_local();
        let sent_folder = acc.folder(FolderRole::Sent).map(|f| f.path.clone());
        let now = self.clock.now;
        let mid = mime::new_message_id(&from.email, now, self.random());
        let raw = mime::reaction(&original, emoji, &from, &self.my_addresses(), now, &mid).map_err(CmdError::Params)?;
        let recipients = mime::parse(&raw).map(|p| p.to.iter().chain(&p.cc).map(|a| a.email.clone()).collect::<Vec<_>>()).unwrap_or_default();
        if local {
            if let Some(folder) = &sent_folder {
                self.file_locally(&account, folder, &raw, true)?;
            }
        } else {
            let id = self.fresh_id();
            self.jobs.push(Job::Send { account: account.clone(), id, raw, from: from.email.clone(), recipients: recipients.clone(), sent_folder });
        }
        Ok(serde_json::json!({"emoji": emoji, "to": recipients}))
    }
}
