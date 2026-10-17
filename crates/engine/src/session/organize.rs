//! Organising mail: categories, pinning, snoozing, inbox rules and the Focused/Other split.

use mailcraft_model::organize::{Category, Rule, RuleAction, Snooze, actions_for};
use mailcraft_model::{PINNED, is_keyword};

use super::*;

/// The folder snoozed mail waits in.
const SNOOZED: &str = "Snoozed";

impl Session {
    // ----- categories and pins -----

    /// Add or remove a keyword on the selection (`on: None` toggles, following the first message).
    fn keyword_selection(&mut self, keyword: &str, on: Option<bool>) -> CmdResult {
        if !is_keyword(keyword) {
            return Err(CmdError::Params(format!("`{keyword}` can't be used as a keyword")));
        }
        let sel = self.selected();
        let first = sel.first().ok_or_else(|| CmdError::Disabled("no message selected".into()))?;
        let on = on.unwrap_or(!first.flags.has(keyword));
        let mut by_account: HashMap<String, Vec<MessageKey>> = HashMap::new();
        let mut changed: HashMap<String, Vec<MessageKey>> = HashMap::new();
        for m in &sel {
            by_account.entry(m.key.account.clone()).or_default().push(m.key.clone());
            if m.flags.has(keyword) != on {
                changed.entry(m.key.account.clone()).or_default().push(m.key.clone());
            }
        }
        for (a, keys) in changed {
            self.remember(Undo::Ops { account: a, ops: vec![Op::SetKeyword { keys, keyword: keyword.to_string(), on: !on }] });
        }
        for (a, keys) in by_account {
            self.apply_op(&a, Op::SetKeyword { keys, keyword: keyword.to_string(), on });
        }
        Ok(serde_json::json!({ "on": on, "count": sel.len() }))
    }

    /// The category called `name` (any case).
    pub fn category(&self, name: &str) -> Option<&Category> {
        self.prefs.categories.iter().find(|c| c.name.eq_ignore_ascii_case(name.trim()))
    }

    /// The categories a message carries, in the order the user keeps them.
    pub fn categories_of(&self, m: &Summary) -> Vec<&Category> {
        self.prefs.categories.iter().filter(|c| m.flags.has(&c.keyword)).collect()
    }

    pub fn categorize_selection(&mut self, name: &str, on: Option<bool>) -> CmdResult {
        let kw = self.category(name).map(|c| c.keyword.clone()).ok_or_else(|| CmdError::Params(format!("no category called {name}")))?;
        self.keyword_selection(&kw, on)
    }

    pub fn pin_selection(&mut self, on: Option<bool>) -> CmdResult {
        self.keyword_selection(PINNED, on)
    }

    pub fn add_category(&mut self, name: &str, color: Option<u8>) -> CmdResult {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 60 {
            return Err(CmdError::Params("a category needs a name (up to 60 characters)".into()));
        }
        if self.category(name).is_some() {
            return Err(CmdError::Params(format!("there is already a category called {name}")));
        }
        let color = color.unwrap_or_else(|| u8::try_from(self.prefs.categories.len() % 8).unwrap_or(0));
        let c = Category::new(name, color);
        let kw = c.keyword.clone();
        self.prefs.categories.push(c);
        Ok(serde_json::json!({ "created": name, "keyword": kw }))
    }

    /// Forget a category. Messages keep the keyword (another client may still use it).
    pub fn remove_category(&mut self, name: &str) -> CmdResult {
        let before = self.prefs.categories.len();
        self.prefs.categories.retain(|c| !c.name.eq_ignore_ascii_case(name.trim()));
        if self.prefs.categories.len() == before {
            return Err(CmdError::Params(format!("no category called {name}")));
        }
        Ok(serde_json::json!({ "removed": name }))
    }

    // ----- snooze -----

    /// The Snoozed folder of an account, created when missing.
    fn snoozed_folder(&mut self, account: &str) -> Result<String, CmdError> {
        let found = self.account(account).and_then(|a| {
            a.folders.iter().find(|f| f.selectable && f.role == FolderRole::Other && f.display_name().eq_ignore_ascii_case(SNOOZED) && f.depth() <= 1)
        });
        if let Some(f) = found {
            return Ok(f.path.clone());
        }
        let r = self.create_folder(account, None, SNOOZED)?;
        r.get("created").and_then(Value::as_str).map(str::to_string).ok_or_else(|| CmdError::Failed("the Snoozed folder couldn't be made".into()))
    }

