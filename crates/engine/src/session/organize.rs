//! Organising mail: categories, pinning, snoozing, inbox rules and the Focused/Other split.

use mailcraft_model::organize::{Rule, RuleAction, Snooze, actions_for};
use mailcraft_model::{PINNED, is_keyword};

use super::*;

pub mod categories;
pub mod junk;
pub mod quick_steps;
pub mod search_folders;
mod sent_rules;
pub mod server_rules;

/// The folder snoozed mail waits in.
const SNOOZED: &str = "Snoozed";
/// How long a due snooze whose message isn't here yet waits to be looked for again (seconds),
/// unless its folder changes first.
const SNOOZE_LOOK_AGAIN: i64 = 60;

/// When due snoozes were last looked for in their folders ([`Session::tick`]), and since then:
/// whether one of those folders changed, and whether an import held some back.
#[derive(Debug)]
pub(crate) struct SnoozesLooked {
    pub(crate) at: i64,
    pub(crate) folder_changed: bool,
    pub(crate) held: bool,
}

impl Default for SnoozesLooked {
    fn default() -> Self {
        SnoozesLooked { at: i64::MIN, folder_changed: false, held: false }
    }
}

impl Session {
    // ----- categories and pins -----

    /// Add or remove a keyword on the selection (`on: None` toggles, following the first message).
    fn keyword_selection(&mut self, keyword: &str, on: Option<bool>) -> CmdResult {
        if !is_keyword(keyword) {
            return Err(CmdError::Params(format!("`{keyword}` can't be used as a keyword")));
        }
        let sel = self.selected_shared();
        let first = sel.first().ok_or_else(|| CmdError::Disabled("no message selected".into()))?;
        let on = on.unwrap_or(!first.flags.has(keyword));
        self.mark_selection(&sel, on, |m| m.flags.has(keyword), |keys, on| Op::SetKeyword { keys, keyword: keyword.to_string(), on });
        Ok(serde_json::json!({ "on": on, "count": sel.len() }))
    }

