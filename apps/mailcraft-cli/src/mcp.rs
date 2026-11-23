//! `mailcraft-cli mcp` (issue #255): MailCraft's MCP server, newline-delimited JSON-RPC on stdin
//! and stdout (the Craft apps' `craft-mcp-server`), for AI assistants on this computer (Claude
//! Desktop, Claude Code…).
//!
//! Before each tool call it looks for the MailCraft that has the same data folder open and lets
//! agents in (File › Options › AI agents): its control channel's connection file and token in
//! the data folder (`craft-control`'s agents). There it reads the mail, calendar, tasks and
//! people as the window has them, runs a command once the user allowed it in the window, and sees
//! the window. With no such MailCraft, and agents let in by its settings, it reads the data
//! folder itself for that one call, as `mailcraft-cli` does, with the user's settings (the
//! accounts kept from agents among them), and lets go of it after: an assistant stays connected
//! all day, and MailCraft opening meanwhile finds its mail free and takes the next call. It runs
//! nothing there: MailCraft isn't started for an agent, and only the window asks the user about a
//! change. Logs go to stderr; stdout carries the protocol alone.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use craft_control::Effect;
use craft_control::agents::find_window;
use craft_control::client::{CallError, Connection, Request, Waits};
use craft_mcp_server::{Info, Server, Tools, text_result};
use mailcraft_engine::store::Store;
use mailcraft_engine::{Prefs, Session};
use serde_json::{Value, json};

const INFO: Info = Info { name: "mailcraft", title: "MailCraft", version: env!("CARGO_PKG_VERSION"), max_line: 4 << 20 };

/// How long a command waits for the user's answer in the window: a minute past the ten minutes
/// the window waits for it, so the window's own answer (refused, unanswered) comes back.
const ANSWER_WAIT: Duration = Duration::from_secs(660);
/// The largest settings file read.
const SETTINGS_MAX: u64 = 64 << 20;
/// How long the data folder is waited for when it looks open elsewhere, and how often it's tried.
const OPEN_WAIT: Duration = Duration::from_secs(2);
const OPEN_POLL: Duration = Duration::from_millis(100);

/// Said when MailCraft's settings don't let agents in.
const NOT_LET_IN: &str =
    "MailCraft doesn't let AI agents in: the user turns that on in MailCraft, File › Options › AI agents › Let AI agents use MailCraft";

fn app() -> craft_control::auth::App {
    craft_control::auth::App {
        name: "MailCraft",
        data_dir: || None,
        env: |name| std::env::var_os(format!("MAILCRAFT_{name}")),
        utc: |secs| secs.to_string(),
        default_port: 0,
    }
}

/// MailCraft's settings in `root` (`ui.json`), as JSON; nothing when there are none to read.
fn saved_settings(root: &Path) -> Value {
    let path = root.join("ui.json");
    let read = std::fs::metadata(&path).ok().filter(|m| m.len() < SETTINGS_MAX).and_then(|_| std::fs::read(&path).ok());
    read.and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(Value::Null)
}

/// MailCraft's settings in `root`, when they let agents in. Which accounts are kept from them
/// must be read: settings that don't say are refused, never taken as none kept.
fn agents_prefs(root: &Path) -> Result<Prefs, String> {
    let saved = saved_settings(root);
    let saved = saved.get("prefs").filter(|p| p.get("agents").and_then(Value::as_bool) == Some(true)).ok_or(NOT_LET_IN)?;
    match Prefs::from_saved(saved) {
        Some((p, unread)) if !unread.iter().any(|n| n.starts_with("agents")) => Ok(p),
        _ => Err("MailCraft's settings for AI agents couldn't be read: the user sets them again in File › Options › AI agents".into()),
    }
}

/// The tools only the open window has.
const WINDOW_TOOLS: [&str; 4] = ["execute_command", "list_commands", "inspect", "screenshot"];

/// The open window's control channel, kept while it's the same window: port, token, process.
struct WindowLink {
    key: (u16, String, u32),
    connection: Connection,
}

/// MailCraft's tools: what the engine reads ([`mailcraft_engine::agents`]), and the window's.
struct MailTools {
    root: PathBuf,
    files: PathBuf,
    window: Option<WindowLink>,
    /// The organisation's profile (issue #245), read once: its locks hold here too.
    organisation: mailcraft_engine::policy::Organisation,
}

impl MailTools {
    fn new(root: PathBuf, files: PathBuf) -> MailTools {
        MailTools { root, files, window: None, organisation: mailcraft_engine::policy::Organisation::default() }
    }

