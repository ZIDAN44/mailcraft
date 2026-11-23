//! The organisation's directory (Exchange's Global Address List, issue #9): looked up on the
//! server as names are searched for, so someone added there is found without a restart. What
//! the servers answer is kept briefly, to spare them a request per frame: a list of people for
//! [`FOUND_FOR`] seconds, "no one" and failures for [`NONE_FOR`] (someone new is found soon after
//! the directory knows them). Refresh forgets it all and reads the address books again.

use super::*;

/// How long a list of people found stays fresh, in seconds.
pub const FOUND_FOR: i64 = 300;
/// How long "no one found" (or a failed search) stays fresh, in seconds.
pub const NONE_FOR: i64 = 30;
/// The shortest search sent to the servers by itself (a shorter one goes when asked).
pub const MIN_QUERY: usize = 3;
/// Searches remembered at most.
const MAX_KEPT: usize = 200;
/// People's job titles and departments kept at most (issue #213).
const DETAILS_KEPT: usize = 5_000;

/// What one account's directory answered for one search.
#[derive(Clone, Debug, Default)]
pub struct Answer {
    pub people: Vec<EmailAddress>,
    pub error: Option<String>,
    /// When it answered (`None` while it's being asked).
    pub at: Option<i64>,
}

/// The directory searches asked and answered.
#[derive(Clone, Debug, Default)]
pub struct Directory {
    /// (account, search in lower case) → its answer.
    pub answers: HashMap<(String, String), Answer>,
    /// Bumped on every answer and refresh, for what shows the people found.
    pub rev: u64,
    /// Exchange Online accounts whose directory needs the user's permission first (issue #9).
    pub consent: BTreeSet<String>,
    /// The job title and department the directory gave each address (lower case) it found
    /// (issue #213), for the Address Book's columns.
    pub details: HashMap<String, (String, String)>,
}

/// A search as it's kept and sent: trimmed, lower case.
fn key(query: &str) -> String {
    query.trim().to_lowercase()
}

impl Session {
    /// The accounts with an organisation directory: Exchange, on premises (EWS) or online
    /// (Graph, issue #9). A personal Microsoft account (Outlook.com, Hotmail) has none.
    pub fn directory_accounts(&self) -> Vec<String> {
        self.accounts
            .iter()
            .filter(|a| {
                let c = &a.config;
                c.ews.is_some() || (c.graph.is_some() && !mailcraft_model::microsoft_consumer(c.oauth_user.as_deref().unwrap_or(&c.email)))
            })
            .map(|a| a.config.id.clone())
            .collect()
    }

    /// Ask the directories for `query` where their last answer isn't fresh; `force` asks again
    /// whatever they answered, and asks for a search under [`MIN_QUERY`] letters too.
    pub fn look_up_directory(&mut self, query: &str, force: bool) {
        self.ask_directory(query, force, force);
    }

    /// Ask `account`'s own directory about `email` (issue #245: the person themselves, for the
    /// details their organisation's signature names), unless it's being asked or answered:
    /// asked again only after a failure.
    pub(crate) fn ask_directory_once(&mut self, account: &str, email: &str) {
        let k = (account.to_string(), key(email));
        if k.1.is_empty() || self.directory.answers.get(&k).is_some_and(|a| a.error.is_none()) {
            return;
        }
        self.directory.answers.insert(k.clone(), Answer::default());
        self.jobs.push(Job::SearchDirectory { account: k.0, query: k.1 });
    }

