//! The session: accounts' folder lists and what's kept of each folder.

use super::*;

mod nav;
mod rows;
pub(crate) use nav::ListNav;
pub use rows::{ARRANGE_HEADINGS, Arrange, Row, arrange_list};

impl Session {
    pub fn account(&self, id: &str) -> Option<&AccountState> {
        self.accounts.iter().find(|a| a.config.id == id)
    }

    pub(super) fn account_mut(&mut self, id: &str) -> Option<&mut AccountState> {
        self.accounts.iter_mut().find(|a| a.config.id == id)
    }

    pub fn folder(&self, account: &str, path: &str) -> Option<&Folder> {
        self.account(account).and_then(|a| a.folders.iter().find(|f| f.path == path))
    }

    /// What opens at the start: the first account's Inbox, as Outlook opens on its first mailbox
    /// (All Inboxes is a click away in Favorites; the accounts can be put in any order).
    pub(super) fn default_view(&self) -> View {
        match self.accounts.first() {
            Some(a) => {
                let folder = a.folder(FolderRole::Inbox).map(|f| f.path.clone()).unwrap_or_else(|| "INBOX".to_string());
                View::Folder { account: a.config.id.clone(), folder }
            }
            None => View::Empty,
        }
    }

    /// The account the view belongs to (or the first one, for unified views).
    pub fn current_account(&self) -> Option<&AccountState> {
        match &self.view {
            View::Folder { account, .. } => self.account(account),
            _ => self.accounts.first(),
        }
    }

    /// The role of the folder on screen.
    pub fn view_role(&self) -> Option<FolderRole> {
        match &self.view {
            View::Folder { account, folder } => self.folder(account, folder).map(|f| f.role),
            View::Unified { role } => Some(*role),
            View::Empty | View::Outbox => None,
        }
    }

    /// The view's title for the list header.
    pub fn view_title(&self) -> String {
        if let Some((q, _)) = &self.search {
            return format!("Results for “{q}”");
        }
        match &self.view {
            View::Folder { account, folder } => {
                self.folder(account, folder).map(Folder::display_name).unwrap_or_else(|| mailcraft_model::utf7::decode(folder))
            }
            View::Unified { role } => match role {
                FolderRole::Inbox => "All Inboxes".to_string(),
                r => format!("All {}", r.label().unwrap_or("folders")),
            },
            View::Outbox => "Outbox".to_string(),
            View::Empty => "MailCraft".to_string(),
        }
    }

    /// The cached state of a folder, loaded from disk on first use; empty while it can't be read.
    pub fn folder_state(&mut self, account: &str, folder: &str) -> &FolderState {
        if let Err(e) = self.load_cached(account, folder) {
            log::warn!("a folder's list couldn't be read: {e}");
        }
        static EMPTY: std::sync::OnceLock<FolderState> = std::sync::OnceLock::new();
        self.cache.get(&(account.to_string(), folder.to_string())).unwrap_or_else(|| EMPTY.get_or_init(FolderState::default))
    }

    /// Have a folder's list in the cache. One that can't be read is left out, not kept as empty:
    /// a flag changed in it then saved the empty list over the folder's mail.
    pub(super) fn load_cached(&mut self, account: &str, folder: &str) -> Result<(), mailcraft_store::StoreError> {
        let k = (account.to_string(), folder.to_string());
        if !self.cache.contains_key(&k) {
            let st = match &self.store {
                Some(s) => s.load_folder(account, folder)?,
                None => FolderState::default(),
            };
            self.cache.insert(k.clone(), st);
            self.cache_order.retain(|o| *o != k);
            self.cache_order.push(k.clone());
            self.trim_cache(&k);
        } else if self.cache_order.last() != Some(&k) {
            self.cache_order.retain(|o| *o != k);
            self.cache_order.push(k);
        }
        Ok(())
    }

