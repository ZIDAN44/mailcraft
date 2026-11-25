//! UI tests: AI agents in the open window (issue #255). Off until the user lets them in; then
//! reading answers at once, and a command waits for the user in a small window of its own (a
//! dialog being filled in stays): Allow, Always allow, or Deny. What they did is in File › Info,
//! the setting and the commands run without asking in File › Options.

use super::*;
use crate::AgentAnswer;

#[test]
fn agents_wait_for_the_user_before_running_a_command() {
    let (mut a, _) = app("agents-ask");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    frame(&ctx, &mut a, vec![]);
    // Off: nothing reaches MailCraft.
    assert_eq!(a.agent_request("agents.read", &json!({"tool": "list_accounts"})), AgentAnswer::Now(Err(crate::AGENTS_OFF.into())));
    a.run("options.agents", json!({"on": true}));
    let AgentAnswer::Now(Ok(found)) = a.agent_request("agents.read", &json!({"tool": "search_mail", "args": {"query": "from:ana"}})) else {
        panic!("read at once")
    };
    assert!(found["found"].as_u64().unwrap() > 0, "{found}");
    let AgentAnswer::Now(Ok(view)) = a.agent_request("ui.inspect", &json!({})) else { panic!("inspected") };
    assert_eq!(view["view"]["folder"], "INBOX", "{view}");
    assert!(a.agent_may_picture().is_ok());
    // Read only until the user lets agents ask to run commands: refused at once, nothing asked.
    let AgentAnswer::Now(Err(e)) = a.agent_request("engine.execute", &json!({"command": "view.calendar"})) else { panic!("refused") };
    assert!(e.contains("read only") && !a.agents_asking(), "{e}");
    a.run("options.agentsRun", json!({"on": true}));
    // A command, with a dialog open: asked about beside it, the dialog left as it was.
    a.ui.dialog = Some(Dialog::new_event(&a.session, Some(NOW + 86_400)));
    let AgentAnswer::Asking(ticket) = a.agent_request("engine.execute", &json!({"command": "view.calendar"})) else { panic!("asked") };
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    let texts = drawn_strings(&ctx, &mut a, egui::vec2(1280.0, 900.0));
    assert!(texts.iter().any(|t| t == "An AI agent asks"), "{texts:?}");
    assert!(matches!(a.ui.dialog, Some(Dialog::Event(_))), "the form stays");
    assert_eq!(a.session.module, mailcraft_engine::Module::Mail, "not run before the user allowed it");
    click_named(&ctx, &mut a, "Allow");
    frame(&ctx, &mut a, vec![]);
    let decided = a.agent_decisions();
    assert!(matches!(decided.as_slice(), [(t, Ok(_))] if *t == ticket), "{decided:?}");
    assert_eq!(a.session.module, mailcraft_engine::Module::Calendar);
    // Denied: not run, and said so.
    let AgentAnswer::Asking(_) = a.agent_request("engine.execute", &json!({"command": "view.mail"})) else { panic!("asked") };
    frame(&ctx, &mut a, vec![]);
    click_named(&ctx, &mut a, "Deny");
    frame(&ctx, &mut a, vec![]);
    let decided = a.agent_decisions();
    assert!(matches!(decided.as_slice(), [(_, Err(e))] if e.contains("didn't allow")), "{decided:?}");
    assert_eq!(a.session.module, mailcraft_engine::Module::Calendar);
    // Always allowed: run at once from then on.
    let AgentAnswer::Asking(_) = a.agent_request("engine.execute", &json!({"command": "view.mail"})) else { panic!("asked") };
    frame(&ctx, &mut a, vec![]);
    click_named(&ctx, &mut a, "Always allow");
    frame(&ctx, &mut a, vec![]);
    assert!(a.agent_decisions().first().is_some_and(|(_, r)| r.is_ok()));
    assert_eq!(a.session.prefs.agents_always, ["view.mail"]);
    a.run("view.calendar", json!({}));
    assert!(matches!(a.agent_request("engine.execute", &json!({"command": "view.mail"})), AgentAnswer::Now(Ok(_))));
    assert_eq!(a.session.module, mailcraft_engine::Module::Mail);
    // Their own settings, a command there isn't, params that aren't an object: refused at once.
    for (cmd, params) in [
        ("options.agents", json!({"on": false})),
        ("options.agentsAlways", json!({"allow": "mail.send"})),
        ("no.such", json!({})),
        ("view.mail", json!(3)),
    ] {
        let answer = a.agent_request("engine.execute", &json!({"command": cmd, "params": params}));
        assert!(matches!(answer, AgentAnswer::Now(Err(_))), "{cmd}: {answer:?}");
    }
    assert!(a.session.prefs.agents);
    // The sample's account kept from agents: what the window shows of it isn't told, and no
    // picture is taken (the window could show it).
    a.run("folder.open", json!({"account": "sample", "folder": "INBOX"}));
    a.run("options.agentsAccount", json!({"account": "sample", "on": false}));
    let AgentAnswer::Now(Ok(view)) = a.agent_request("ui.inspect", &json!({})) else { panic!("inspected") };
    assert!(view["view"].is_null() && view["listed"] == 0 && view["accounts"].as_array().is_some_and(Vec::is_empty), "{view}");
    assert!(a.agent_may_picture().unwrap_err().contains("kept from AI agents"));
    a.run("options.agentsAccount", json!({"account": "sample", "on": true}));
    // Turned off: whatever waits is refused.
    let AgentAnswer::Asking(_) = a.agent_request("engine.execute", &json!({"command": "view.calendar"})) else { panic!("asked") };
    a.agents_refuse_all(crate::AGENTS_OFF);
    assert!(!a.agents_asking() && a.agent_decisions().first().is_some_and(|(_, r)| r.is_err()));
}