    /// The MailCraft that has this data folder open and lets agents in now, if one does: its
    /// control channel, made afresh when the window changed.
    fn find_window(&mut self) -> Result<Option<&mut Connection>, String> {
        let Some(w) = find_window(&self.root, "MailCraft")? else {
            self.window = None;
            return Ok(None);
        };
        let key = (w.port, w.token.clone(), w.pid);
        if self.window.as_ref().is_none_or(|l| l.key != key) {
            let mut connection = Connection::new(app(), &w.addr(), Some(&w.token)).with_document("mailbox");
            connection.set_waits(Waits { pending: ANSWER_WAIT, ..Waits::default() });
            self.window = Some(WindowLink { key, connection });
        }
        Ok(self.window.as_mut().map(|l| &mut l.connection))
    }

    /// Ask the open window `method` with `params`.
    fn ask_window(c: &mut Connection, method: &str, params: Value) -> Result<Value, String> {
        let changes = method == "engine.execute";
        let what = match params.get("command").and_then(Value::as_str) {
            Some(cmd) if changes => format!("`{method}` ({cmd})"),
            _ => format!("`{method}`"),
        };
        // A change is never sent twice: the window may have run it.
        let request =
            Request { method: method.into(), params, what, effect: if changes { Effect::Changes } else { Effect::ReadsOnly }, once: changes };
        c.call(request).map_err(|e| match e {
            // The library's own words name a way of starting a Craft app that MailCraft doesn't have.
            CallError::NotRunning(_) => "MailCraft closed, or stopped letting AI agents in, before this call: nothing ran".into(),
            CallError::Lost(_) => "MailCraft stopped answering during this call (it closed, or AI agents were turned off), after running it or not: look before trying a change again".into(),
            other => other.message().to_string(),
        })
    }

    /// Read the data folder here, for this one call: MailCraft isn't open on it with agents let in.
    fn here(&self, tool: &str, args: &Value) -> Result<Value, String> {
        let prefs = agents_prefs(&self.root)?;
        let (store, mut s) = self.open_here()?;
        s.prefs = prefs;
        // AI agents turned off by the organisation stay off, whatever the person's setting. Only
        // that is looked at: its profile put into the session would put in its signatures, and
        // write the accounts' settings, in a call that only reads.
        let domains: Vec<String> = s.accounts.iter().map(|a| mailcraft_engine::policy::domain_of(&a.config.email)).collect();
        let locked_off = self.organisation.effective(&domains).settings.get("agents").is_some_and(|g| g.locked && g.value == Value::Bool(false));
        if !s.prefs.agents || locked_off {
            return Err(NOT_LET_IN.into());
        }
        let read = s.agent_read(tool, args);
        // The session and the store close here: the files are MailCraft's again.
        drop((s, store));
        read
    }

    /// The data folder opened here, every account's mail with it: an account whose mail can't be
    /// read fails the call, rather than reading as if it had none. A store closed a moment ago may
    /// still be letting go of a data file (one being sealed as it closed), which looks like
    /// MailCraft having it: a moment's wait before saying so.
    fn open_here(&self) -> Result<(Store, Session), String> {
        let open_there =
            "MailCraft has its data folder open but isn't letting AI agents in at the moment: the user turns that on in File › Options › AI agents";
        let mut waited = Duration::ZERO;
        loop {
            // The session reads each account's folders as it opens, and one it couldn't read it
            // takes as having none: what the store has now must be what the session took, or
            // it's opened again. (`Err(None)`: not read whole yet, for no reason said.)
            let opened = Store::open_packed(&self.root, &self.files).map_err(Some).and_then(|store| {
                let s = Session::open(Some(store.clone()), crate::clock());
                for a in &s.accounts {
                    let folders = store.load_folders(&a.config.id).map_err(Some)?;
                    if folders.len() != a.folders.len() {
                        return Err(None);
                    }
                }
                Ok((store, s))
            });
            match opened {
                Ok(here) => return Ok(here),
                Err(_) if waited < OPEN_WAIT => {
                    std::thread::sleep(OPEN_POLL);
                    waited += OPEN_POLL;
                }
                Err(Some(e)) if e.is_open_elsewhere() => return Err(open_there.into()),
                Err(Some(e)) => return Err(e.to_string()),
                Err(None) => return Err(open_there.into()),
            }
        }
    }

