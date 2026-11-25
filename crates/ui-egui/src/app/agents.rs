//! AI agents in the open window (issue #255): what MailCraft's MCP server asks of it, over the
//! control channel the desktop app serves while the user lets agents in (File › Options › AI
//! agents). What reads (the agents' tools, the commands there are, what the window shows) is
//! answered at once and changes nothing. A command waits for the user, in a small window of its
//! own above the rest (a dialog being filled in stays): Allow, Always allow (that command, from
//! then on), or Deny. What agents asked, and what came of it, is listed in File › Info.

use std::collections::VecDeque;

use serde_json::{Value, json};

use crate::*;

/// What agents asked lately, kept for File › Info.
const LOG_KEPT: usize = 50;
/// The most commands waiting for the user at once; past that, one is refused.
const ASKS_MAX: usize = 8;
/// The most characters of a command's params the asking window shows; past that, it says so.
const PARAMS_SHOWN: usize = 20_000;

/// How the window answers a request.
#[derive(Debug, PartialEq)]
pub enum AgentAnswer {
    /// Answered.
    Now(Result<Value, String>),
    /// Waits for the user: [`MailApp::agent_decisions`] gives its answer under this ticket.
    Asking(u64),
}

/// A command an agent asked to run, waiting for the user.
#[derive(Clone, Debug)]
pub struct AgentAsk {
    pub ticket: u64,
    pub command: String,
    pub label: &'static str,
    pub params: Value,
    /// The params as the asking window shows them, made once.
    pub shown: String,
    /// What it's about, in words, made once ([`MailApp::described`]).
    pub about: Vec<String>,
}

/// What an agent asked, and what came of it.
#[derive(Clone, Debug)]
pub struct AgentLogLine {
    pub at: i64,
    /// The tool, or the command's id.
    pub what: String,
    pub outcome: AgentOutcome,
}

/// What came of an agent's request.
#[derive(Clone, Debug, PartialEq)]
pub enum AgentOutcome {
    /// Read; nothing changed.
    Read,
    /// The command ran.
    Ran,
    /// The user didn't allow it.
    Denied,
    /// It failed: why, as the engine said.
    Failed(String),
}

impl AgentOutcome {
    /// As File › Info says it.
    pub fn text(&self) -> String {
        match self {
            AgentOutcome::Read => tl!("Looked, changed nothing").to_string(),
            AgentOutcome::Ran => tl!("Ran").to_string(),
            AgentOutcome::Denied => tl!("You didn't allow it").to_string(),
            AgentOutcome::Failed(why) => tf!("Didn't run: {why}", why = why),
        }
    }
}

/// What agents asked of the window, and what's waiting.
#[derive(Default)]
pub struct Agents {
    pub(crate) asks: Vec<AgentAsk>,
    decided: Vec<(u64, Result<Value, String>)>,
    pub(crate) log: VecDeque<AgentLogLine>,
    next: u64,
    /// The ask last brought to the user's attention.
    announced: u64,
    /// Commands run whose answer is the server's, awaited ([`SETTLES`]).
    settling: Vec<Settling>,
}

/// A command that asks a server, answering `pending` (issue #255: an agent was given only that):
/// its answer is what it reads with `refresh` false, once that isn't pending any more.
const SETTLES: [&str; 2] = ["rules.server", "account.vacation"];
/// How long a server's answer is awaited, in seconds.
const SETTLE_WAIT: f64 = 60.0;

/// A command run whose answer is awaited from the server.
#[derive(Clone, Debug)]
struct Settling {
    ticket: u64,
    command: String,
    params: Value,
    /// When it was first looked at (the window's clock), once it has been.
    since: Option<f64>,
}

/// A reply's error, refused because agents are off.
pub const AGENTS_OFF: &str = "AI agents are turned off in MailCraft: File › Options › AI agents › Let AI agents use MailCraft";

impl MailApp {
    /// Answer a request an agent sent the window: `method` with `params`, as the control channel
    /// has them.
    pub fn agent_request(&mut self, method: &str, params: &Value) -> AgentAnswer {
        if !self.session.prefs.agents {
            return AgentAnswer::Now(Err(AGENTS_OFF.into()));
        }
        let text = |name: &str| params.get(name).and_then(Value::as_str).unwrap_or_default().to_string();
        match method {
            "agents.read" => {
                let tool = text("tool");
                let args = params.get("args").cloned().unwrap_or_else(|| json!({}));
                let read = self.session.agent_read(&tool, &args);
                self.agent_log(tool, read.as_ref().map_or_else(|e| AgentOutcome::Failed(e.clone()), |_| AgentOutcome::Read));
                AgentAnswer::Now(read)
            }
            "engine.commands" => AgentAnswer::Now(Ok(self.agent_commands(&text("search")))),
            "ui.inspect" => {
                // What the window shows can be a message's subject and who it's from: noted.
                self.agent_log("inspect".into(), AgentOutcome::Read);
                AgentAnswer::Now(Ok(self.agent_inspect()))
            }
            "engine.execute" => self.agent_execute(&text("command"), params.get("params").cloned().unwrap_or_else(|| json!({}))),
            other => AgentAnswer::Now(Err(format!("unknown method `{other}`"))),
        }
    }

