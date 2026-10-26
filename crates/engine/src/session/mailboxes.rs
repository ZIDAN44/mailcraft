//! The session: accounts' folder lists and what's kept of each folder.

use super::*;

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

    /// The cached state of a folder, loaded from disk on first use.
    pub fn folder_state(&mut self, account: &str, folder: &str) -> &FolderState {
        let k = (account.to_string(), folder.to_string());
        if !self.cache.contains_key(&k) {
            let st = self.store.as_ref().and_then(|s| s.load_folder(account, folder).ok()).unwrap_or_default();
            self.cache.insert(k.clone(), st);
            self.cache_order.retain(|o| *o != k);
            self.cache_order.push(k.clone());
            self.trim_cache(&k);
        } else if self.cache_order.last() != Some(&k) {
            self.cache_order.retain(|o| *o != k);
            self.cache_order.push(k.clone());
        }
        static EMPTY: std::sync::OnceLock<FolderState> = std::sync::OnceLock::new();
        self.cache.get(&k).unwrap_or_else(|| EMPTY.get_or_init(FolderState::default))
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
        self.index = None;
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
                if out.len() >= limit {
                    break;
                }
                if !out.iter().any(|c| c.address.email.eq_ignore_ascii_case(&a.email)) {
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
        let signature = self.signature(&c.account).trim().to_string();
        let body = c.body.trim();
        !(c.to.trim().is_empty()
            && c.cc.trim().is_empty()
            && c.bcc.trim().is_empty()
            && c.subject.trim().is_empty()
            && c.attachments.is_empty()
            && (body.is_empty() || body.trim_start_matches("--").trim() == signature))
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
        let more: Vec<EmailAddress> =
            self.directory_people(query).into_iter().filter(|a| !list.iter().any(|l| l.email.eq_ignore_ascii_case(&a.email))).collect();
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
        let mut list: Vec<Summary> = if let Some((q, scope)) = self.search.clone() {
            let mut found = self.search_results(&q, scope);
            // What the servers found: its words may be in mail not downloaded here.
            let hits: Vec<MessageKey> = self.server_search.iter().filter(|ss| ss.query == q).flat_map(|ss| ss.found.iter().cloned()).collect();
            let have: HashSet<MessageKey> = found.iter().map(|m| m.key.clone()).collect();
            // The servers can't tell what's waiting for a reply: checked here.
            let awaiting = super::searching::awaiting_wanted(&q).map(|on| (on, self.awaiting_replies()));
            let hits = hits.into_iter().filter(|k| awaiting.as_ref().is_none_or(|(on, set)| set.contains(k) == *on));
            for key in hits.filter(|k| !have.contains(k)) {
                if let Some(m) = self.folder_state(&key.account, &key.folder).messages.iter().find(|m| m.key.uid == key.uid).cloned() {
                    found.push(m);
                }
            }
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
        list.retain(|m| self.filter_keeps(filter, m));
        if self.focus_split() {
            self.ensure_contacts();
            let other = self.focus_other;
            list.retain(|m| self.is_focused(m) != other);
        }
        arrange_list(&mut list, self.prefs.arrange, self.prefs.reverse_sort);
        self.threads = if self.prefs.conversations && self.prefs.arrange == Arrange::Date { threads(&list) } else { Vec::new() };
        let alive: BTreeSet<&MessageKey> = list.iter().map(|m| &m.key).collect();
        self.selection.retain(|k| alive.contains(k));
        if self.focus.as_ref().is_some_and(|k| !alive.contains(k)) {
            self.focus = None;
        }
        if let Some(r) = &self.reading
            && !alive.contains(&r.key)
            && self.search.is_none()
        {
            self.reading = None;
        }
        self.list = list;
        self.revision = self.revision.wrapping_add(1);
    }

    /// Whether a message passes a filter.
    pub(super) fn filter_keeps(&self, filter: ListFilter, m: &Summary) -> bool {
        match filter {
            ListFilter::All => true,
            ListFilter::Unread => !m.flags.seen,
            ListFilter::Flagged => m.flags.flagged,
            ListFilter::Attachments => m.has_attachments,
            ListFilter::Categorized => self.prefs.categories.iter().any(|c| m.flags.has(&c.keyword)),
            ListFilter::Important => m.importance > 0,
            ListFilter::ToMe => self.account(&m.key.account).is_some_and(|a| {
                let me = a.config.email.trim();
                !me.is_empty() && m.to.iter().chain(&m.cc).any(|r| r.email.trim().eq_ignore_ascii_case(me))
            }),
        }
    }

    /// The list as rows with date headings (and conversations, when on).
    pub fn rows(&self) -> Vec<Row> {
        let mut out = Vec::with_capacity(self.list.len() + 8);
        let mut last: Option<Row> = None;
        let arrange = self.prefs.arrange;
        let mut push = |out: &mut Vec<Row>, index: usize, count: usize, unread: usize, date: i64| {
            // The heading this message goes under, as the list is arranged.
            let m = self.list.get(index);
            let heading = match arrange {
                Arrange::Date => Row::Header(DateGroup::of(date, &self.clock).label()),
                Arrange::From => Row::Group(m.map(|m| person(Some(&m.from))).unwrap_or_default()),
                Arrange::To => Row::Group(m.map(|m| person(m.to.first())).unwrap_or_default()),
                Arrange::Subject => Row::Group(m.map(Summary::base_subject).unwrap_or_default()),
                Arrange::Size => Row::Header(size_group(m.map_or(0, |m| m.size))),
                Arrange::Flag => Row::Header(if m.is_some_and(|m| m.flags.flagged) { "Flagged" } else { "Not flagged" }),
                Arrange::Attachments => Row::Header(if m.is_some_and(|m| m.has_attachments) { "With attachments" } else { "Without attachments" }),
            };
            let heading = match heading {
                Row::Group(g) if g.trim().is_empty() => Row::Header(if arrange == Arrange::Subject { "(no subject)" } else { "(nobody)" }),
                h => h,
            };
            if last.as_ref() != Some(&heading) {
                out.push(heading.clone());
                last = Some(heading);
            }
            out.push(Row::Message { index, count, unread });
        };
        // Pinned mail first, under its own heading (searches keep plain date order).
        let pins = self.search.is_none();
        let pinned = |i: usize| pins && self.list.get(i).is_some_and(|m| m.flags.pinned());
        let mut pinned_rows = Vec::new();
        if self.prefs.conversations && !self.threads.is_empty() {
            for t in &self.threads {
                let Some(&newest) = t.messages.iter().max_by_key(|&&i| self.list.get(i).map_or(0, |m| m.date)) else { continue };
                let unread = t.messages.iter().filter(|&&i| self.list.get(i).is_some_and(|m| !m.flags.seen)).count();
                if t.messages.iter().any(|&i| pinned(i)) {
                    pinned_rows.push(Row::Message { index: newest, count: t.messages.len(), unread });
                } else {
                    push(&mut out, newest, t.messages.len(), unread, t.latest);
                }
            }
        } else {
            for (i, m) in self.list.iter().enumerate() {
                if pinned(i) {
                    pinned_rows.push(Row::Message { index: i, count: 1, unread: usize::from(!m.flags.seen) });
                } else {
                    push(&mut out, i, 1, usize::from(!m.flags.seen), m.date);
                }
            }
        }
        if !pinned_rows.is_empty() {
            pinned_rows.insert(0, Row::Header("Pinned"));
            pinned_rows.extend(out);
            return pinned_rows;
        }
        out
    }

    pub fn summary(&self, key: &MessageKey) -> Option<&Summary> {
        self.list.iter().find(|m| &m.key == key)
    }

    /// The selected messages (in list order).
    pub fn selected(&self) -> Vec<Summary> {
        if self.selection.len() < 8 {
            return self.list.iter().filter(|m| self.selection.contains(&m.key)).cloned().collect();
        }
        let chosen: HashSet<&MessageKey> = self.selection.iter().collect();
        self.list.iter().filter(|m| chosen.contains(&m.key)).cloned().collect()
    }

    /// The conversation a message belongs to in the current list.
    /// The open message's conversation again (conversations turned on or off).
    pub(crate) fn rethread_reading(&mut self) {
        let Some(key) = self.reading.as_ref().map(|r| r.key.clone()) else { return };
        let thread = self.thread_of(&key);
        if let Some(r) = self.reading.as_mut() {
            r.thread = thread;
        }
    }

    /// The conversation a message is in, shown above it in the reading pane: only while the list
    /// is grouped by conversation (as Outlook's reading pane shows one message otherwise).
    pub(super) fn thread_of(&self, key: &MessageKey) -> Vec<MessageKey> {
        if !self.prefs.conversations {
            return Vec::new();
        }
        let Some(i) = self.list.iter().position(|m| &m.key == key) else { return Vec::new() };
        let ts;
        let threads = if self.threads.is_empty() {
            ts = threads(&self.list);
            &ts
        } else {
            &self.threads
        };
        threads
            .iter()
            .find(|t| t.messages.contains(&i))
            .map(|t| t.messages.iter().filter_map(|&j| self.list.get(j).map(|m| m.key.clone())).collect())
            .unwrap_or_default()
    }
}