    /// Move the selection out of sight until `until` (Unix seconds); it comes back unread to
    /// where it was.
    pub fn snooze_selection(&mut self, until: i64) -> CmdResult {
        if until <= self.clock.now {
            return Err(CmdError::Params("pick a time in the future".into()));
        }
        let sel = self.selected();
        if sel.is_empty() {
            return Err(CmdError::Disabled("no message selected".into()));
        }
        if sel.iter().any(|m| m.message_id.is_empty()) {
            return Err(CmdError::Failed("a message without a Message-ID can't be snoozed".into()));
        }
        let next = self.successor(&self.selection.clone());
        let mut accounts: Vec<String> = sel.iter().map(|m| m.key.account.clone()).collect();
        accounts.sort();
        accounts.dedup();
        for a in accounts {
            let folder = self.snoozed_folder(&a)?;
            let mine: Vec<&Summary> = sel.iter().filter(|m| m.key.account == a && m.key.folder != folder).collect();
            for m in &mine {
                self.snoozes.retain(|s| !(s.account == a && s.message_id == m.message_id));
                self.snoozes.push(Snooze {
                    account: a.clone(),
                    message_id: m.message_id.clone(),
                    folder: folder.clone(),
                    back_to: m.key.folder.clone(),
                    until,
                    subject: m.subject.clone(),
                });
            }
            let keys: Vec<MessageKey> = mine.iter().map(|m| m.key.clone()).collect();
            if !keys.is_empty() {
                self.apply_op(&a, Op::Move { keys, to: folder });
            }
        }
        self.save_snoozes();
        self.after_removal(next);
        Ok(serde_json::json!({ "snoozed": sel.len(), "until": until }))
    }

    /// Bring snoozed messages back now.
    pub fn unsnooze_selection(&mut self) -> CmdResult {
        let sel = self.selected();
        let mut n = 0;
        for m in &sel {
            if let Some(i) = self.snoozes.iter().position(|s| s.account == m.key.account && s.message_id == m.message_id) {
                let s = self.snoozes.remove(i);
                self.wake(&s, m.key.clone());
                n += 1;
            }
        }
        self.save_snoozes();
        Ok(serde_json::json!({ "unsnoozed": n }))
    }

    fn wake(&mut self, s: &Snooze, key: MessageKey) {
        let back = if self.folder(&s.account, &s.back_to).is_some() {
            s.back_to.clone()
        } else {
            self.account(&s.account).and_then(|a| a.folder(FolderRole::Inbox)).map(|f| f.path.clone()).unwrap_or_else(|| "INBOX".into())
        };
        self.apply_op(&s.account, Op::SetFlag { keys: vec![key.clone()], flag: Flag::Seen, on: false });
        self.apply_op(&s.account, Op::Move { keys: vec![key], to: back });
    }

    /// Bring back snoozed mail that is due. Called often (each frame, each CLI run): cheap when
    /// nothing is due.
    pub fn tick(&mut self) {
        self.flush_due_outbox();
        self.read_when_due();
        let now = self.clock.now;
        if !self.snoozes.iter().any(|s| s.until <= now) {
            return;
        }
        let due: Vec<Snooze> = self.snoozes.iter().filter(|s| s.until <= now).cloned().collect();
        let mut changed = false;
        for s in due {
            if self.account(&s.account).is_none() {
                self.snoozes.retain(|x| x != &s);
                changed = true;
                continue;
            }
            let found = self.folder_state(&s.account, &s.folder).messages.iter().find(|m| m.message_id == s.message_id).map(|m| m.key.clone());
            match found {
                Some(key) => {
                    self.snoozes.retain(|x| x != &s);
                    self.wake(&s, key);
                    if let Some(m) = self.list.iter().find(|m| m.message_id == s.message_id).cloned() {
                        self.new_mail.push(m);
                    } else {
                        self.notice(format!("Back from snooze: {}", s.subject), false);
                    }
                    changed = true;
                }
                // Not synced into the Snoozed folder yet; give up after a week.
                None if now > s.until + 7 * 86_400 => {
                    self.snoozes.retain(|x| x != &s);
                    changed = true;
                }
                None => {}
            }
        }
        if changed {
            self.save_snoozes();
        }
    }

    pub(super) fn save_snoozes(&mut self) {
        if let Some(store) = &self.store
            && let Err(e) = store.save_doc("snoozes.json", &self.snoozes)
        {
            self.notice(format!("snoozed mail couldn't be saved: {e}"), true);
        }
    }

    // ----- rules -----

    pub fn set_rules(&mut self, rules: Vec<Rule>) -> CmdResult {
        if rules.len() > 500 {
            return Err(CmdError::Params("at most 500 rules".into()));
        }
        for r in &rules {
            if r.name.trim().is_empty() {
                return Err(CmdError::Params("every rule needs a name".into()));
            }
            for a in &r.actions {
                if let RuleAction::Categorize(c) = a
                    && self.category(c).is_none()
                {
                    return Err(CmdError::Params(format!("rule {}: no category called {c}", r.name)));
                }
            }
        }
        self.rules = rules;
        if let Some(store) = &self.store
            && let Err(e) = store.save_doc("rules.json", &self.rules)
        {
            return Err(CmdError::Failed(format!("the rules couldn't be saved: {e}")));
        }
        Ok(serde_json::json!({ "rules": self.rules.len() }))
    }