    /// Ask the directories for `query`: `short`, under [`MIN_QUERY`] letters too (a name Check
    /// Names is given, issue #211); `again`, whatever they answered last.
    pub(crate) fn ask_directory(&mut self, query: &str, short: bool, again: bool) {
        let q = key(query);
        if q.is_empty() || (!short && q.chars().count() < MIN_QUERY) {
            return;
        }
        let now = self.clock.now;
        for account in self.directory_accounts() {
            let k = (account.clone(), q.clone());
            let fresh = self.directory.answers.get(&k).is_some_and(|a| match a.at {
                None => true,
                Some(_) if again => false,
                // A failure keeps who was found before, but isn't their answer.
                Some(at) => now - at < if a.error.is_some() || a.people.is_empty() { NONE_FOR } else { FOUND_FOR },
            });
            if fresh {
                continue;
            }
            if self.directory.answers.len() >= MAX_KEPT {
                // The oldest answered go first; one being asked stays.
                let mut done: Vec<((String, String), i64)> =
                    self.directory.answers.iter().filter_map(|(k, a)| a.at.map(|at| (k.clone(), at))).collect();
                done.sort_by_key(|(_, at)| *at);
                for (k, _) in done.into_iter().take(MAX_KEPT / 4) {
                    self.directory.answers.remove(&k);
                }
            }
            let keep = self.directory.answers.remove(&k).map(|a| a.people).unwrap_or_default();
            self.directory.answers.insert(k, Answer { people: keep, error: None, at: None });
            self.jobs.push(Job::SearchDirectory { account, query: q.clone() });
        }
    }

    /// Everyone the directories found for `query`, by name. While a search is being asked, the
    /// longest earlier search it starts with stands in, narrowed to it.
    pub fn directory_people(&self, query: &str) -> Vec<EmailAddress> {
        let q = key(query);
        if q.is_empty() {
            return Vec::new();
        }
        let mut out: Vec<EmailAddress> = Vec::new();
        for account in self.directory_accounts() {
            let exact = self.directory.answers.get(&(account.clone(), q.clone())).filter(|a| a.at.is_some() || !a.people.is_empty());
            let people: Vec<EmailAddress> = match exact {
                Some(a) => a.people.clone(),
                None => self
                    .directory
                    .answers
                    .iter()
                    .filter(|((acc, s), a)| *acc == account && a.at.is_some() && q.starts_with(s.as_str()))
                    .max_by_key(|((_, s), _)| s.len())
                    .map(|(_, a)| a.people.iter().filter(|p| matches_query(p, &q)).cloned().collect())
                    .unwrap_or_default(),
            };
            for p in people {
                if !out.iter().any(|o| o.email.eq_ignore_ascii_case(&p.email)) {
                    out.push(p);
                }
            }
        }
        out.sort_by_key(|a| (if a.name.is_empty() { a.email.to_lowercase() } else { a.name.to_lowercase() }, a.email.to_lowercase()));
        out
    }

    /// Whether a directory is still being asked about `query`.
    pub fn directory_pending(&self, query: &str) -> bool {
        let q = key(query);
        self.directory.answers.iter().any(|((_, s), a)| *s == q && a.at.is_none())
    }

    /// Why a directory couldn't look `query` up (each account's reason), if one couldn't.
    pub fn directory_errors(&self, query: &str) -> Vec<String> {
        let q = key(query);
        let mut out: Vec<String> = self
            .directory
            .answers
            .iter()
            // One waiting for permission is asked for it instead ([`Session::allow_directory`]).
            .filter(|((acc, s), a)| *s == q && a.at.is_some() && !self.directory.consent.contains(acc))
            .filter_map(|((acc, _), a)| {
                let who = self.account(acc).map_or_else(|| acc.clone(), |x| x.config.email.clone());
                a.error.as_ref().map(|e| format!("{who}: {e}"))
            })
            .collect();
        out.sort();
        out
    }

