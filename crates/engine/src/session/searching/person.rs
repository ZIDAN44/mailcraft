//! Everything exchanged with one person (issue #178): the mail between the user and them in
//! every account (by conversation), the files of it, the links in it, the meetings with them,
//! the people on that mail with them, and the details their signature gives. Gathered from what
//! is on this computer (the folders' lists, the bodies downloaded, the calendars), nothing asked
//! of a server; narrowed by a search in each part.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use mailcraft_model::FolderRole;
use mailcraft_model::pim::signature::{FoundDetails, signature_details};
use mailcraft_model::search::{Doc, Query, SummaryText, Tri, WholeText};
use mailcraft_model::summary::{MessageKey, SharedSummary};

use super::*;

/// The most messages gathered (the newest).
const MESSAGES_MAX: usize = 5_000;
/// The newest messages whose bodies are read for files, links and the signature.
const BODIES_MAX: usize = 300;
/// The most files, links, meetings and people kept.
const FILES_MAX: usize = 1_000;
const LINKS_MAX: usize = 1_000;
const MEETINGS_MAX: usize = 500;
const PEOPLE_MAX: usize = 200;
/// The newest messages of theirs looked at for a signature.
const SIGNATURES_LOOKED: usize = 10;
/// The most characters of each message's text kept for the search within it.
const TEXT_KEPT: usize = 32_000;

/// A conversation with them: its messages, the newest first.
#[derive(Clone, Debug)]
pub struct PersonThread {
    pub subject: String,
    pub latest: i64,
    pub messages: Vec<SharedSummary>,
}

/// A file of a message between the user and them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PersonFile {
    pub name: String,
    pub size: usize,
    pub key: MessageKey,
    /// Its place among the message's attachments.
    pub index: usize,
    pub date: i64,
    /// They sent it (else the user, or someone else on the mail).
    pub from_them: bool,
}

/// A web address in a message between the user and them (the newest it's in).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PersonLink {
    pub url: String,
    pub key: MessageKey,
    pub date: i64,
}

/// A meeting they organise or are a guest of.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PersonMeeting {
    pub account: String,
    pub uid: String,
    pub summary: String,
    pub start: i64,
    pub end: i64,
    pub all_day: bool,
    /// They organise it.
    pub theirs: bool,
}

/// Someone else on the mail between the user and them: how often.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PersonPeer {
    pub name: String,
    pub email: String,
    pub count: usize,
}

/// Everything exchanged with one person ([`Session::person_activity`]).
#[derive(Clone, Debug, Default)]
pub struct PersonActivity {
    /// The address asked about, lower-case.
    pub email: String,
    /// Their name: their contact's, else the newest one their mail gives.
    pub name: String,
    /// Their addresses: their contact's, else the one asked about.
    pub addresses: Vec<String>,
    /// Their contact (account, uid), when they have one.
    pub contact: Option<(String, String)>,
    pub threads: Vec<PersonThread>,
    pub files: Vec<PersonFile>,
    pub links: Vec<PersonLink>,
    pub meetings: Vec<PersonMeeting>,
    pub people: Vec<PersonPeer>,
    /// What their signature gives that their contact doesn't have.
    pub found: FoundDetails,
    /// Messages whose bodies aren't on this computer, among those read for files and links.
    pub not_downloaded: usize,
    /// More messages than are gathered: the oldest left out.
    pub cut_short: bool,
    /// The text of the messages read (with their files' names), for the search within them.
    pub texts: Arc<HashMap<MessageKey, String>>,
}

impl PersonActivity {
    /// How many messages it holds.
    pub fn message_count(&self) -> usize {
        self.threads.iter().map(|t| t.messages.len()).sum()
    }

    /// What a search narrows it to (issue #178: refined within the results): the mail by the
    /// search's own terms (`subject:`, `from:`, `received:`, words), a word its summary doesn't
    /// settle looked for in its text where that was read; the rest by words in their names,
    /// addresses or titles.
    pub fn refined(&self, query: &str) -> PersonActivity {
        let query = query.trim();
        if query.is_empty() {
            return self.clone();
        }
        let q = Query::parse(query);
        let words: Vec<String> =
            mailcraft_model::search::fold_all(query).split(' ').filter(|w| !w.is_empty() && !w.contains(':')).map(str::to_string).collect();
        let has = |text: &str| {
            let t = mailcraft_model::search::fold_all(text);
            words.iter().all(|w| t.contains(w.as_str()))
        };
        let mail = |m: &SharedSummary| {
            let st = SummaryText::of(m);
            match q.eval(&Doc { summary: m, summary_text: &st, awaiting: false, files: None, text: &matching::Unindexed }) {
                Tri::Unknown => self
                    .texts
                    .get(&m.key)
                    .is_some_and(|text| q.matches(&Doc { summary: m, summary_text: &st, awaiting: false, files: None, text: &WholeText::new(text) })),
                found => found == Tri::Yes,
            }
        };
        let mut out = PersonActivity { threads: Vec::new(), ..self.clone() };
        for t in &self.threads {
            let messages: Vec<SharedSummary> = t.messages.iter().filter(|m| mail(m)).cloned().collect();
            if !messages.is_empty() {
                out.threads.push(PersonThread { subject: t.subject.clone(), latest: t.latest, messages });
            }
        }
        out.files.retain(|f| has(&f.name));
        out.links.retain(|l| has(&l.url));
        out.meetings.retain(|m| has(&m.summary));
        out.people.retain(|p| has(&format!("{} {}", p.name, p.email)));
        out
    }
}

