//! What AI agents read here (issue #255): the accounts and their folders, a folder's mail, a
//! search of it, one message, the calendar, the tasks and the people, and everything with one
//! person (issue #178), each a tool of MailCraft's MCP server. Nothing of it changes anything:
//! no message is marked read, the window's folder, search and reading pane stay as they are,
//! and nothing is asked of a server. Changes go through the commands, which the window asks the
//! user about first.
//!
//! Dates go out twice: as Unix seconds (`date`, `start`…), and as the user reads them, in their
//! time zone (`when`).

use mailcraft_model::summary::{MessageKey, Summary};
use serde_json::{Value, json};

use crate::Session;

/// The most messages one answer lists.
pub const MESSAGES_MAX: usize = 200;
/// The most characters of a message's text one answer holds.
pub const TEXT_MAX: usize = 100_000;
/// The longest a span of days asked for the calendar may be.
const DAYS_MAX: i64 = 366;

/// The most people one answer lists.
const PEOPLE_MAX: usize = 50;

/// A tool: its name, title, what it does, and its arguments (a JSON schema).
pub struct AgentTool {
    pub name: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub schema: fn() -> Value,
}

fn none() -> Value {
    json!({"type": "object", "properties": {}, "additionalProperties": false})
}

fn object(props: Value, required: &[&str]) -> Value {
    json!({"type": "object", "properties": props, "required": required, "additionalProperties": false})
}

/// Every tool agents read with.
pub const AGENT_TOOLS: &[AgentTool] = &[
    AgentTool {
        name: "list_accounts",
        title: "Accounts and folders",
        description: "The mail accounts in MailCraft, each with its folders: their paths (to name a folder in other tools), names, roles (inbox, sent, drafts…) and how many messages and unread ones they hold.",
        schema: none,
    },
    AgentTool {
        name: "list_messages",
        title: "Messages in a folder",
        description: "The messages in one folder, newest first: each one's key (to read it with read_message), date, sender, recipients, subject, the start of its text, and whether it's unread, flagged or has files. Reading them changes nothing.",
        schema: || {
            object(
                json!({
                    "account": {"type": "string", "description": "The account's id (list_accounts); default: the first account."},
                    "folder": {"type": "string", "description": "The folder's path (list_accounts); default: the account's Inbox."},
                    "limit": {"type": "integer", "description": "How many, at most (default 50, 200 at most)."},
                    "offset": {"type": "integer", "description": "How many of the newest to skip (default 0)."}
                }),
                &[],
            )
        },
    },
    AgentTool {
        name: "search_mail",
        title: "Search mail",
        description: "Messages in every account's folders that match a search, newest first, as MailCraft's search box finds them: words anywhere (in the text of the messages downloaded), from:, to:, subject:, has:attachment, is:unread, is:flagged, received:2026-11, before:/after: a date, category:, -word to leave out, \"a phrase\". Reading them changes nothing, and the search box keeps what it had.",
        schema: || {
            object(
                json!({
                    "query": {"type": "string", "description": "What to search for, as typed in MailCraft's search box."},
                    "account": {"type": "string", "description": "Only this account (its id)."},
                    "folder": {"type": "string", "description": "Only this folder (its path; with account)."},
                    "limit": {"type": "integer", "description": "How many, at most (default 50, 200 at most)."}
                }),
                &["query"],
            )
        },
    },
    AgentTool {
        name: "read_message",
        title: "Read a message",
        description: "One message by its key (from list_messages or search_mail): its sender, recipients, date, subject, text (HTML mail as text) and the files attached. It isn't marked read. A message whose text isn't downloaded yet says so.",
        schema: || object(json!({"key": {"type": "string", "description": "The message's key."}}), &["key"]),
    },
    AgentTool {
        name: "list_events",
        title: "Calendar",
        description: "The events in the user's calendars from one day to another (each occurrence of a series on its own): title, start and end, place, whether it's all day, the user's answer to a meeting, its organizer and guests, and its calendar. Default: today and the six days after.",
        schema: || {
            object(
                json!({
                    "from": {"type": "string", "description": "The first day, YYYY-MM-DD (default: today)."},
                    "to": {"type": "string", "description": "The last day, YYYY-MM-DD (default: six days after the first; a year at most)."}
                }),
                &[],
            )
        },
    },
    AgentTool {
        name: "list_tasks",
        title: "Tasks",
        description: "The user's tasks: title, due day, whether done, its list, and its notes. A search narrows them.",
        schema: || {
            object(
                json!({
                    "query": {"type": "string", "description": "Words its title or notes have (default: every task)."},
                    "done": {"type": "boolean", "description": "Done ones too (default false)."}
                }),
                &[],
            )
        },
    },
    AgentTool {
        name: "find_people",
        title: "Find people",
        description: "People whose name or address starts so, from the user's address books and the mail and meetings they've had with them, best known first: name and address.",
        schema: || {
            object(
                json!({
                    "query": {"type": "string", "description": "The start of a name, a word of it, or of an address."},
                    "limit": {"type": "integer", "description": "How many, at most (default 10, 50 at most)."}
                }),
                &["query"],
            )
        },
    },
    AgentTool {
        name: "everything_with",
        title: "Everything with one person",
        description: "Everything here with one person, by their address: the conversations the user and they had (each with its messages' keys), the files and links in them, the meetings they organize or are invited to, and the others on that mail. A search narrows it.",
        schema: || {
            object(
                json!({
                    "email": {"type": "string", "description": "Their address."},
                    "query": {"type": "string", "description": "A search narrowing every part, as search_mail's."}
                }),
                &["email"],
            )
        },
    },
];