    /// Keep the cached lists under `cache_limit` summaries: the least recently used folders
    /// go first (read again when wanted), never the ones on screen nor `keep`. Looking through a
    /// big mailbox (the address book, a search of everything) no longer leaves all of it in memory.
    pub(super) fn trim_cache(&mut self, keep: &(String, String)) {
        let mut total: usize = self.cache.values().map(|st| st.messages.len()).sum();
        if total <= self.cache_limit {
            return;
        }
        let shown = self.view_folders();
        let mut i = 0;
        while total > self.cache_limit && i < self.cache_order.len() {
            let k = self.cache_order.get(i).cloned().unwrap_or_default();
            if k == *keep || shown.contains(&k) {
                i += 1;
                continue;
            }
            if let Some(st) = self.cache.remove(&k) {
                total = total.saturating_sub(st.messages.len());
            }
            self.cache_order.remove(i);
        }
    }

    /// Change how many summaries the cache keeps, letting go of what's past it now.
    #[cfg(test)]
    pub(crate) fn set_cache_limit(&mut self, n: usize) {
        self.cache_limit = n;
        self.trim_cache(&(String::new(), String::new()));
    }

    pub(super) fn reload_folder(&mut self, account: &str, folder: &str) {
        let k = (account.to_string(), folder.to_string());
        self.cache.remove(&k);
        self.index_changed(account, folder);
        self.contacts = None;
    }

    /// Remember what an Inbox holds; unread mail above the last high-water mark is new. The first
    /// look at a folder only sets the mark, so a first sync doesn't announce the backlog.
    pub(super) fn note_new_mail(&mut self, account: &str, folder: &str) {
        let role = self.folder(account, folder).map(|f| f.role);
        if !matches!(role, Some(FolderRole::Inbox | FolderRole::Junk)) {
            return;
        }
        let st = self.folder_state(account, folder);
        let (validity, max) = (st.validity, st.messages.iter().map(|m| m.key.uid).max().unwrap_or(0));
        let k = (account.to_string(), folder.to_string());
        let arrived: Vec<Summary> = match self.seen_uid.get(&k) {
            Some(&seen) => {
                self.folder_state(account, folder).messages.iter().filter(|m| m.key.uid > seen && m.key.validity == validity).cloned().collect()
            }
            None => Vec::new(),
        };
        self.seen_uid.insert(k, max);
        // Junk from safe senders goes back to the Inbox (and is seen arriving there).
        if role == Some(FolderRole::Junk) {
            self.rescue_arrived(&arrived);
            return;
        }
        // Blocked senders' mail goes to Junk, replies in ignored conversations to Deleted Items,
        // then rules; what they file away or mark read isn't announced.
        let mut quiet = self.block_arrived(&arrived);
        let rest: Vec<Summary> = arrived.iter().filter(|m| !quiet.contains(&m.key)).cloned().collect();
        // Invitations into the calendar as tentative (not from blocked senders), before rules
        // move the mail (issue #20).
        self.invitations_arrived(&rest);
        quiet.extend(self.ignore_arrived(&rest));
        let rest: Vec<Summary> = arrived.iter().filter(|m| !quiet.contains(&m.key)).cloned().collect();
        quiet.extend(self.run_rules(&rest));
        // Mail older than a day (a resync after a long time away) isn't "new" any more.
        let cutoff = self.clock.now - 86_400;
        let fresh: Vec<Summary> = arrived.into_iter().filter(|m| !m.flags.seen && m.date >= cutoff && !quiet.contains(&m.key)).collect();
        if self.prefs.notify {
            self.new_mail.extend(fresh.into_iter().rev());
            // A burst (a first sync, a long time offline) is summed up by the UI; keep it bounded.
            let excess = self.new_mail.len().saturating_sub(50);
            self.new_mail.drain(..excess);
        }
    }

    /// Junk's high-water mark, from what's kept, as soon as the folder is known: what the server
    /// files there after is seen arriving (a safe sender's goes back to the Inbox), even when the
    /// folder was empty at the first sync and so never changed until then.
    pub(super) fn mark_junk(&mut self, account: &str) {
        let Some(junk) = self.account(account).and_then(|a| a.folder(FolderRole::Junk)).map(|f| f.path.clone()) else { return };
        if !self.seen_uid.contains_key(&(account.to_string(), junk.clone())) {
            self.note_new_mail(account, &junk);
        }
    }