    /// Whether an agent may have a picture of the window now, noted as what it read when it may:
    /// agents let in, and no account kept from them (the window could show it: the calendar
    /// shows every account's).
    pub fn agent_may_picture(&mut self) -> Result<(), String> {
        if !self.session.prefs.agents {
            return Err(AGENTS_OFF.into());
        }
        if !self.session.prefs.agents_hidden.is_empty() {
            return Err("no picture of the window while an account is kept from AI agents: it could show it".into());
        }
        self.agent_log("screenshot".into(), AgentOutcome::Read);
        Ok(())
    }

    /// A command an agent asks to run: at once when the user always allows it, else asked about.
    fn agent_execute(&mut self, command: &str, params: Value) -> AgentAnswer {
        if !params.is_object() {
            return AgentAnswer::Now(Err("a command's params are an object".into()));
        }
        let label = match self.agent_gate(command, &params) {
            Ok(label) => label,
            Err(e) => return AgentAnswer::Now(Err(e)),
        };
        if self.session.prefs.agents_always.iter().any(|c| c == command) {
            let ran = self.agent_run(command, &params);
            if awaits_server(command, &ran) {
                self.ui.agents.next = self.ui.agents.next.wrapping_add(1);
                let ticket = self.ui.agents.next;
                self.ui.agents.settling.push(Settling { ticket, command: command.to_string(), params, since: None });
                return AgentAnswer::Asking(ticket);
            }
            return AgentAnswer::Now(ran);
        }
        if self.ui.agents.asks.len() >= ASKS_MAX {
            return AgentAnswer::Now(Err("too many commands are waiting for the user's answer".into()));
        }
        self.ui.agents.next = self.ui.agents.next.wrapping_add(1);
        let ticket = self.ui.agents.next;
        let shown = shown_params(&params);
        let about = self.described(&params);
        self.ui.agents.asks.push(AgentAsk { ticket, command: command.to_string(), label, params, shown, about });
        AgentAnswer::Asking(ticket)
    }