    // ----- ignoring conversations -----

    /// The Message-IDs a message names: its own, and those it replies to.
    fn conversation_ids(m: &Summary) -> Vec<String> {
        std::iter::once(&m.message_id)
            .chain(std::iter::once(&m.in_reply_to))
            .chain(&m.references)
            .map(|id| mailcraft_model::thread::norm_id(id).to_ascii_lowercase())
            .filter(|id| !id.is_empty())
            .collect()
    }

    /// Whether a message belongs to a conversation being ignored.
    pub fn is_ignored(&self, m: &Summary) -> bool {
        Self::conversation_ids(m).iter().any(|id| self.prefs.ignored.contains(id))
    }

    /// Ignore the selected conversations, as Outlook's Delete › Ignore: every message of them
    /// goes to Deleted Items, and so does whatever arrives in them later. On conversations already
    /// ignored, stop ignoring them.
    pub fn ignore_selection(&mut self) -> CmdResult {
        let sel = self.selected();
        if sel.is_empty() {
            return Err(CmdError::Disabled("no message selected".into()));
        }
        if sel.iter().all(|m| self.is_ignored(m)) {
            for m in &sel {
                for id in Self::conversation_ids(m) {
                    self.prefs.ignored.remove(&id);
                }
            }
            return Ok(serde_json::json!({ "ignoring": false }));
        }
        // The conversations' messages, in the folders the selection is in.
        let mut members: Vec<Summary> = Vec::new();
        for m in &sel {
            let folder = self.folder_state(&m.key.account, &m.key.folder).messages.clone();
            for t in mailcraft_model::thread::threads(&folder) {
                let msgs: Vec<&Summary> = t.messages.iter().filter_map(|&i| folder.get(i)).collect();
                if msgs.iter().any(|x| x.key == m.key) {
                    members.extend(msgs.into_iter().cloned());
                }
            }
            if !members.iter().any(|x| x.key == m.key) {
                members.push(m.clone());
            }
        }
        for m in &members {
            self.prefs.ignored.extend(Self::conversation_ids(m));
        }
        // Kept bounded: the oldest ids go first (a BTreeSet has no age; dropping any is safe).
        while self.prefs.ignored.len() > 5_000 {
            let Some(first) = self.prefs.ignored.iter().next().cloned() else { break };
            self.prefs.ignored.remove(&first);
        }
        let keys: BTreeSet<MessageKey> = members.iter().map(|m| m.key.clone()).collect();
        self.selection = keys.into_iter().collect();
        let n = self.selection.len();
        self.move_selection(MoveTarget::Role(FolderRole::Trash))?;
        Ok(serde_json::json!({ "ignoring": true, "moved": n }))
    }

    /// New mail in an ignored conversation goes to Deleted Items (and its own id joins the
    /// conversation's). Returns the keys moved away.
    pub(super) fn ignore_arrived(&mut self, msgs: &[Summary]) -> Vec<MessageKey> {
        if self.prefs.ignored.is_empty() {
            return Vec::new();
        }
        let mut gone = Vec::new();
        for m in msgs {
            if !self.is_ignored(m) {
                continue;
            }
            self.prefs.ignored.extend(Self::conversation_ids(m));
            let a = m.key.account.clone();
            if let Some(trash) = self.account(&a).and_then(|x| x.folder(FolderRole::Trash)).map(|f| f.path.clone())
                && trash != m.key.folder
            {
                self.apply_op(&a, Op::Move { keys: vec![m.key.clone()], to: trash });
                gone.push(m.key.clone());
            }
        }
        gone
    }