#[test]
fn what_agents_did_is_in_file_info_and_the_setting_in_options() {
    let (mut a, _) = app("agents-pages");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    a.run("options.agents", json!({"on": true}));
    let _ = a.agent_request("agents.read", &json!({"tool": "find_people", "args": {"query": "zo"}}));
    // What the window shows is a read too, and noted as one.
    let _ = a.agent_request("ui.inspect", &json!({}));
    let _ = a.agent_request("agents.read", &json!({"tool": "no_such_tool"}));
    a.ui.backstage = Some(crate::backstage::Backstage::new(crate::backstage::Page::Info));
    for _ in 0..2 {
        frame(&ctx, &mut a, vec![]);
    }
    let texts = drawn_strings(&ctx, &mut a, egui::vec2(1280.0, 4000.0));
    for want in ["What AI agents did", "find_people", "inspect", "Looked, changed nothing", "no_such_tool"] {
        assert!(texts.iter().any(|t| t == want), "{want}: {texts:?}");
    }
    assert!(texts.iter().any(|t| t.starts_with("Didn't run: ")), "{texts:?}");
    // The commands run without asking: each by its name, its id under it, and Forget (in a
    // window tall enough for the end of Options).
    let _tall = super::windows::layout::Dressed::new(egui::vec2(1280.0, 8000.0), "");
    a.run("options.agentsAlways", json!({"allow": "view.calendar"}));
    a.ui.backstage = Some(crate::backstage::Backstage::new(crate::backstage::Page::Options));
    for _ in 0..2 {
        frame(&ctx, &mut a, vec![]);
    }
    let texts = drawn_strings(&ctx, &mut a, egui::vec2(1280.0, 8000.0));
    for want in ["Let AI agents use MailCraft", "Let AI agents ask to run commands", "Run without asking", "Calendar", "view.calendar", "Forget"] {
        assert!(texts.iter().any(|t| t == want), "{want}: {texts:?}");
    }
    click_named(&ctx, &mut a, "Forget");
    frame(&ctx, &mut a, vec![]);
    assert!(a.session.prefs.agents_always.is_empty());
    // With a second account, each has its switch: turned off here, kept from agents.
    assert!(!texts.iter().any(|t| t == "Accounts AI agents read"), "one account: nothing to choose");
    a.run("account.add", json!({"email": "riley@other.example", "password": "pw"}));
    for _ in 0..2 {
        frame(&ctx, &mut a, vec![]);
    }
    let texts = drawn_strings(&ctx, &mut a, egui::vec2(1280.0, 8000.0));
    assert!(texts.iter().any(|t| t == "Accounts AI agents read") && texts.iter().any(|t| t == "riley@other.example"), "{texts:?}");
    // Its switch (Options has other settings by account, under its name too).
    let nodes = super::named(&ctx, &mut a, egui::vec2(1280.0, 8000.0));
    let mut switch = nodes.iter().filter(|n| n.label.as_deref() == Some("riley@other.example") && n.role == egui::accesskit::Role::CheckBox);
    let at = switch.find_map(super::Named::centre).unwrap_or_else(|| panic!("{nodes:?}"));
    super::click(&ctx, &mut a, at);
    frame(&ctx, &mut a, vec![]);
    assert_eq!(a.session.prefs.agents_hidden.len(), 1);
    assert!(!a.session.agent_sees(&a.session.prefs.agents_hidden[0]) && a.session.agent_sees("sample"));
}