    /// Whether an agent may have `command` run with `params` now, asked when it asks, every frame
    /// while it waits and again as the user allows it: [`Session::agent_may_run`]'s, and, with an
    /// account kept from agents, not while the window shows that account's mail (a command
    /// without params acts on what's shown).
    fn agent_gate(&self, command: &str, params: &Value) -> Result<&'static str, String> {
        let label = self.session.agent_may_run(command, params)?;
        if self.agent_window_shows_kept() {
            return Err("MailCraft's window shows an account kept from AI agents: no command of theirs runs while it does".into());
        }
        Ok(label)
    }

    /// The window shows mail of an account kept from agents: its folder (or every account's,
    /// as All Inboxes and the Outbox do), a message of it selected, open or being written.
    fn agent_window_shows_kept(&self) -> bool {
        let s = &self.session;
        if s.prefs.agents_hidden.is_empty() {
            return false;
        }
        let view = match &s.view {
            mailcraft_engine::View::Folder { account, .. } => !s.agent_sees(account),
            mailcraft_engine::View::Empty => false,
            _ => true,
        };
        view || s.selection.iter().any(|k| !s.agent_sees(&k.account))
            || s.reading.as_ref().is_some_and(|r| !s.agent_sees(&r.key.account))
            || s.compose.as_ref().is_some_and(|c| !s.agent_sees(&c.account))
    }

    /// Run an agent's command, and note it. With an account kept from agents, what the command
    /// returns isn't given them: a command can read (`list.messages`, `account.list`…), and
    /// only the reading tools leave that account out.
    fn agent_run(&mut self, command: &str, params: &Value) -> Result<Value, String> {
        let ran = self.session.run(command, params).map_err(|e| e.to_string());
        self.agent_log(command.to_string(), ran.as_ref().map_or_else(|e| AgentOutcome::Failed(e.clone()), |_| AgentOutcome::Ran));
        if self.session.prefs.agents_hidden.is_empty() {
            ran
        } else {
            ran.map(|_| json!({"ran": true, "result": "not given while an account is kept from AI agents"}))
        }
    }

    /// The asks that may no longer run (agents let ask no more, an account kept meanwhile, the
    /// window showing one) are refused, each said: every frame, before the user is asked.
    pub(crate) fn agents_recheck(&mut self) {
        let asks = std::mem::take(&mut self.ui.agents.asks);
        for ask in asks {
            match self.agent_gate(&ask.command, &ask.params) {
                Ok(_) => self.ui.agents.asks.push(ask),
                Err(e) => {
                    self.agent_log(ask.command.clone(), AgentOutcome::Failed(e.clone()));
                    self.ui.agents.decided.push((ask.ticket, Err(e)));
                }
            }
        }
    }

    /// The ask under `ticket` taken back, unanswered: the agent cancelled it or gave up waiting,
    /// or it waited too long. Whether it was still waiting.
    pub fn agent_withdraw(&mut self, ticket: u64) -> bool {
        let before = self.ui.agents.asks.len() + self.ui.agents.settling.len();
        self.ui.agents.asks.retain(|a| a.ticket != ticket);
        self.ui.agents.settling.retain(|a| a.ticket != ticket);
        self.ui.agents.asks.len() + self.ui.agents.settling.len() != before
    }

    /// The commands awaiting a server's answer, answered once it came, or after [`SETTLE_WAIT`]
    /// with what there is: every frame, at the window's time `now`.
    pub(crate) fn agents_settle(&mut self, now: f64) {
        for mut w in std::mem::take(&mut self.ui.agents.settling) {
            let since = *w.since.get_or_insert(now);
            let mut asked = w.params.clone();
            if let Some(o) = asked.as_object_mut() {
                o.insert("refresh".into(), json!(false));
            }
            let read = self.session.run(&w.command, &asked).map_err(|e| e.to_string());
            let waiting = read.as_ref().is_ok_and(|v| v.get("pending").and_then(Value::as_bool) == Some(true));
            if waiting && now - since < SETTLE_WAIT {
                self.ui.agents.settling.push(w);
                continue;
            }
            let answer = if waiting {
                Err(format!("`{}`: the server didn't answer within a minute; ask again with \"refresh\": false later", w.command))
            } else if self.session.prefs.agents_hidden.is_empty() {
                read
            } else {
                read.map(|_| json!({"ran": true, "result": "not given while an account is kept from AI agents"}))
            };
            self.ui.agents.decided.push((w.ticket, answer));
        }
    }

    /// Whether answers are awaited from servers for agents.
    pub fn agents_settling(&self) -> bool {
        !self.ui.agents.settling.is_empty()
    }

    /// The user's answer to the command waiting under `ticket`: `allow` (and, with `always`, that
    /// command from then on), or not.
    pub fn agent_decide(&mut self, ticket: u64, allow: bool, always: bool) {
        let Some(i) = self.ui.agents.asks.iter().position(|a| a.ticket == ticket) else { return };
        let ask = self.ui.agents.asks.remove(i);
        let answer = if allow {
            // As things are now, not as they were when it asked.
            match self.agent_gate(&ask.command, &ask.params) {
                Ok(_) => {
                    if always {
                        let _ = self.session.run("options.agentsAlways", &json!({"allow": ask.command}));
                    }
                    let ran = self.agent_run(&ask.command, &ask.params);
                    if awaits_server(&ask.command, &ran) {
                        self.ui.agents.settling.push(Settling { ticket, command: ask.command.clone(), params: ask.params.clone(), since: None });
                        return;
                    }
                    ran
                }
                Err(e) => {
                    self.agent_log(ask.command.clone(), AgentOutcome::Failed(e.clone()));
                    Err(e)
                }
            }
        } else {
            self.agent_log(ask.command.clone(), AgentOutcome::Denied);
            Err(format!("the user didn't allow `{}`", ask.command))
        };
        self.ui.agents.decided.push((ticket, answer));
    }

    /// The answers the user gave since the last call, each under its ticket.
    pub fn agent_decisions(&mut self) -> Vec<(u64, Result<Value, String>)> {
        std::mem::take(&mut self.ui.agents.decided)
    }

    /// Whether commands agents asked to run are waiting for the user.
    pub fn agents_asking(&self) -> bool {
        !self.ui.agents.asks.is_empty()
    }

    /// Everything waiting is refused: agents were turned off, or the channel closed.
    pub fn agents_refuse_all(&mut self, why: &str) {
        for ask in std::mem::take(&mut self.ui.agents.asks) {
            self.ui.agents.decided.push((ask.ticket, Err(why.to_string())));
        }
    }

    /// Note what an agent asked, and what came of it. The tool or command is the agent's own
    /// text, so it's cut short, as is why it failed.
    fn agent_log(&mut self, what: String, outcome: AgentOutcome) {
        let outcome = match outcome {
            AgentOutcome::Failed(e) => AgentOutcome::Failed(e.chars().take(200).collect()),
            o => o,
        };
        let what = what.chars().take(80).collect();
        self.ui.agents.log.push_back(AgentLogLine { at: self.session.clock.now, what, outcome });
        while self.ui.agents.log.len() > LOG_KEPT {
            self.ui.agents.log.pop_front();
        }
    }

    /// What agents asked lately, the newest last.
    pub fn agents_log(&self) -> impl Iterator<Item = &AgentLogLine> {
        self.ui.agents.log.iter()
    }

    /// The commands there are (those whose id, label or place has `search`), as the CLI's
    /// `commands` lists them, with whether each can run now and whether agents run it without
    /// asking.
    fn agent_commands(&self, search: &str) -> Value {
        let s = search.trim().to_lowercase();
        let listed: Vec<Value> = self
            .session
            .registry()
            .all()
            .iter()
            .filter(|c| !c.id.starts_with("options.agents"))
            .filter(|c| s.is_empty() || [c.id, c.label, c.location].iter().any(|x| x.to_lowercase().contains(&s)))
            .map(|c| {
                let why = (c.enabled)(&self.session);
                let mut d = json!({
                    "id": c.id,
                    "label": c.label,
                    "location": c.location,
                    "params": c.params,
                    "enabled": why.is_none(),
                    "withoutAsking": self.session.prefs.agents_always.iter().any(|x| x == c.id),
                });
                // Why not now, and (issue #255: "enabled: false unless something is selected")
                // that the messages a command acts on can be named instead.
                if let Some(why) = why {
                    d["why"] = json!(why);
                }
                if c.on_selection {
                    d["keys"] = json!("the messages it acts on, in the list shown: [key]; else those selected");
                }
                d
            })
            .collect();
        json!(listed)
    }

    /// What the window shows: where it is, what's open, what's waiting. What's of an account kept
    /// from agents is left out.
    fn agent_inspect(&self) -> Value {
        let s = &self.session;
        json!({
            "module": s.module,
            "view": match &s.view {
                mailcraft_engine::View::Folder { account, .. } if !s.agent_sees(account) => Value::Null,
                mailcraft_engine::View::Folder { account, folder } => json!({"account": account, "folder": folder}),
                other => json!(format!("{other:?}")),
            },
            "search": s.search.as_ref().map(|(q, _)| q.clone()),
            "listed": s.list.iter().filter(|m| s.agent_sees(&m.key.account)).count(),
            "selected": s.selection.iter().filter(|k| s.agent_sees(&k.account)).take(100).map(|k| k.to_id()).collect::<Vec<_>>(),
            "reading": s.reading.as_ref().filter(|r| s.agent_sees(&r.key.account)).map(|r| json!({"key": r.key.to_id(), "subject": r.parsed.as_ref().map(|m| m.subject.clone())})),
            "writing": s.compose.as_ref().filter(|c| s.agent_sees(&c.account)).map(|c| json!({"to": c.to, "subject": c.subject})),
            "dialog": self.ui.dialog.as_ref().map(|d| format!("{d:?}").split(['(', ' ', '{']).next().unwrap_or_default().to_string()),
            "accounts": s.accounts.iter().map(|a| a.config.id.clone()).filter(|a| s.agent_sees(a)).collect::<Vec<_>>(),
            "waitingForUser": self.ui.agents.asks.iter().map(|a| a.command.clone()).collect::<Vec<_>>(),
        })
    }

    /// The commands agents asked to run, waiting for the user: a small window above the rest.
    pub(crate) fn agents_asking_window(&mut self, ctx: &egui::Context) {
        if self.agents_settling() {
            self.agents_settle(ctx.input(|i| i.time));
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
        self.agents_recheck();
        let Some(ask) = self.ui.agents.asks.first().cloned() else { return };
        // Easy to miss behind the agent's own window (issue #255: calls timed out waiting): each
        // ask, as it comes up, brings MailCraft forward and is said in a notification.
        if ask.ticket != self.ui.agents.announced {
            self.ui.agents.announced = ask.ticket;
            if self.ui.in_tray {
                self.show_window(ctx);
            } else {
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(egui::UserAttentionType::Critical));
            let mut body = tf!("An AI agent wants to: {command}", command = tl!(ask.label));
            for line in ask.about.iter().take(3) {
                body.push('\n');
                body.push_str(line);
            }
            self.services.notify(tl!("An AI agent asks"), &body);
        }
        let t = self.ui.tokens;
        let mut decided = None;
        egui::Window::new(tl!("An AI agent asks"))
            .id(egui::Id::unique("agents-asking"))
            .collapsible(false)
            .resizable(false)
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-16.0, -40.0))
            .show(ctx, |ui| {
                ui.set_max_width(420.0);
                ui.label(egui::RichText::new(tf!("An AI agent wants to: {command}", command = tl!(ask.label))).strong());
                // What it's about, in words; the command and its params as sent, folded away.
                for line in &ask.about {
                    ui.add(egui::Label::new(egui::RichText::new(line.as_str()).color(t.text)).wrap());
                }
                egui::CollapsingHeader::new(tl!("Details")).id_salt(("agent-ask", ask.ticket)).default_open(ask.about.is_empty()).show(ui, |ui| {
                    ui.label(egui::RichText::new(ask.command.as_str()).monospace().color(t.dim));
                    if ask.params.as_object().is_some_and(|o| !o.is_empty()) {
                        egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                            ui.label(egui::RichText::new(ask.shown.as_str()).monospace().small());
                        });
                    }
                });
                if self.ui.agents.asks.len() > 1 {
                    ui.label(egui::RichText::new(tf!("{n} more waiting", n = self.ui.agents.asks.len() - 1)).color(t.dim).small());
                }
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button(tl!("Allow")).clicked() {
                        decided = Some((true, false));
                    }
                    if ui.button(tl!("Always allow")).on_hover_text(tl!("Agents run this command from now on without asking")).clicked() {
                        decided = Some((true, true));
                    }
                    if ui.button(tl!("Deny")).clicked() {
                        decided = Some((false, false));
                    }
                });
            });
        if let Some((allow, always)) = decided {
            self.agent_decide(ask.ticket, allow, always);
        }
    }
}