    /// The Inbox's high-water mark, from what's kept, once the account's mail has been synced
    /// in full: an Inbox empty at the first sync never changed, so it had none, and its first
    /// message was taken for the backlog (not announced, no rules, no invitation taken in).
    pub(super) fn mark_inbox(&mut self, account: &str) {
        let Some(inbox) = self.account(account).and_then(|a| a.folder(FolderRole::Inbox)).map(|f| f.path.clone()) else { return };
        if !self.seen_uid.contains_key(&(account.to_string(), inbox.clone())) {
            self.note_new_mail(account, &inbox);
        }
    }

    /// New mail to announce since the last call.
    pub fn take_new_mail(&mut self) -> Vec<Summary> {
        std::mem::take(&mut self.new_mail)
    }

    /// Unread mail in every Inbox (for the window title and badges).
    pub fn inbox_unread(&self) -> u32 {
        self.accounts.iter().filter_map(|a| a.folder(FolderRole::Inbox)).map(|f| f.unread).sum()
    }

    /// People to suggest for a recipient box, best first: those mail was exchanged with and
    /// contacts, then the organisation directory's (issue #9), asked once the search is three
    /// letters long.
    pub fn suggest_contacts(&mut self, query: &str, limit: usize) -> Vec<Contact> {
        self.ensure_contacts();
        let mut out = self.contacts.as_ref().map(|b| b.suggest(query, limit, self.clock.now)).unwrap_or_default();
        if out.len() < limit {
            self.look_up_directory(query, false);
            for a in self.directory_people(query) {
                if let Some(c) = out.iter_mut().find(|c| c.address.email.eq_ignore_ascii_case(&a.email)) {
                    fill_name(&mut c.address, &a);
                } else if out.len() < limit {
                    out.push(Contact { address: a, sent: 0, received: 0, last: 0 });
                }
            }
        }
        out
    }

    /// Whether anything was written in the message being written (more than its signature):
    /// closing it then asks whether to keep it.
    pub fn compose_written(&self) -> bool {
        self.compose.as_ref().is_some_and(|c| self.written(c))
    }

    /// Whether anything was written in `c` (beyond its signature).
    pub fn written(&self, c: &Compose) -> bool {
        // Only a signature: the one put in, or (a message from before) the account's.
        let put = c.signature.trim();
        let signature = self.default_signature(&c.account, c.mode.is_some()).map(|s| signature_parts(&s).0).unwrap_or_default();
        let body = c.body.trim();
        !(c.to.trim().is_empty()
            && c.cc.trim().is_empty()
            && c.bcc.trim().is_empty()
            && c.subject.trim().is_empty()
            && c.attachments.is_empty()
            && (body.is_empty() || (!put.is_empty() && body == put) || body.trim_start_matches("--").trim() == signature.trim()))
    }