    /// Run the rules on messages: their actions become ordinary changes. Returns the keys the
    /// rules marked read or sent to Deleted Items or Junk (no need to announce those).
    pub(super) fn run_rules(&mut self, msgs: &[Summary]) -> Vec<MessageKey> {
        let mut quiet = Vec::new();
        if self.rules.is_empty() {
            return quiet;
        }
        let rules = self.rules.clone();
        for m in msgs {
            let actions = actions_for(&rules, m);
            if actions.is_empty() {
                continue;
            }
            let a = m.key.account.clone();
            let keys = vec![m.key.clone()];
            let mut dest: Option<String> = None;
            for act in actions {
                match act {
                    RuleAction::MarkRead => {
                        self.apply_op(&a, Op::SetFlag { keys: keys.clone(), flag: Flag::Seen, on: true });
                        quiet.push(m.key.clone());
                    }
                    RuleAction::Flag => self.apply_op(&a, Op::SetFlag { keys: keys.clone(), flag: Flag::Flagged, on: true }),
                    RuleAction::Pin => self.apply_op(&a, Op::SetKeyword { keys: keys.clone(), keyword: PINNED.into(), on: true }),
                    RuleAction::Categorize(c) => {
                        if let Some(kw) = self.category(&c).map(|c| c.keyword.clone()) {
                            self.apply_op(&a, Op::SetKeyword { keys: keys.clone(), keyword: kw, on: true });
                        }
                    }
                    RuleAction::MoveTo(f) => dest = self.folder(&a, &f).map(|f| f.path.clone()).or(dest),
                    RuleAction::MoveToRole(r) => dest = self.account(&a).and_then(|x| x.folder(r)).map(|f| f.path.clone()).or(dest),
                    RuleAction::Delete => dest = self.account(&a).and_then(|x| x.folder(FolderRole::Trash)).map(|f| f.path.clone()).or(dest),
                }
            }
            // Moves last, so flags and categories are set on the message before it leaves. Mail
            // filed in a folder is still new mail (announced, as Outlook does); mail a rule sends
            // to Deleted Items or Junk isn't.
            if let Some(to) = dest.filter(|to| *to != m.key.folder) {
                let role = self.folder(&a, &to).map(|f| f.role);
                if matches!(role, Some(FolderRole::Trash | FolderRole::Junk)) {
                    quiet.push(m.key.clone());
                }
                self.apply_op(&a, Op::Move { keys, to });
            }
        }
        quiet
    }

    /// Run the rules (or one) now, on the list on screen or (`inbox`) on the mail already in the
    /// Inbox of the accounts they apply to, as Outlook's "run this rule now".
    pub fn run_rules_now(&mut self, only: Option<usize>, inbox: bool) -> CmdResult {
        let saved = self.rules.clone();
        if let Some(i) = only {
            let r = saved.get(i).cloned().ok_or_else(|| CmdError::Params(format!("no rule {i}")))?;
            self.rules = vec![Rule { enabled: true, ..r }];
        }
        let msgs: Vec<Summary> = if inbox {
            let only_account = only.and_then(|i| saved.get(i)).and_then(|r| r.account.clone());
            let inboxes: Vec<(String, String)> = self
                .accounts
                .iter()
                .filter(|a| only_account.as_ref().is_none_or(|o| *o == a.config.id))
                .filter_map(|a| a.folder(FolderRole::Inbox).map(|f| (a.config.id.clone(), f.path.clone())))
                .collect();
            inboxes
                .iter()
                .flat_map(|(a, f)| self.folder_state(a, f).messages.iter().filter(|m| !m.flags.deleted).cloned().collect::<Vec<_>>())
                .collect()
        } else {
            self.list.clone()
        };
        let n = msgs.iter().filter(|m| !actions_for(&self.rules, m).is_empty()).count();
        self.run_rules(&msgs);
        self.rules = saved;
        let what = if n == 1 { "1 message".to_string() } else { format!("{n} messages") };
        self.notice(format!("Rules ran on the mail there now: {what} matched"), false);
        Ok(serde_json::json!({ "matched": n }))
    }

    // ----- Focused / Other -----

    /// Whether a message belongs in Focused: the user's choice for its sender, else anything not
    /// sent in bulk, and bulk mail from people the user writes to.
    pub fn is_focused(&self, m: &Summary) -> bool {
        let sender = m.from.email.trim().to_ascii_lowercase();
        if let Some(&f) = self.prefs.focus_overrides.get(&sender) {
            return f;
        }
        !m.bulk || self.contacts.as_ref().and_then(|b| b.get(&sender)).is_some_and(|c| c.sent > 0)
    }

    /// Whether the list shows the Focused/Other split (an Inbox, with the option on).
    pub fn focus_split(&self) -> bool {
        self.prefs.focused_inbox && self.search.is_none() && self.view_role() == Some(FolderRole::Inbox)
    }

    /// Always put the selection's senders in Focused (or Other).
    pub fn set_focus_for_selection(&mut self, focused: bool) -> CmdResult {
        let sel = self.selected();
        if sel.is_empty() {
            return Err(CmdError::Disabled("no message selected".into()));
        }
        for m in &sel {
            let sender = m.from.email.trim().to_ascii_lowercase();
            if !sender.is_empty() && self.prefs.focus_overrides.len() < 10_000 {
                self.prefs.focus_overrides.insert(sender, focused);
            }
        }
        self.reading = None;
        self.selection.clear();
        self.refresh_list();
        Ok(serde_json::json!({ "focused": focused, "senders": sel.len() }))
    }
}