    pub fn pin_selection(&mut self, on: Option<bool>) -> CmdResult {
        self.keyword_selection(PINNED, on)
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
        let sel = self.selected_shared();
        if sel.is_empty() {
            return Err(CmdError::Disabled("no message selected".into()));
        }
        if sel.iter().any(|m| m.message_id.is_empty()) {
            return Err(CmdError::Failed("a message without a Message-ID can't be snoozed".into()));
        }
        // Its record wouldn't be kept: the mail would stay in Snoozed after a restart.
        if self.docs_unread.contains(SNOOZES_DOC) {
            return Err(CmdError::Failed("Snoozed mail couldn't be read, so no more can be snoozed".into()));
        }
        let next = self.successor(&self.selection.clone());
        let mut accounts: Vec<String> = sel.iter().map(|m| m.key.account.clone()).collect();
        accounts.sort();
        accounts.dedup();
        // A data file's import writes its account's lists: nothing is moved out of them.
        for a in &accounts {
            self.import_holds(a)?;
        }
        for a in accounts {
            let folder = self.snoozed_folder(&a)?;
            let mine: Vec<&Summary> = sel.iter().map(|m| &**m).filter(|m| m.key.account == a && m.key.folder != folder).collect();
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
            // One already snoozed stays where it is and comes back at the new time, to where it
            // was (said snoozed, it still came back at the old time).
            for m in sel.iter().filter(|m| m.key.account == a && m.key.folder == folder) {
                if let Some(s) = self.snoozes.iter_mut().find(|s| s.account == a && s.message_id == m.message_id) {
                    s.until = until;
                }
            }
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
        let sel = self.selected_shared();
        for m in &sel {
            self.import_holds(&m.key.account)?;
        }
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

    /// Bring back snoozed mail that is due (and what else waits for its time: the list's date
    /// headings on a new day). Called often (each frame, each CLI run): cheap when nothing is due.
    pub fn tick(&mut self) {
        self.imported_follow_up_at_start();
        self.let_go_some();
        self.open_indexes_ahead();
        self.make_texts_ahead();
        self.marks_kept();
        self.flush_due_outbox();
        self.undo_holds_due();
        self.read_when_due();
        self.web_calendars_due();
        self.free_busy_due();
        self.archive_waits_due();
        self.auto_archive_due();
        // The sender check follows contacts saved, accounts and the directory (cheap when not).
        self.keep_senders_when(false);
        self.held_invitations_due();
        // A new day moves the list's date headings (before the snoozes' early return).
        self.follow_the_day();
        let now = self.clock.now;
        if !self.snoozes.iter().any(|s| s.until <= now) {
            return;
        }
        // Those not found when last looked for (not synced here yet) are looked for again once
        // their folder changes, or a minute on, or with one newly due: each frame, each looked
        // through the whole Snoozed folder, for up to a week.
        let looked = &self.snoozes_looked;
        let newly_due = self.snoozes.iter().any(|s| s.until <= now && s.until > looked.at);
        // Held back by an import that's over now: at once, as the import ends.
        let let_go = looked.held && !self.snoozes.iter().any(|s| s.until <= now && self.importing.held(&s.account));
        if !newly_due && !looked.folder_changed && !let_go && now < looked.at.saturating_add(SNOOZE_LOOK_AGAIN) {
            return;
        }
        self.snoozes_looked = SnoozesLooked { at: now, folder_changed: false, held: false };
        #[cfg(test)]
        {
            self.snooze_looks += 1;
        }
        let due: Vec<Snooze> = self.snoozes.iter().filter(|s| s.until <= now).cloned().collect();
        let mut changed = false;
        for s in due {
            if self.account(&s.account).is_none() {
                self.snoozes.retain(|x| x != &s);
                changed = true;
                continue;
            }
            // Its account's import writes the folders: it comes back once that's done.
            if self.importing.held(&s.account) {
                self.snoozes_looked.held = true;
                continue;
            }
            // Its folder's list can't be read just now (its read marks being saved, its data file
            // being compacted): not looked through, so looked for again at the next tick (cheap
            // while it can't be read), not a minute on, and not given up on meanwhile.
            if self.load_cached(&s.account, &s.folder).is_err_and(|e| super::mailboxes::read_soon(&e)) {
                self.snoozes_looked.folder_changed = true;
                continue;
            }
            let found = self.folder_state(&s.account, &s.folder).messages.iter().find(|m| m.message_id == s.message_id).map(|m| m.key.clone());
            match found {
                Some(key) => {
                    self.snoozes.retain(|x| x != &s);
                    self.wake(&s, key);
                    if let Some(m) = self.list.iter().find(|m| m.message_id == s.message_id).map(|m| Summary::clone(m)) {
                        self.new_mail.push(m);
                    } else {
                        self.notice(format!("Back from snooze: {}", s.subject), false);
                    }
                    changed = true;
                }
                // Not synced into the Snoozed folder yet; give up after a week.
                None if now > s.until.saturating_add(7 * 86_400) => {
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

    /// Rules, Quick Steps and snoozes name a folder by its path: renamed or moved (deleted, it
    /// moves into Deleted Items), they follow it, as Favorites do. A rule went on naming the old
    /// path, so its mail stayed in the Inbox, and a snoozed message never came back.
    pub(super) fn organizing_follows_folder(&mut self, account: &str, from: &str, to: &str, delimiter: Option<char>) {
        // A path another account has too may be what a rule for every account means: left.
        let elsewhere = |s: &Self, path: &str| s.accounts.iter().any(|a| a.config.id != account && a.folders.iter().any(|f| f.path == path));
        let moved = |s: &Self, action: &RuleAction, all: bool| match action {
            RuleAction::MoveTo(f) if !(all && elsewhere(s, f)) => mailcraft_model::rebase_path(f, from, to, delimiter).map(RuleAction::MoveTo),
            _ => None,
        };
        let mut rules = self.rules.clone();
        let mut changed = false;
        for r in rules.iter_mut().filter(|r| r.account.as_deref().is_none_or(|a| a == account)) {
            for a in r.actions.iter_mut() {
                if let Some(new) = moved(self, a, r.account.is_none()) {
                    *a = new;
                    changed = true;
                }
            }
        }
        if changed {
            self.rules = rules;
            if let Some(store) = &self.store
                && let Err(e) = store.save_doc(RULES_DOC, &self.rules)
            {
                log::warn!("the rules couldn't be saved: {e}");
            }
        }
        let mut steps = self.prefs.quick_steps.clone();
        for a in steps.iter_mut().flat_map(|q| q.actions.iter_mut()) {
            if let Some(new) = moved(self, a, true) {
                *a = new;
            }
        }
        self.prefs.quick_steps = steps;
        let before = self.snoozes.clone();
        for s in self.snoozes.iter_mut().filter(|s| s.account == account) {
            for path in [&mut s.folder, &mut s.back_to] {
                if let Some(new) = mailcraft_model::rebase_path(path, from, to, delimiter) {
                    *path = new;
                }
            }
        }
        if self.snoozes != before {
            self.save_snoozes();
        }
    }

    pub(super) fn save_snoozes(&mut self) {
        // What couldn't be read isn't written over (the mail snoozed before wouldn't come back).
        if self.docs_unread.contains(SNOOZES_DOC) {
            return;
        }
        // An online-only account's snoozed mail is kept without its subject (issue #4).
        let kept: Vec<_> = self
            .snoozes
            .iter()
            .map(|s| match self.account(&s.account) {
                Some(a) if a.config.keeps_online() => mailcraft_model::organize::Snooze { subject: String::new(), ..s.clone() },
                _ => s.clone(),
            })
            .collect();
        if let Some(store) = &self.store
            && let Err(e) = store.save_doc(SNOOZES_DOC, &kept)
        {
            self.notice(format!("snoozed mail couldn't be saved: {e}"), true);
        }
    }

    // ----- rules -----

    pub fn set_rules(&mut self, rules: Vec<Rule>) -> CmdResult {
        self.check_rules(&rules)?;
        self.rules = rules;
        if let Some(store) = &self.store
            && let Err(e) = store.save_doc(RULES_DOC, &self.rules)
        {
            return Err(CmdError::Failed(format!("the rules couldn't be saved: {e}")));
        }
        Ok(serde_json::json!({ "rules": self.rules.len() }))
    }

    /// Whether `rules` can take the place of the rules here: at most 500, each named, each
    /// category they file mail under in the list, and the rules here read (not saved over).
    fn check_rules(&self, rules: &[Rule]) -> Result<(), CmdError> {
        if rules.len() > 500 {
            return Err(CmdError::Params("at most 500 rules".into()));
        }
        for r in rules {
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
        if self.docs_unread.contains(RULES_DOC) {
            return Err(CmdError::Failed("Your rules couldn't be read, so they aren't saved over".into()));
        }
        Ok(())
    }

    /// The rules as a file to keep or take to another computer (Outlook's Export Rules): each
    /// rule's account named by its address, which another computer knows it by.
    pub fn rules_file(&self) -> Result<Vec<u8>, CmdError> {
        let rules: Vec<serde_json::Value> = self
            .rules
            .iter()
            .map(|r| {
                let mut v = serde_json::to_value(r).unwrap_or_default();
                if let (Some(id), Some(o)) = (&r.account, v.as_object_mut()) {
                    let email = self.account(id).map(|a| a.config.email.clone()).unwrap_or_default();
                    o.insert("account".into(), serde_json::Value::Null);
                    o.insert("accountEmail".into(), serde_json::Value::String(email));
                }
                v
            })
            .collect();
        // The categories the rules file mail under, with their keywords: another computer
        // makes a missing one with the same keyword, so both mark mail the same way.
        let used: Vec<&mailcraft_model::organize::Category> = self
            .prefs
            .categories
            .iter()
            .filter(|c| self.rules.iter().flat_map(|r| &r.actions).any(|a| matches!(a, RuleAction::Categorize(n) if n.eq_ignore_ascii_case(&c.name))))
            .collect();
        let doc = serde_json::json!({ "mailcraftRules": 1, "rules": rules, "categories": used });
        serde_json::to_vec_pretty(&doc).map_err(|e| CmdError::Failed(e.to_string()))
    }

    /// Add the rules in a file made by [`Session::rules_file`] (Outlook's Import Rules) after
    /// those here, or in place of them (`replace`). A rule for an account this computer doesn't
    /// have comes in turned off; a category one files mail under is made when it's missing; a
    /// rule already here (the same in every part) isn't added twice. What came in, and how many
    /// were turned off.
    pub fn import_rules(&mut self, bytes: &[u8], replace: bool) -> CmdResult {
        if bytes.len() > 4 << 20 {
            return Err(CmdError::Failed("that file is too big for a rules file".into()));
        }
        // A file saved again by an editor may start with a byte order mark.
        let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
        let doc: serde_json::Value = serde_json::from_slice(bytes).map_err(|_| CmdError::Failed("not a MailCraft rules file".into()))?;
        let list = match (doc.get("mailcraftRules"), doc.get("rules").and_then(serde_json::Value::as_array)) {
            (Some(_), Some(list)) => list.clone(),
            _ => return Err(CmdError::Failed("not a MailCraft rules file".into())),
        };
        let known: Vec<mailcraft_model::organize::Category> =
            doc.get("categories").cloned().and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default();
        let mut rules = if replace { Vec::new() } else { self.rules.clone() };
        let (mut added, mut off) = (0usize, 0usize);
        // The categories the rules read file mail under (those already here among them too).
        let mut wanted: Vec<String> = Vec::new();
        for (i, v) in list.into_iter().take(500).enumerate() {
            let email = v.get("accountEmail").and_then(serde_json::Value::as_str).map(str::to_string);
            let mut rule: Rule =
                serde_json::from_value(v).map_err(|e| CmdError::Failed(format!("not a MailCraft rules file: rule {}: {e}", i + 1)))?;
            rule.account = None;
            if let Some(email) = email.filter(|e| !e.is_empty()) {
                match self.accounts.iter().find(|a| a.config.email.eq_ignore_ascii_case(&email)) {
                    Some(a) => rule.account = Some(a.config.id.clone()),
                    // Kept for that account only: off until it's chosen again.
                    None => {
                        rule.enabled = false;
                        off += 1;
                    }
                }
            }
            wanted.extend(rule.actions.iter().filter_map(|a| if let RuleAction::Categorize(c) = a { Some(c.clone()) } else { None }));
            if !rules.contains(&rule) {
                rules.push(rule);
                added += 1;
            }
        }
        // Every rule read: the categories they file mail under are made, and taken back when
        // the rules are refused all the same (too many, the rules here unread), so a failed
        // import leaves nothing behind. Made ones only go on the end of the list.
        let had = self.prefs.categories.len();
        let made = self.make_categories(&wanted, &known).and_then(|()| self.check_rules(&rules));
        if let Err(e) = made {
            self.prefs.categories.truncate(had);
            return Err(e);
        }
        self.set_rules(rules)?;
        Ok(serde_json::json!({ "added": added, "turnedOff": off, "rules": self.rules.len() }))
    }

    /// Make the categories called `names` that aren't in the list: with the keyword and colour
    /// they have where rules came from (`known`), when that keyword is free here.
    fn make_categories(&mut self, names: &[String], known: &[mailcraft_model::organize::Category]) -> Result<(), CmdError> {
        for c in names {
            if self.category(c).is_some() {
                continue;
            }
            let made = known.iter().find(|k| k.name.eq_ignore_ascii_case(c)).and_then(|k| self.add_category(c, Some(k.color), Some(&k.keyword)).ok());
            if made.is_none() {
                self.add_category(c, None, None)?;
            }
        }
        Ok(())
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
        let sel = self.selected_shared();
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
        let folders: Vec<(String, String)> = sel.iter().map(|m| (m.key.account.clone(), m.key.folder.clone())).collect();
        self.folders_ready(&folders)?;
        // Each folder is threaded once, however many of its messages are selected; its
        // conversations that hold a selected message are taken whole.
        let selected: HashSet<&MessageKey> = sel.iter().map(|m| &m.key).collect();
        let folders: BTreeSet<(String, String)> = folders.into_iter().collect();
        let (mut members, mut taken): (Vec<SharedSummary>, HashSet<MessageKey>) = (Vec::new(), HashSet::new());
        for (account, path) in folders {
            let folder = self.folder_state(&account, &path).messages.clone();
            for t in mailcraft_model::thread::threads(&folder) {
                let msgs: Vec<&SharedSummary> = t.messages.iter().filter_map(|&i| folder.get(i)).collect();
                if msgs.iter().any(|x| selected.contains(&x.key)) {
                    members.extend(msgs.into_iter().filter(|x| taken.insert(x.key.clone())).cloned());
                }
            }
        }
        members.extend(sel.iter().filter(|m| taken.insert(m.key.clone())).cloned());
        for m in &members {
            self.prefs.ignored.extend(Self::conversation_ids(m));
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
        let mut changes = Batch::default();
        for m in msgs {
            if !self.is_ignored(m) {
                continue;
            }
            self.prefs.ignored.extend(Self::conversation_ids(m));
            let a = &m.key.account;
            if let Some(trash) = self.account(a).and_then(|x| x.folder(FolderRole::Trash)).map(|f| f.path.clone())
                && trash != m.key.folder
            {
                changes.add(a, Op::Move { keys: vec![m.key.clone()], to: trash });
                gone.push(m.key.clone());
            }
        }
        self.apply_changes(changes);
        gone
    }

    /// Run the rules on messages: their actions become ordinary changes. Returns the keys the
    /// rules marked read or sent to Deleted Items or Junk (no need to announce those). A message
    /// in Sent has the rules for mail sent, any other the rest (issue #202).
    pub(super) fn run_rules(&mut self, msgs: &[Summary]) -> Vec<MessageKey> {
        if self.rules.is_empty() {
            return Vec::new();
        }
        let rules = self.rules.clone();
        self.apply_rules(&rules, msgs).0
    }

    /// [`Session::run_rules`] with these rules: the keys not to announce, and how many of `msgs`
    /// a rule matched.
    fn apply_rules<S: std::borrow::Borrow<Summary>>(&mut self, rules: &[Rule], msgs: &[S]) -> (Vec<MessageKey>, usize) {
        let (mut quiet, mut matched) = (Vec::new(), 0usize);
        let mut changes = Batch::default();
        let (sending, arriving): (Vec<Rule>, Vec<Rule>) = rules.iter().cloned().partition(|r| r.sent);
        for m in msgs {
            let m: &Summary = m.borrow();
            let sent = self.folder(&m.key.account, &m.key.folder).is_some_and(|f| f.role == FolderRole::Sent);
            let actions = actions_for(if sent { &sending } else { &arriving }, m);
            if actions.is_empty() {
                continue;
            }
            matched += 1;
            let a = m.key.account.clone();
            let keys = vec![m.key.clone()];
            let mut dest: Option<String> = None;
            for act in actions {
                match act {
                    RuleAction::MarkRead => {
                        changes.add(&a, Op::SetFlag { keys: keys.clone(), flag: Flag::Seen, on: true });
                        quiet.push(m.key.clone());
                    }
                    RuleAction::Flag => changes.add(&a, Op::SetFlag { keys: keys.clone(), flag: Flag::Flagged, on: true }),
                    RuleAction::Pin => changes.add(&a, Op::SetKeyword { keys: keys.clone(), keyword: PINNED.into(), on: true }),
                    RuleAction::Categorize(c) => {
                        if let Some(kw) = self.category(&c).map(|c| c.keyword.clone()) {
                            changes.add(&a, Op::SetKeyword { keys: keys.clone(), keyword: kw, on: true });
                        }
                    }
                    RuleAction::MoveTo(f) => dest = self.folder(&a, &f).map(|f| f.path.clone()).or(dest),
                    RuleAction::MoveToRole(r) => dest = self.account(&a).and_then(|x| x.folder(r)).map(|f| f.path.clone()).or(dest),
                    RuleAction::Delete => dest = self.account(&a).and_then(|x| x.folder(FolderRole::Trash)).map(|f| f.path.clone()).or(dest),
                }
            }
            // Moves last (`apply_changes`), so flags and categories are set on the message before
            // it leaves. Mail filed in a folder is still new mail (announced, as Outlook does);
            // mail a rule sends to Deleted Items or Junk isn't.
            if let Some(to) = dest.filter(|to| *to != m.key.folder) {
                let role = self.folder(&a, &to).map(|f| f.role);
                if matches!(role, Some(FolderRole::Trash | FolderRole::Junk)) {
                    quiet.push(m.key.clone());
                }
                changes.add(&a, Op::Move { keys, to });
            }
        }
        self.apply_changes(changes);
        (quiet, matched)
    }

    /// Run the rules (or one) now, on the list on screen or (`inbox`) on the mail already in the
    /// Inbox of the accounts they apply to (and in Sent, for the rules for mail sent), as
    /// Outlook's "run this rule now".
    pub fn run_rules_now(&mut self, only: Option<usize>, inbox: bool) -> CmdResult {
        // The one asked for runs even when it's turned off; the rules kept stay as they are.
        let rules = match only {
            Some(i) => vec![self.rules.get(i).cloned().map(|r| Rule { enabled: true, ..r }).ok_or_else(|| CmdError::Params(format!("no rule {i}")))?],
            None => self.rules.clone(),
        };
        let msgs: Vec<SharedSummary> = if inbox {
            let only_account = only.and(rules.first()).and_then(|r| r.account.clone());
            let roles: Vec<FolderRole> = [(FolderRole::Inbox, false), (FolderRole::Sent, true)]
                .into_iter()
                .filter(|(_, sent)| rules.iter().any(|r| r.sent == *sent))
                .map(|(role, _)| role)
                .collect();
            let inboxes: Vec<(String, String)> = self
                .accounts
                .iter()
                .filter(|a| only_account.as_ref().is_none_or(|o| *o == a.config.id))
                .flat_map(|a| roles.iter().filter_map(|r| a.folder(*r).map(|f| (a.config.id.clone(), f.path.clone()))))
                .collect();
            self.folders_ready(&inboxes)?;
            inboxes
                .iter()
                .flat_map(|(a, f)| self.folder_state(a, f).messages.iter().filter(|m| !m.flags.deleted).cloned().collect::<Vec<_>>())
                .collect()
        } else {
            self.list.clone()
        };
        let (_, n) = self.apply_rules(&rules, &msgs);
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
        !m.bulk || self.focus_book().and_then(|b| b.get(&sender)).is_some_and(|c| c.sent > 0)
    }

    /// Whether the list shows the Focused/Other split (an Inbox, with the option on).
    pub fn focus_split(&self) -> bool {
        self.prefs.focused_inbox && self.search.is_none() && self.view_role() == Some(FolderRole::Inbox)
    }

    /// Always put the selection's senders in Focused (or Other).
    pub fn set_focus_for_selection(&mut self, focused: bool) -> CmdResult {
        let sel = self.selected_shared();
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
        // Which pane its mail is in changed: a list kept from before a search is built again.
        self.mail_changed();
        self.refresh_list();
        Ok(serde_json::json!({ "focused": focused, "senders": sel.len() }))
    }
}

impl Session {
    // ----- clean up -----

    /// Clean Up, as Outlook's: messages of a conversation that a later reply quotes in full go to
    /// Deleted Items ([`mailcraft_model::cleanup`]). In these folders, every conversation, or
    /// (`only`) those holding these messages. Bodies not downloaded are left alone.
    pub fn clean_up(&mut self, folders: &[(String, String)], only: Option<&[MessageKey]>) -> CmdResult {
        let categories: Vec<String> = self.prefs.categories.iter().map(|c| c.keyword.clone()).collect();
        let mut gone: Vec<MessageKey> = Vec::new();
        let mut looked = 0usize;
        for (a, _) in folders {
            self.import_holds(a)?;
        }
        self.folders_ready(folders)?;
        for (a, f) in folders {
            // Deleted Items and Junk aren't cleaned up into Deleted Items.
            if self.folder(a, f).is_none_or(|x| matches!(x.role, FolderRole::Trash | FolderRole::Junk)) {
                continue;
            }
            let msgs: Vec<SharedSummary> = self.folder_state(a, f).messages.iter().filter(|m| !m.flags.deleted).cloned().collect();
            for t in mailcraft_model::thread::threads(&msgs) {
                let members: Vec<&Summary> = t.messages.iter().filter_map(|&i| msgs.get(i).map(|m| &**m)).collect();
                if members.len() < 2 || only.is_some_and(|keys| !members.iter().any(|m| keys.contains(&m.key))) {
                    continue;
                }
                looked += 1;
                // Only the sources whose text can change the answer are read and parsed: none in a
                // conversation where nothing may go (all unread, flagged or categorized).
                let wanted = mailcraft_model::cleanup::texts_wanted(&members, &categories);
                if !wanted.contains(&true) {
                    continue;
                }
                let texts: Vec<Option<String>> = members
                    .iter()
                    .zip(&wanted)
                    .map(|(m, &w)| {
                        w.then(|| self.store.as_ref().and_then(|s| s.read_body(&m.key)).and_then(|raw| mime::parse(&raw)).map(|p| p.text)).flatten()
                    })
                    .collect();
                gone.extend(
                    mailcraft_model::cleanup::redundant(&members, &texts, &categories)
                        .into_iter()
                        .filter_map(|i| members.get(i).map(|m| m.key.clone())),
                );
            }
        }
        if gone.is_empty() {
            let why = if looked == 0 { "No conversations to clean up" } else { "Nothing to clean up: no message is quoted in full by a later reply" };
            self.notice(why.to_string(), false);
            return Ok(serde_json::json!({ "moved": 0, "conversations": looked }));
        }
        // Through the selection, so it can be undone like any move.
        let n = gone.len();
        self.selection = gone;
        self.move_selection(MoveTarget::Role(FolderRole::Trash))?;
        let what = if n == 1 { "1 message".to_string() } else { format!("{n} messages") };
        self.notice(format!("Clean Up moved {what} to Deleted Items"), false);
        Ok(serde_json::json!({ "moved": n, "conversations": looked }))
    }
}