/// The tools' definitions as MCP lists them (`name`, `title`, `description`, `inputSchema`, read
/// only).
pub fn agent_tool_list() -> Vec<Value> {
    AGENT_TOOLS
        .iter()
        .map(|t| {
            json!({
                "name": t.name,
                "title": t.title,
                "description": t.description,
                "inputSchema": (t.schema)(),
                "annotations": {"title": t.title, "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false},
            })
        })
        .collect()
}

/// A number argument within `0..=most`, else `default`.
fn count(args: &Value, name: &str, default: usize, most: usize) -> usize {
    args.get(name).and_then(Value::as_u64).map_or(default, |n| usize::try_from(n).unwrap_or(most).min(most))
}

/// The folder of `folders` at `path`.
fn by_path<'a>(folders: &'a [mailcraft_model::Folder], path: &str) -> Option<&'a mailcraft_model::Folder> {
    folders.iter().find(|f| f.path == path)
}

fn text<'a>(args: &'a Value, name: &str) -> Option<&'a str> {
    args.get(name).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())
}

impl Session {
    /// Whether agents may ask to run the command `id` at all (the window asks the user about
    /// each, unless the user always allows it): one there is, and not one of the agents' own
    /// settings, which only the user changes. Its label, to ask about it by.
    pub fn agent_command(&self, id: &str) -> Result<&'static str, String> {
        let registry = self.registry();
        let label = registry.get(id).map(|spec| spec.label).ok_or_else(|| format!("no command `{id}` (list_commands has them)"))?;
        if id.starts_with("options.agents") {
            return Err("agents' own settings are the user's to change, in File › Options".into());
        }
        Ok(label)
    }

    /// Whether an agent may ask to run the command `id` with `params` now: commands let in
    /// (File › Options › AI agents), [`Session::agent_command`]'s, and nothing of an account kept
    /// from agents named in its params. Its label, to ask about it by.
    pub fn agent_may_run(&self, id: &str, params: &Value) -> Result<&'static str, String> {
        if !self.prefs.agents_run {
            return Err(
                "MailCraft lets AI agents read only: the user lets them ask to run commands in File › Options › AI agents › Let AI agents ask to run commands"
                    .into(),
            );
        }
        let label = self.agent_command(id)?;
        if let Some(a) = self.agent_hidden_in(params, 0) {
            return Err(format!("the account `{a}` is kept from AI agents"));
        }
        Ok(label)
    }

    /// Whether agents read the account `id`: not one kept from them.
    pub fn agent_sees(&self, id: &str) -> bool {
        !self.prefs.agents_hidden.iter().any(|a| a == id)
    }

    /// An account kept from agents that `v` names: its id, or a key of one of its messages,
    /// folders or items (`work:…`), anywhere in it.
    fn agent_hidden_in(&self, v: &Value, depth: usize) -> Option<String> {
        if depth > 32 {
            return self.prefs.agents_hidden.first().cloned();
        }
        match v {
            Value::String(s) => {
                self.prefs.agents_hidden.iter().find(|a| s == *a || s.strip_prefix(a.as_str()).is_some_and(|r| r.starts_with([':', '|']))).cloned()
            }
            Value::Array(a) => a.iter().find_map(|x| self.agent_hidden_in(x, depth + 1)),
            Value::Object(o) => o.values().find_map(|x| self.agent_hidden_in(x, depth + 1)),
            _ => None,
        }
    }

    /// Run the agents' tool `tool` with `args`: what it read, or why it couldn't. An account kept
    /// from agents is left out of all of it, as if it weren't there.
    pub fn agent_read(&mut self, tool: &str, args: &Value) -> Result<Value, String> {
        match tool {
            "list_accounts" => {
                let mut v = self.run("account.list", &json!({})).map_err(|e| e.to_string())?;
                if let Value::Array(list) = &mut v {
                    list.retain(|a| a["account"].as_str().is_some_and(|id| self.agent_sees(id)));
                }
                Ok(v)
            }
            "list_messages" => self.agent_messages(args),
            "search_mail" => self.agent_search(args),
            "read_message" => self.agent_message(args),
            "list_events" => self.agent_events(args),
            "list_tasks" => Ok(self.agent_tasks(args)),
            "find_people" => {
                let q = text(args, "query").ok_or("say whom to look for")?;
                let limit = count(args, "limit", 10, PEOPLE_MAX);
                // Nothing asked of a directory's server. With an account kept from agents, those
                // only it knows of (its mail, its address book) aren't told of, nor its
                // organisation's directory: the most asked for, then those the others know.
                let kept = !self.prefs.agents_hidden.is_empty();
                let mut people = self.suggest_contacts_here(q, if kept { PEOPLE_MAX } else { limit }, !kept);
                if kept {
                    let known = self.agent_known_people();
                    people.retain(|c| known.contains(&c.address.email.trim().to_lowercase()));
                    people.truncate(limit);
                }
                Ok(json!(people.iter().map(|c| json!({"name": c.address.name, "email": c.address.email})).collect::<Vec<_>>()))
            }
            "everything_with" => self.agent_person(args),
            other => Err(format!("no tool `{other}`")),
        }
    }

    /// A message's summary as agents read it.
    pub(crate) fn agent_summary(&self, m: &Summary) -> Value {
        let folder = self.account(&m.key.account).and_then(|a| by_path(&a.folders, &m.key.folder)).map(|f| f.display_name()).unwrap_or_default();
        json!({
            "key": m.key.to_id(),
            "account": m.key.account,
            "folder": m.key.folder,
            "folderName": folder,
            "date": m.date,
            "when": mailcraft_model::dates::full_time(m.date, &self.clock),
            "from": m.from.to_string(),
            "to": mailcraft_model::addr::format_list(&m.to),
            "cc": mailcraft_model::addr::format_list(&m.cc),
            "subject": m.subject,
            "preview": m.preview,
            "unread": !m.flags.seen,
            "flagged": m.flags.flagged,
            "attachments": m.has_attachments,
        })
    }

    /// The addresses the accounts agents read know of, lower-case: their address books', and
    /// their Inbox's and Sent Items' (senders and recipients), as far as those are on this
    /// computer. Every folder of every account would be too much to load for one look-up.
    fn agent_known_people(&mut self) -> std::collections::HashSet<String> {
        let mut known = std::collections::HashSet::new();
        for (a, p) in &self.pim {
            if self.agent_sees(a) {
                known.extend(p.cards.iter().flat_map(|c| c.card.emails.iter().map(|e| e.value.trim().to_lowercase())));
            }
        }
        let folders: Vec<(String, String)> = self
            .accounts
            .iter()
            .filter(|a| self.agent_sees(&a.config.id))
            .flat_map(|a| {
                [mailcraft_model::FolderRole::Inbox, mailcraft_model::FolderRole::Sent]
                    .into_iter()
                    .filter_map(|role| a.folder(role).map(|f| (a.config.id.clone(), f.path.clone())))
            })
            .collect();
        for (a, f) in folders {
            for m in &self.folder_state(&a, &f).messages {
                known.extend(std::iter::once(&m.from).chain(&m.to).chain(&m.cc).map(|x| x.email.trim().to_lowercase()));
            }
        }
        known
    }

    /// The account `id` names, when agents read it; else as if there were none.
    fn agent_account(&self, id: &str) -> Result<&crate::AccountState, String> {
        self.account(id).filter(|_| self.agent_sees(id)).ok_or_else(|| format!("no account `{id}`"))
    }

    fn agent_messages(&mut self, args: &Value) -> Result<Value, String> {
        let account = match text(args, "account") {
            Some(a) => self.agent_account(a)?,
            None => self.accounts.iter().find(|a| self.agent_sees(&a.config.id)).ok_or("no accounts yet")?,
        };
        let id = account.config.id.clone();
        let folder = match text(args, "folder") {
            Some(f) => by_path(&account.folders, f).map(|f| f.path.clone()).ok_or_else(|| format!("no folder `{f}` in {id}"))?,
            None => account.folder(mailcraft_model::FolderRole::Inbox).map(|f| f.path.clone()).ok_or("the account has no Inbox")?,
        };
        let (limit, offset) = (count(args, "limit", 50, MESSAGES_MAX), count(args, "offset", 0, usize::MAX));
        let mut list: Vec<mailcraft_model::summary::SharedSummary> =
            self.folder_state(&id, &folder).messages.iter().filter(|m| !m.flags.deleted).cloned().collect();
        list.sort_by_key(|m| std::cmp::Reverse(m.date));
        let total = list.len();
        let messages: Vec<Value> = list.iter().skip(offset).take(limit).map(|m| self.agent_summary(m)).collect();
        Ok(json!({"account": id, "folder": folder, "total": total, "messages": messages}))
    }

    fn agent_message(&mut self, args: &Value) -> Result<Value, String> {
        let id = text(args, "key").ok_or("say which message (its key)")?;
        let key = MessageKey::from_id(id).ok_or_else(|| format!("`{id}` isn't a message's key"))?;
        if !self.agent_sees(&key.account) {
            return Err(format!("no message `{id}`"));
        }
        let summary = self.folder_state(&key.account, &key.folder).messages.iter().find(|m| m.key == key).cloned();
        let Some(raw) = self.body_raw(&key) else {
            let s = summary.ok_or_else(|| format!("no message `{id}`"))?;
            let mut v = self.agent_summary(&s);
            v["downloaded"] = json!(false);
            return Ok(v);
        };
        let m = mailcraft_mime::parse(&raw).ok_or("the message couldn't be read")?;
        let mut body: String = m.text.chars().take(TEXT_MAX).collect();
        if m.text.chars().nth(TEXT_MAX).is_some() {
            body.push_str("\n[…]");
        }
        Ok(json!({
            "key": key.to_id(),
            "downloaded": true,
            "from": m.from.to_string(),
            "to": mailcraft_model::addr::format_list(&m.to),
            "cc": mailcraft_model::addr::format_list(&m.cc),
            "date": m.date,
            "when": mailcraft_model::dates::full_time(m.date, &self.clock),
            "subject": m.subject,
            "text": body,
            "attachments": m.attachments.iter().map(|a| json!({"index": a.index, "name": a.name, "mime": a.mime, "size": a.size})).collect::<Vec<_>>(),
            "unread": summary.is_some_and(|s| !s.flags.seen),
        }))
    }

    fn agent_events(&mut self, args: &Value) -> Result<Value, String> {
        let day = |name: &str| -> Result<Option<i64>, String> {
            text(args, name)
                .map(|d| mailcraft_model::dates::read_day(d, self.clock.style.order).ok_or_else(|| format!("{name}: `{d}` isn't a day (YYYY-MM-DD)")))
                .transpose()
        };
        let today = self.clock.local_day(self.clock.now);
        let first = day("from")?.unwrap_or(today);
        let last = day("to")?.unwrap_or_else(|| first.saturating_add(6));
        if last < first || last.saturating_sub(first) >= DAYS_MAX {
            return Err("from one day to another, a year at most".into());
        }
        let (from, to) = (self.local_midnight(first), self.local_midnight(last.saturating_add(1)));
        let mine = self.my_addresses();
        let events: Vec<Value> = self
            .agenda(from, to)
            .into_iter()
            .filter(|i| !i.task && self.agent_sees(&i.account))
            .take(1000)
            .map(|i| {
                let stored = self.pim.get(&i.account).and_then(|p| {
                    p.events
                        .iter()
                        .find(|e| e.event.uid == i.uid && e.event.recurrence_id == Some(i.occurrence))
                        .or_else(|| p.events.iter().find(|e| e.event.uid == i.uid && e.event.recurrence_id.is_none()))
                });
                let calendar = self.pim.get(&i.account).and_then(|p| p.calendars.iter().find(|c| c.id == i.calendar)).map(|c| c.name.clone());
                let mut v = json!({
                    "uid": i.uid,
                    "account": i.account,
                    "calendar": calendar.unwrap_or_else(|| i.calendar.clone()),
                    "title": i.summary,
                    "location": i.location,
                    "start": i.start,
                    "end": i.end,
                    "when": mailcraft_model::dates::full_time(i.start, &self.clock),
                    "allDay": i.all_day,
                    "recurring": i.recurring,
                    "occurrence": i.occurrence,
                    "status": i.status,
                    "showAs": format!("{:?}", i.show_as),
                    "busyOnly": i.busy_only,
                });
                if let Some(e) = stored.map(|s| &s.event).filter(|_| !i.busy_only) {
                    v["organizer"] = json!(e.organizer.as_ref().map(|o| o.email.clone()));
                    v["guests"] =
                        json!(e.attendees.iter().map(|a| json!({"email": a.email, "name": a.name, "answer": a.partstat})).collect::<Vec<_>>());
                    v["yourAnswer"] =
                        json!(e.attendees.iter().find(|a| mine.iter().any(|m| m.eq_ignore_ascii_case(&a.email))).map(|a| a.partstat.clone()));
                    v["notes"] = json!(e.description.chars().take(4000).collect::<String>());
                }
                v
            })
            .collect();
        Ok(json!({"from": from, "to": to, "events": events}))
    }

    fn agent_tasks(&self, args: &Value) -> Value {
        let done = args.get("done").and_then(Value::as_bool).unwrap_or(false);
        let tasks: Vec<Value> = self
            .tasks(text(args, "query").unwrap_or(""))
            .into_iter()
            .filter(|t| (done || !t.task.is_done()) && self.agent_sees(t.account))
            .take(500)
            .map(|t| {
                json!({
                    "uid": t.task.uid,
                    "account": t.account,
                    "list": t.list,
                    "title": t.task.title,
                    "due": t.task.due,
                    "done": t.task.is_done(),
                    "notes": t.task.notes.chars().take(4000).collect::<String>(),
                })
            })
            .collect();
        json!(tasks)
    }

    fn agent_search(&mut self, args: &Value) -> Result<Value, String> {
        let query = text(args, "query").ok_or("say what to search for")?;
        let folders: Vec<(String, String)> = match (text(args, "account"), text(args, "folder")) {
            (Some(a), Some(f)) => {
                let account = self.agent_account(a)?;
                let folder = by_path(&account.folders, f).ok_or_else(|| format!("no folder `{f}` in {a}"))?;
                vec![(a.to_string(), folder.path.clone())]
            }
            (Some(a), None) => {
                let account = self.agent_account(a)?;
                account.folders.iter().filter(|f| f.selectable).map(|f| (a.to_string(), f.path.clone())).collect()
            }
            (None, Some(_)) => return Err("say the folder's account too".into()),
            (None, None) => {
                let mut all = self.scope_folders(crate::SearchScope::All);
                all.retain(|(a, _)| self.agent_sees(a));
                all
            }
        };
        let found = self.agent_search_in(query, &folders);
        let limit = count(args, "limit", 50, MESSAGES_MAX);
        let messages: Vec<Value> = found.hits.iter().take(limit).map(|m| self.agent_summary(m)).collect();
        Ok(json!({
            "query": query,
            "found": found.hits.len(),
            "messages": messages,
            "textsRead": found.texts_read,
            "textsNotRead": found.texts_left,
        }))
    }

    fn agent_person(&mut self, args: &Value) -> Result<Value, String> {
        let email = text(args, "email").ok_or("say whose (their address)")?;
        if !mailcraft_model::addr::is_valid_email(email) {
            return Err(format!("`{email}` isn't an address"));
        }
        // From the accounts agents read alone, before any limit or a message met twice is left out.
        let kept = self.prefs.agents_hidden.clone();
        let all = self.person_activity_in(email, &|a| !kept.iter().any(|k| k == a));
        let p = match text(args, "query") {
            Some(q) => all.refined(q),
            None => all,
        };
        let threads: Vec<Value> = p
            .threads
            .iter()
            .take(100)
            .map(|t| json!({"subject": t.subject, "latest": t.latest, "messages": t.messages.iter().take(50).map(|m| self.agent_summary(m)).collect::<Vec<_>>()}))
            .collect();
        Ok(json!({
            "email": email,
            "messages": p.message_count(),
            "conversations": threads,
            "files": p.files.iter().take(200).map(|f| json!({"name": f.name, "size": f.size, "key": f.key.to_id(), "index": f.index, "date": f.date, "fromThem": f.from_them})).collect::<Vec<_>>(),
            "links": p.links.iter().take(200).map(|l| json!({"url": l.url, "key": l.key.to_id(), "date": l.date})).collect::<Vec<_>>(),
            "meetings": p.meetings.iter().take(200).map(|m| json!({"uid": m.uid, "title": m.summary, "start": m.start, "theirs": m.theirs})).collect::<Vec<_>>(),
            "people": p.people.iter().take(100).map(|x| json!({"name": x.name, "email": x.email})).collect::<Vec<_>>(),
        }))
    }
}