/// A command asked about runs only as things are when the user answers: not once agents may no
/// longer ask, nor once the agent took it back. With an account kept from agents, none runs
/// while the window shows its mail, and what one returns isn't given them.
#[test]
fn what_agents_asked_runs_only_as_things_are_now() {
    let (mut a, _) = app("agents-stale");
    let ctx = egui::Context::default();
    a.run("options.agents", json!({"on": true}));
    a.run("options.agentsRun", json!({"on": true}));
    let asked = |a: &mut MailApp, cmd: &str| match a.agent_request("engine.execute", &json!({"command": cmd})) {
        AgentAnswer::Asking(t) => t,
        other => panic!("{cmd}: {other:?}"),
    };
    // Agents may ask no more: what waits is refused at the next frame, not run on Allow.
    let t = asked(&mut a, "view.calendar");
    a.run("options.agentsRun", json!({"on": false}));
    frame(&ctx, &mut a, vec![]);
    a.agent_decide(t, true, false);
    let decided = a.agent_decisions();
    assert!(matches!(decided.as_slice(), [(x, Err(e))] if *x == t && e.contains("read only")), "{decided:?}");
    assert_eq!(a.session.module, mailcraft_engine::Module::Mail);
    // Taken back by the agent: gone, unanswered, never run.
    a.run("options.agentsRun", json!({"on": true}));
    let t = asked(&mut a, "view.calendar");
    assert!(a.agent_withdraw(t) && !a.agents_asking());
    a.agent_decide(t, true, false);
    assert!(a.agent_decisions().is_empty());
    assert_eq!(a.session.module, mailcraft_engine::Module::Mail);
    // Long params are shown whole, up to a point, then said cut.
    let long = "x".repeat(30_000);
    let AgentAnswer::Asking(t) =
        a.agent_request("engine.execute", &json!({"command": "view.mail", "params": {"body": long, "to": "x@evil.example"}}))
    else {
        panic!("asked")
    };
    let shown = &a.ui.agents.asks.iter().find(|x| x.ticket == t).unwrap().shown;
    assert!(shown.ends_with("more characters") && shown.chars().count() < 21_000, "{}", shown.len());
    a.agent_withdraw(t);

    // The sample's account kept: no command while its Inbox is shown, the Inbox of another
    // account fine, and nothing a command returns given.
    let other =
        a.session.run("account.add", &json!({"email": "riley@other.example", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    a.run("options.agentsAccount", json!({"account": "sample", "on": false}));
    a.run("folder.open", json!({"account": "sample", "folder": "INBOX"}));
    let AgentAnswer::Now(Err(e)) = a.agent_request("engine.execute", &json!({"command": "view.mail"})) else { panic!("refused") };
    assert!(e.contains("window shows an account kept"), "{e}");
    a.session.open_view(mailcraft_engine::View::Folder { account: other.clone(), folder: "INBOX".into() });
    a.run("options.agentsAlways", json!({"allow": "account.list"}));
    let AgentAnswer::Now(Ok(v)) = a.agent_request("engine.execute", &json!({"command": "account.list"})) else { panic!("ran") };
    assert_eq!(v["ran"], true);
    assert!(!v.to_string().contains("studio.example"), "{v}");
}

/// Issue #255, as the tester met it: the question said in words (the event and its occurrence,
/// whom to), the JSON folded away; a notification as it comes up (the window was easy to miss);
/// and a command that asks a server answered with the server's answer, not `pending`.
#[test]
fn an_ask_reads_in_words_is_announced_and_a_servers_answer_is_awaited() {
    let (mut a, _) = app("agents-words");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let shown = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    a.services = Box::new(Recorder(shown.clone()));
    a.run("options.agents", json!({"on": true}));
    a.run("options.agentsRun", json!({"on": true}));
    let p = json!({"summary": "Delegation check", "start": "2026-10-22T14:00", "attendees": "Zoe Park <zoe@studio.example>"});
    let uid = a.session.run("event.create", &p).unwrap()["uid"].as_str().unwrap().to_string();
    let at = a.session.find_event(&uid).unwrap().1.event.start;
    let params = json!({"uid": uid, "occurrence": at, "to": "Jane Doe <jane@studio.example>"});
    let AgentAnswer::Asking(ticket) = a.agent_request("engine.execute", &json!({"command": "invite.forward", "params": params})) else {
        panic!("asked")
    };
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    let texts = drawn_strings(&ctx, &mut a, egui::vec2(1280.0, 900.0));
    assert!(texts.iter().any(|t| t.starts_with("Event: “Delegation check”, Thu 2026-10-22")), "{texts:?}");
    assert!(texts.iter().any(|t| t == "To: Jane Doe <jane@studio.example>"), "{texts:?}");
    assert!(texts.iter().any(|t| t == "Details") && !texts.iter().any(|t| t.contains("\"occurrence\"")), "the JSON folded: {texts:?}");
    let said = shown.lock().unwrap().clone();
    assert!(matches!(said.as_slice(), [(title, body)] if title == "An AI agent asks" && body.contains("Delegation check")), "{said:?}");
    frame(&ctx, &mut a, vec![]);
    assert_eq!(shown.lock().unwrap().len(), 1, "said once");
    a.agent_withdraw(ticket);

    // A server's answer awaited: given once it came, under the same ticket.
    let id = a.session.run("account.add", &json!({"email": "ann@mc31.test", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    a.session.take_jobs();
    a.run("options.agentsAlways", json!({"allow": "rules.server"}));
    let AgentAnswer::Asking(ticket) = a.agent_request("engine.execute", &json!({"command": "rules.server", "params": {"account": id}})) else {
        panic!("awaited")
    };
    frame(&ctx, &mut a, vec![]);
    assert!(a.agent_decisions().is_empty(), "not yet");
    let rules = mailcraft_engine::model::organize::ServerRules { other_script: Some("sogo".into()), ..Default::default() };
    a.session.apply(mailcraft_engine::model::Event::ServerRules { account: id, rules: Some(rules), error: None });
    frame(&ctx, &mut a, vec![]);
    let decided = a.agent_decisions();
    assert!(matches!(decided.as_slice(), [(t, Ok(v))] if *t == ticket && v["rules"]["otherScript"] == "sogo"), "{decided:?}");
}