impl MailApp {
    /// What a command's params name, in words (issue #255: only the JSON was shown, a meeting
    /// as a uid and its occurrence in Unix seconds): the event or task, the occurrence, the
    /// messages, whom to, the account. Only what's found here: the JSON says the rest.
    fn described(&self, params: &Value) -> Vec<String> {
        let s = &self.session;
        let text = |k: &str| params.get(k).and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty());
        let when = |t: i64| crate::i18n::date(&mailcraft_engine::model::dates::full_time(t, &s.clock));
        let mut out = Vec::new();
        if let Some(uid) = text("uid") {
            if let Some((_, e)) = s.find_event(uid) {
                out.push(tf!("Event: “{name}”, {when}", name = e.event.summary, when = when(e.event.start)));
            } else if let Some(t) = s.find_task(uid) {
                out.push(tf!("Task: “{name}”", name = t.task.title));
            }
        }
        if let Some(at) = params.get("occurrence").and_then(Value::as_i64) {
            out.push(tf!("This occurrence: {when}", when = when(at)));
        }
        let keys = params.get("keys").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str);
        for k in text("key").into_iter().chain(keys).take(5) {
            if let Some(m) = mailcraft_engine::model::MessageKey::from_id(k).and_then(|k| s.summary(&k)) {
                let from = if m.from.name.is_empty() { m.from.email.as_str() } else { m.from.name.as_str() };
                out.push(tf!("Message: “{subject}”, from {from}", subject = m.subject, from = from));
            }
        }
        if let Some(to) = text("to") {
            out.push(tf!("To: {who}", who = to));
        }
        if let Some(a) = text("account").and_then(|a| s.account(a)) {
            out.push(tf!("Account: {email}", email = a.config.email));
        }
        out
    }
}

/// Whether a command's answer is a server's, still to come ([`SETTLES`]).
fn awaits_server(command: &str, ran: &Result<Value, String>) -> bool {
    SETTLES.contains(&command) && ran.as_ref().is_ok_and(|v| v.get("pending").and_then(Value::as_bool) == Some(true))
}

/// A command's params as the asking window shows them: every one, as JSON, up to
/// [`PARAMS_SHOWN`] characters, then how many more there are (never cut without a word).
fn shown_params(params: &Value) -> String {
    let all = serde_json::to_string_pretty(params).unwrap_or_default();
    let total = all.chars().count();
    if total <= PARAMS_SHOWN {
        return all;
    }
    let mut shown: String = all.chars().take(PARAMS_SHOWN).collect();
    shown.push('\n');
    shown.push_str(&tf!("… and {n} more characters", n = total - PARAMS_SHOWN));
    shown
}