    /// The address book, as Outlook's Select Names dialog lists it: `people` for the people
    /// saved in Contacts, `directory` for the organisation's directory (Exchange's Global
    /// Address List, asked of the server: issue #9), anything else for everyone known (Contacts,
    /// everyone mail was exchanged with, and the directory's matches). Filtered by `query`
    /// (name or address), sorted by name.
    pub fn address_book(&mut self, source: &str, query: &str) -> Vec<EmailAddress> {
        const MAX: usize = 2000;
        if source == "directory" {
            self.look_up_directory(query, false);
            return self.directory_people(query);
        }
        let book = source.strip_prefix("book:").and_then(|b| b.split_once(':'));
        if source == "people" || book.is_some() {
            let mut v: Vec<EmailAddress> = self
                .people(query)
                .into_iter()
                .filter(|p| book.is_none_or(|(a, b)| p.account == a && p.book == b))
                .flat_map(|p| p.card.emails.iter().map(|e| EmailAddress::new(p.card.display(), e.value.clone())).collect::<Vec<_>>())
                .collect();
            v.truncate(MAX);
            return v;
        }
        self.ensure_contacts();
        let mut list: Vec<EmailAddress> =
            self.contacts.as_ref().map(|b| b.list(query, MAX).into_iter().map(|c| c.address).collect()).unwrap_or_default();
        self.look_up_directory(query, false);
        let mut more = Vec::new();
        for a in self.directory_people(query) {
            match list.iter_mut().find(|l| l.email.eq_ignore_ascii_case(&a.email)) {
                Some(l) => fill_name(l, &a),
                None => more.push(a),
            }
        }
        if !more.is_empty() {
            list.extend(more);
            list.sort_by_key(|a| (if a.name.is_empty() { a.email.to_lowercase() } else { a.name.to_lowercase() }, a.email.to_lowercase()));
            list.truncate(MAX);
        }
        list
    }

    /// The address books Select Names can show besides everyone (`all`) and every contact
    /// (`people`): each account's, as (source, name, the account's address).
    pub fn address_books(&self) -> Vec<(String, String, String)> {
        self.accounts
            .iter()
            .filter_map(|a| self.pim.get(&a.config.id).map(|p| (a, p)))
            .flat_map(|(a, p)| p.books.iter().map(move |b| (format!("book:{}:{}", a.config.id, b.id), b.name.clone(), a.config.email.clone())))
            .collect()
    }

    /// Build the address book from the cache if it isn't built.
    pub(super) fn ensure_contacts(&mut self) {
        if self.contacts.is_none() {
            let mine: Vec<String> = self.accounts.iter().flat_map(|a| [a.config.email.clone(), a.config.username.clone()]).collect();
            let folders: Vec<(String, String)> =
                self.accounts.iter().flat_map(|a| a.folders.iter().filter(|f| f.selectable).map(|f| (a.config.id.clone(), f.path.clone()))).collect();
            let mut book = ContactBook::new();
            for (a, f) in folders {
                for m in &self.folder_state(&a, &f).messages {
                    book.add(m, &mine);
                }
            }
            // People saved in an address book come first.
            for p in self.pim.values() {
                for c in &p.cards {
                    for e in &c.card.emails {
                        book.add_saved(&EmailAddress::new(c.card.display(), e.value.clone()), self.clock.now);
                    }
                }
            }
            self.contacts = Some(book);
            self.contacts_built = self.contacts_built.wrapping_add(1);
        }
    }

    /// The folders the view draws from.
    pub(super) fn view_folders(&self) -> Vec<(String, String)> {
        match &self.view {
            View::Folder { account, folder } => vec![(account.clone(), folder.clone())],
            View::Unified { role } => self.accounts.iter().filter_map(|a| a.folder(*role).map(|f| (a.config.id.clone(), f.path.clone()))).collect(),
            View::Empty | View::Outbox => Vec::new(),
        }
    }