    /// What a directory answered ([`Event::Directory`]).
    pub(crate) fn apply_directory(
        &mut self,
        account: &str,
        query: &str,
        people: &[mailcraft_model::DirectoryPerson],
        error: Option<String>,
        consent: bool,
    ) {
        if consent {
            self.directory.consent.insert(account.to_string());
        } else if error.is_none() {
            self.directory.consent.remove(account);
        }
        // The person themselves, for their organisation's signature (issue #245).
        self.details_from_directory(account, people);
        let k = (account.to_string(), key(query));
        // Forgotten since (a refresh): a stale answer isn't kept.
        let Some(a) = self.directory.answers.get_mut(&k) else { return };
        if error.is_none() {
            let fine = |p: &&mailcraft_model::DirectoryPerson| mailcraft_model::addr::parse_list(&p.email).is_ok_and(|l| l.len() == 1);
            a.people = people.iter().filter(fine).map(|p| EmailAddress::new(p.name.clone(), p.email.clone())).collect();
            let details = &mut self.directory.details;
            for p in people.iter().filter(fine).filter(|p| !p.title.is_empty() || !p.department.is_empty()) {
                if details.len() >= DETAILS_KEPT {
                    break;
                }
                details.insert(p.email.to_lowercase(), (p.title.clone(), p.department.clone()));
            }
        }
        a.error = error;
        a.at = Some(self.clock.now);
        self.directory.rev = self.directory.rev.wrapping_add(1);
    }

    /// The job title and department the directory gave `email`, when it gave either (issue #213).
    pub fn directory_details(&self, email: &str) -> Option<(&str, &str)> {
        self.directory.details.get(&email.trim().to_lowercase()).map(|(t, d)| (t.as_str(), d.as_str()))
    }

    /// Ask the user to let Exchange Online accounts search their directory (issue #9): `account`,
    /// or each one waiting for permission. The browser opens for each.
    pub fn allow_directory(&mut self, account: Option<&str>) -> CmdResult {
        let wanted: Vec<String> = match account {
            Some(a) => vec![a.to_string()],
            None => self.directory.consent.iter().cloned().collect(),
        };
        let mut asked = Vec::new();
        for id in wanted {
            let config = self.config_of(&id)?;
            if config.graph.is_none() || config.oauth.is_none() {
                return Err(CmdError::Params(format!("{}: only an Exchange Online account signed in with Microsoft asks for this", config.email)));
            }
            asked.push(id);
            self.jobs.push(Job::AllowDirectory { account: config });
        }
        if asked.is_empty() {
            return Err(CmdError::Disabled("no account is waiting for permission to search its directory".into()));
        }
        Ok(serde_json::json!({"asked": asked}))
    }

    /// What asking for that permission came to: the account's searches asked again, or why not.
    pub(crate) fn directory_allowed(&mut self, account: &str, error: Option<String>) {
        match error {
            None => {
                self.directory.consent.remove(account);
                self.directory.answers.retain(|(a, _), _| a != account);
                self.directory.rev = self.directory.rev.wrapping_add(1);
            }
            Some(e) => {
                let who = self.account(account).map_or_else(|| account.to_string(), |a| a.config.email.clone());
                self.notice(format!("{who}: the directory can't be searched: {e}"), true);
            }
        }
    }

    /// Refresh (issue #9): forget what the directories answered, and read every Exchange
    /// account's address books again.
    pub fn refresh_directory(&mut self) -> CmdResult {
        self.directory.answers.clear();
        self.directory.rev = self.directory.rev.wrapping_add(1);
        let exchange: Vec<String> =
            self.accounts.iter().filter(|a| a.config.ews.is_some() || a.config.graph.is_some()).map(|a| a.config.id.clone()).collect();
        for account in &exchange {
            self.jobs.push(Job::SyncDav { account: account.clone() });
        }
        Ok(serde_json::json!({"refreshed": exchange}))
    }
}

/// Whether a person's name or address has a word starting with `q` (lower case), as a
/// directory's search matches.
fn matches_query(a: &EmailAddress, q: &str) -> bool {
    let name = a.name.to_lowercase();
    let email = a.email.to_lowercase();
    email.starts_with(q) || name.starts_with(q) || name.split([' ', ',', '.', '-']).any(|w| w.starts_with(q))
}