impl Session {
    /// Everything exchanged with the person at `email` (issue #178): the mail between the user
    /// and them in every account (sent or received, in any folder but Junk and Deleted Items,
    /// once each), by conversation, the newest first; the files and links of the newest of it
    /// that are on this computer; the meetings they organise or are asked to; the others on
    /// that mail; and what their signature gives.
    pub fn person_activity(&mut self, email: &str) -> PersonActivity {
        self.person_activity_in(email, &|_| true)
    }

    /// [`Session::person_activity`] from the accounts `keep` says alone: their mail and meetings,
    /// the rest as if there were no others (AI agents', with accounts kept from them).
    pub(crate) fn person_activity_in(&mut self, email: &str, keep: &dyn Fn(&str) -> bool) -> PersonActivity {
        let email = email.trim().to_lowercase();
        let mut out = PersonActivity { email: email.clone(), ..Default::default() };
        if !email.contains('@') {
            return out;
        }
        // Their contact: their name and their other addresses.
        let contact = self.contact_by_address(&email).cloned();
        let mut theirs: Vec<String> =
            contact.iter().flat_map(|c| c.emails.iter().map(|e| e.value.trim().to_lowercase())).filter(|e| e.contains('@')).collect();
        if !theirs.contains(&email) {
            theirs.insert(0, email.clone());
        }
        out.contact = contact
            .as_ref()
            .and_then(|c| self.pim.iter().find(|(_, p)| p.cards.iter().any(|x| x.card.uid == c.uid)).map(|(a, _)| (a.clone(), c.uid.clone())));
        let theirs_set: HashSet<&str> = theirs.iter().map(String::as_str).collect();
        let mine: HashSet<String> = self
            .accounts
            .iter()
            .flat_map(|a| a.config.addresses().chain([a.config.username.as_str()]))
            .map(|e| e.trim().to_lowercase())
            .filter(|e| e.contains('@'))
            .collect();
        let is_them = |e: &str| theirs_set.contains(e.trim().to_lowercase().as_str());

        // Their mail: Inbox and Sent first, so a message in several folders (Gmail's labels) is
        // kept where it's met in them.
        let mut folders: Vec<(String, String, u8)> = self
            .accounts
            .iter()
            .filter(|a| keep(&a.config.id))
            .flat_map(|a| {
                a.folders.iter().filter(|f| f.selectable && !matches!(f.role, FolderRole::Junk | FolderRole::Trash)).map(|f| {
                    let rank = match f.role {
                        FolderRole::Inbox => 0,
                        FolderRole::Sent => 1,
                        _ => 2,
                    };
                    (a.config.id.clone(), f.path.clone(), rank)
                })
            })
            .collect();
        folders.sort_by_key(|f| f.2);
        let mut seen_ids: HashSet<String> = HashSet::new();
        let mut mail: Vec<SharedSummary> = Vec::new();
        for (account, folder, _) in folders {
            let list: Vec<SharedSummary> = self
                .folder_state(&account, &folder)
                .messages
                .iter()
                .filter(|m| is_them(&m.from.email) || m.to.iter().chain(&m.cc).any(|r| is_them(&r.email)))
                .cloned()
                .collect();
            for m in list {
                if m.message_id.is_empty() || seen_ids.insert(m.message_id.clone()) {
                    mail.push(m);
                }
            }
        }
        mail.sort_by_key(|m| std::cmp::Reverse(m.when()));
        if mail.len() > MESSAGES_MAX {
            mail.truncate(MESSAGES_MAX);
            out.cut_short = true;
        }

        // Their name: their contact's, else the newest their mail gives.
        out.name = contact.as_ref().map(|c| c.display()).filter(|n| !n.trim().is_empty()).unwrap_or_else(|| {
            mail.iter()
                .flat_map(|m| std::iter::once(&m.from).chain(&m.to).chain(&m.cc))
                .find(|a| is_them(&a.email) && !a.name.trim().is_empty())
                .map(|a| a.name.trim().to_string())
                .unwrap_or_else(|| email.clone())
        });
        out.addresses = theirs.clone();

        // By conversation, the newest first.
        for t in mailcraft_model::thread::threads(&mail) {
            let mut messages: Vec<SharedSummary> = t.messages.iter().filter_map(|i| mail.get(*i)).cloned().collect();
            messages.sort_by_key(|m| std::cmp::Reverse(m.when()));
            let subject = messages.first().map(|m| mailcraft_model::thread::base_subject(&m.subject).to_string()).unwrap_or_default();
            out.threads.push(PersonThread { subject, latest: messages.first().map_or(t.latest, |m| m.when()), messages });
        }
        out.threads.sort_by_key(|t| std::cmp::Reverse(t.latest));

        // The others on it.
        let mut peers: HashMap<String, PersonPeer> = HashMap::new();
        for m in &mail {
            for a in std::iter::once(&m.from).chain(&m.to).chain(&m.cc) {
                let e = a.email.trim().to_lowercase();
                if !e.contains('@') || is_them(&e) || mine.contains(&e) {
                    continue;
                }
                let p = peers.entry(e.clone()).or_insert_with(|| PersonPeer { name: String::new(), email: e, count: 0 });
                p.count += 1;
                if p.name.is_empty() && !a.name.trim().is_empty() {
                    p.name = a.name.trim().to_string();
                }
            }
        }
        out.people = peers.into_values().collect();
        out.people.sort_by(|a, b| {
            b.count.cmp(&a.count).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())).then_with(|| a.email.cmp(&b.email))
        });
        out.people.truncate(PEOPLE_MAX);

        // The files and links of the newest messages on this computer, their text, and their
        // signature.
        let mut texts: HashMap<MessageKey, String> = HashMap::new();
        let mut links: HashSet<String> = HashSet::new();
        let mut signatures = 0;
        for m in mail.iter().take(BODIES_MAX) {
            let Some(raw) = self.store.as_ref().and_then(|s| s.read_body(&m.key)) else {
                out.not_downloaded += 1;
                continue;
            };
            let Some(parsed) = mailcraft_mime::parse(&raw) else { continue };
            let from_them = is_them(&m.from.email);
            for a in parsed.attachments.iter().filter(|a| !a.inline) {
                if out.files.len() < FILES_MAX {
                    out.files.push(PersonFile { name: a.name.clone(), size: a.size, key: m.key.clone(), index: a.index, date: m.when(), from_them });
                }
            }
            for url in mailcraft_model::pim::ical::links_in(&parsed.text) {
                if out.links.len() < LINKS_MAX && links.insert(url.to_lowercase()) {
                    out.links.push(PersonLink { url, key: m.key.clone(), date: m.when() });
                }
            }
            if from_them && out.found.is_empty() && signatures < SIGNATURES_LOOKED {
                signatures += 1;
                out.found = signature_details(&parsed.text, &out.name);
            }
            let mut text: String = match parsed.text.char_indices().nth(TEXT_KEPT) {
                Some((i, _)) => parsed.text.get(..i).unwrap_or_default().to_string(),
                None => parsed.text.clone(),
            };
            for a in &parsed.attachments {
                text.push('\n');
                text.push_str(&a.name);
            }
            texts.insert(m.key.clone(), text);
        }
        out.texts = Arc::new(texts);
        // What the contact has already isn't offered again.
        if let Some(c) = &contact {
            let digits = |s: &str| s.chars().filter(char::is_ascii_digit).collect::<String>();
            let known: HashSet<String> = c.phones.iter().map(|p| digits(&p.value)).collect();
            out.found.phones.retain(|p| !known.contains(&digits(p)));
            // Its web addresses are among the lines kept as written (`URL;TYPE=work:https://…`).
            let urls: Vec<String> = c
                .extra
                .iter()
                .filter(|l| l.get(..3).is_some_and(|n| n.eq_ignore_ascii_case("url")))
                .map(|l| l.split_once(':').map_or("", |(_, v)| v).trim().to_lowercase())
                .filter(|u| !u.is_empty())
                .collect();
            out.found.websites.retain(|w| !urls.iter().any(|u| u.contains(&w.to_lowercase()) || w.to_lowercase().contains(u.as_str())));
        }

        // Meetings they organise or are asked to, in every account, once each.
        let mut uids: HashSet<String> = HashSet::new();
        for (account, p) in self.pim.iter().filter(|(a, _)| keep(a)) {
            for e in &p.events {
                let ev = &e.event;
                let organises = ev.organizer.as_ref().is_some_and(|o| is_them(&o.email));
                if !organises && !ev.attendees.iter().any(|a| is_them(&a.email)) {
                    continue;
                }
                if ev.recurrence_id.is_none() && uids.insert(ev.uid.clone()) {
                    out.meetings.push(PersonMeeting {
                        account: account.clone(),
                        uid: ev.uid.clone(),
                        summary: ev.summary.clone(),
                        start: ev.start,
                        end: ev.end,
                        all_day: ev.all_day,
                        theirs: organises,
                    });
                }
            }
        }
        out.meetings.sort_by_key(|m| std::cmp::Reverse(m.start));
        out.meetings.truncate(MEETINGS_MAX);
        out
    }
}