    /// Rebuild the list from the view, search and filter, keeping the selection where possible.
    pub fn refresh_list(&mut self) {
        // What the list shows about senders (issue #29) is ready as it's drawn.
        self.keep_senders();
        let mut list: Vec<Summary> = if let Some((q, scope)) = self.search.clone() {
            let mut found = self.search_results(&q, scope);
            // What the servers found: its words may be in mail not downloaded here.
            let hits: Vec<MessageKey> = self.server_search.iter().filter(|ss| ss.query == q).flat_map(|ss| ss.found.iter().cloned()).collect();
            let have: HashSet<MessageKey> = found.iter().map(|m| m.key.clone()).collect();
            let hits: Vec<MessageKey> = hits.into_iter().filter(|k| !have.contains(k)).collect();
            // The servers can't tell dates or what's waiting for a reply: checked here.
            found.extend(self.server_hits(&q, &hits));
            // A Search Folder leaves out Deleted Items and Junk, as Outlook's.
            if self.open_search_folder_name().is_some() {
                found.retain(|m| self.folder(&m.key.account, &m.key.folder).is_none_or(|f| !matches!(f.role, FolderRole::Trash | FolderRole::Junk)));
            }
            sort_newest_first(&mut found);
            found
        } else if self.view == View::Outbox {
            self.outbox_summaries()
        } else {
            let mut v = Vec::new();
            for (a, f) in self.view_folders() {
                v.extend(self.folder_state(&a, &f).messages.iter().filter(|m| !m.flags.deleted).cloned());
            }
            v
        };
        let filter = self.filter;
        // The message being read stays where it was though it no longer passes (marked read
        // under Unread, say), as Outlook's: it goes once another is chosen (`show_message`).
        let reading = self.reading.as_ref().map(|r| r.key.clone()).filter(|k| self.list.iter().any(|m| &m.key == k));
        list.retain(|m| self.filter_keeps(filter, m) || reading.as_ref() == Some(&m.key));
        if self.focus_split() {
            self.ensure_contacts();
            let other = self.focus_other;
            list.retain(|m| self.is_focused(m) != other);
        }
        arrange_list(&mut list, self.prefs.arrange, self.prefs.reverse_sort);
        // Conversations are worked out once here when the list shows them (arranged by date);
        // arranged otherwise, the reading pane still shows the open message's, worked out once
        // with the first message opened (`thread_at`) rather than with every rebuild.
        let conversations = self.prefs.conversations;
        let shown = conversations && self.prefs.arrange == Arrange::Date;
        let all = if shown {
            self.nav.threaded();
            threads(&list)
        } else {
            Vec::new()
        };
        self.nav.build(&list, &all, conversations, !shown);
        self.threads = all;
        let alive: BTreeSet<&MessageKey> = list.iter().map(|m| &m.key).collect();
        self.selection.retain(|k| alive.contains(k));
        if self.focus.as_ref().is_some_and(|k| !alive.contains(k)) {
            self.focus = None;
        }
        if self.anchor.as_ref().is_some_and(|k| !alive.contains(k)) {
            self.anchor = None;
        }
        if let Some(r) = &self.reading
            && !alive.contains(&r.key)
            && self.search.is_none()
        {
            self.reading = None;
        }
        self.list = list;
        self.rebuild_rows();
        self.revision = self.revision.wrapping_add(1);
    }

    /// Whether a message passes a filter.
    pub(super) fn filter_keeps(&self, filter: ListFilter, m: &Summary) -> bool {
        match filter {
            ListFilter::All => true,
            ListFilter::Unread => !m.flags.seen,
            ListFilter::Flagged => m.flags.flagged,
            ListFilter::Attachments => m.has_attachments,
            ListFilter::Categorized => self.is_categorized(m),
            ListFilter::Important => m.importance > 0,
            ListFilter::ToMe => self.account(&m.key.account).is_some_and(|a| {
                let me = a.config.email.trim();
                !me.is_empty() && m.to.iter().chain(&m.cc).any(|r| r.email.trim().eq_ignore_ascii_case(me))
            }),
        }
    }

    /// The open message's conversation again (conversations turned on or off).
    pub(crate) fn rethread_reading(&mut self) {
        let Some(key) = self.reading.as_ref().map(|r| r.key.clone()) else { return };
        let thread = self.thread_of(&key);
        if let Some(r) = self.reading.as_mut() {
            r.thread = thread;
        }
    }
}

/// A known address with no real name (none, the address itself, or its user part, as some
/// directories and senders give: issue #9) takes the directory's full name for it.
fn fill_name(known: &mut EmailAddress, directory: &EmailAddress) {
    let name = known.name.trim();
    let user = known.email.split('@').next().unwrap_or("");
    let bare = name.is_empty() || name.eq_ignore_ascii_case(&known.email) || name.eq_ignore_ascii_case(user);
    if bare && !directory.name.trim().is_empty() {
        known.name = directory.name.clone();
    }
}
