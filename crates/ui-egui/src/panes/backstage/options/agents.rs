//! Options' AI agents card (issue #255): whether agents may use MailCraft through its MCP server,
//! how they connect, and the commands they run without asking. What they asked lately is in File
//! › Info ([`log`]).

use super::*;

pub(super) fn agents(s: &Session, t: &Tokens, ui: &mut Ui, acts: &mut Vec<Action>) {
    card(ui, t, tl!("AI agents"), None, |ui| {
        let about = tl!(
            "An AI assistant on this computer (Claude Desktop, Claude Code…) reads your mail, calendar, tasks and people through MailCraft's MCP server, and asks here before each command it wants to run. Off, it reaches nothing."
        );
        switch_row(
            s,
            ui,
            t,
            true,
            Switch { label: tl!("Let AI agents use MailCraft"), about: Some(about), on: s.prefs.agents, cmd: "options.agents" },
            acts,
        );
        if s.prefs.agents {
            setting(
                ui,
                t,
                false,
                tl!("Connect an assistant"),
                Some(tl!("Add MailCraft's MCP server to it: the command mailcraft-cli with the argument mcp")),
                |_| {},
            );
            let about =
                tl!("Sending, moving, answering an invitation… each asked about here first, unless you always allow it. Off, agents only read.");
            switch_row(
                s,
                ui,
                t,
                false,
                Switch { label: tl!("Let AI agents ask to run commands"), about: Some(about), on: s.prefs.agents_run, cmd: "options.agentsRun" },
                acts,
            );
            // Each account, read by agents or kept from them.
            if s.accounts.len() > 1 || !s.prefs.agents_hidden.is_empty() {
                setting(
                    ui,
                    t,
                    false,
                    tl!("Accounts AI agents read"),
                    Some(tl!("One turned off is left out of all they read, and they can't take pictures of the window")),
                    |_| {},
                );
                for a in &s.accounts {
                    let id = a.config.id.as_str();
                    let name = if a.config.email.is_empty() { a.config.name.as_str() } else { a.config.email.as_str() };
                    setting_by(s, "options.agentsAccount", ui, t, false, name, None, |ui| {
                        let mut on = s.agent_sees(id);
                        if widgets::switch(ui, t, &mut on, name).changed() {
                            acts.push(Action::Run("options.agentsAccount", json!({"account": id, "on": on})));
                        }
                    });
                }
            }
        }
        if !s.prefs.agents_always.is_empty() {
            setting(ui, t, false, tl!("Run without asking"), Some(tl!("Commands agents run without asking first")), |_| {});
            // Each one by its name, its id below it, as the commands are listed to agents.
            for id in &s.prefs.agents_always {
                let label = s.registry().get(id).map(|c| tl!(c.label).to_string()).unwrap_or_else(|| id.clone());
                setting_by(s, "options.agentsAlways", ui, t, false, &label, Some(id), |ui| {
                    if ui.button(tl!("Forget")).clicked() {
                        acts.push(Action::Run("options.agentsAlways", json!({"forget": id})));
                    }
                });
            }
        }
    });
}

/// What agents asked lately, and what came of it (File › Info): the newest first.
pub(in crate::panes::backstage) fn log(s: &Session, t: &Tokens, agents: &crate::app::Agents, ui: &mut Ui) {
    let lines: Vec<&crate::AgentLogLine> = agents.log.iter().rev().collect();
    if lines.is_empty() && !s.prefs.agents {
        return;
    }
    card(ui, t, tl!("What AI agents did"), None, |ui| {
        if lines.is_empty() {
            ui.label(RichText::new(tl!("Nothing yet")).color(t.dim));
        }
        for l in lines {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(crate::calendar::hm(l.at, s)).color(t.dim));
                ui.label(RichText::new(l.what.as_str()).monospace());
                ui.label(RichText::new(l.outcome.text()).color(t.dim));
            });
        }
    });
}