    fn run(&mut self, name: &str, args: &Value) -> Result<Value, String> {
        let text = |k: &str| args.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
        let Some(w) = self.find_window()? else {
            if WINDOW_TOOLS.contains(&name) {
                return Err(format!("{name} needs MailCraft's window: open MailCraft and let AI agents in there (File › Options › AI agents)"));
            }
            return self.here(name, args);
        };
        match name {
            "execute_command" => {
                let params = args.get("params").cloned().unwrap_or_else(|| json!({}));
                Self::ask_window(w, "engine.execute", json!({"command": text("command"), "params": params}))
            }
            "list_commands" => Self::ask_window(w, "engine.commands", json!({"search": text("search")})),
            "inspect" => Self::ask_window(w, "ui.inspect", json!({})),
            "screenshot" => Self::ask_window(w, "ui.screenshot", json!({})),
            tool => Self::ask_window(w, "agents.read", json!({"tool": tool, "args": args})),
        }
    }
}

impl Tools for MailTools {
    fn instructions(&self) -> String {
        "MailCraft is the user's mail, calendar, tasks and people. Read with list_accounts, list_messages, search_mail, \
         read_message, list_events, list_tasks, find_people and everything_with: they change nothing (a message read isn't \
         marked read). With MailCraft open and AI agents let in (File › Options › AI agents), list_commands lists what it \
         can do, and execute_command runs one where the user lets agents ask to run commands: MailCraft asks the user in \
         its window first, so a command may wait for their answer, and Deny is an answer to respect. inspect says what its \
         window shows, screenshot shows it. Accounts the user keeps from agents are left out of everything."
            .into()
    }

    fn list(&mut self) -> Vec<Value> {
        let mut tools = mailcraft_engine::agents::agent_tool_list();
        let read = |name: &str, title: &str, desc: &str, schema: Value| {
            json!({"name": name, "title": title, "description": desc, "inputSchema": schema,
                   "annotations": {"title": title, "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false}})
        };
        let none = json!({"type": "object", "properties": {}, "additionalProperties": false});
        tools.push(json!({
            "name": "execute_command",
            "title": "Run a MailCraft command",
            "description": "Run one of MailCraft's commands (list_commands has them, each with its params) in its open window: sending, replying, moving, flagging, answering an invitation, making an event or a task… The user is asked in MailCraft's window first, unless they always allow that command: the call waits for their answer.",
            "inputSchema": {"type": "object", "properties": {
                "command": {"type": "string", "description": "The command's id (`mail.flag`, `event.create`…)."},
                "params": {"type": "object", "description": "Its params, as list_commands describes them."}
            }, "required": ["command"], "additionalProperties": false},
            "annotations": {"title": "Run a MailCraft command", "readOnlyHint": false, "destructiveHint": true, "idempotentHint": false, "openWorldHint": true},
        }));
        tools.push(read(
            "list_commands",
            "MailCraft's commands",
            "Every command MailCraft has (those whose id, label or place has `search`): id, label, where it is in the window, its params, whether it can run now, and whether agents run it without the user being asked.",
            json!({"type": "object", "properties": {"search": {"type": "string", "description": "Words of the id, label or place."}}, "additionalProperties": false}),
        ));
        tools.push(read(
            "inspect",
            "What MailCraft shows",
            "What MailCraft's window shows now: Mail, Calendar, People, Tasks or Notes, the folder, the search, what's selected and open, the message being written, a dialog, and the commands waiting for the user.",
            none.clone(),
        ));
        tools.push(read("screenshot", "Picture of MailCraft", "A picture of MailCraft's window as it is now (PNG).", none));
        tools
    }

    fn call(&mut self, name: &str, args: &Value) -> Value {
        match self.run(name, args) {
            Ok(v) if name == "screenshot" => match v.get("png").and_then(Value::as_str) {
                Some(png) => json!({"content": [
                    {"type": "image", "data": png, "mimeType": "image/png"},
                    {"type": "text", "text": format!("MailCraft's window, {}×{}", v["width"], v["height"])},
                ]}),
                None => text_result("the window sent no picture", true),
            },
            Ok(v) => text_result(serde_json::to_string_pretty(&v).unwrap_or_default(), false),
            Err(e) => text_result(e, true),
        }
    }
}

/// Serve MCP on stdin and stdout until the client closes them.
pub(crate) fn serve(root: PathBuf, files: PathBuf) -> Result<(), String> {
    let mut tools = MailTools::new(root, files);
    tools.organisation = crate::organisation();
    let mut server = Server::new(INFO, tools);
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let served = server.serve(stdin.lock(), stdout.lock());
    let _ = std::io::stdout().flush();
    served.map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests;
