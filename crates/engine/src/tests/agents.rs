//! Issue #255: what AI agents read here (`crate::agents`), from the sample mailbox. Nothing of it
//! changes anything: the window's folder, list, search and reading pane stay, no message is
//! marked read, and nothing is asked of a server.

use super::*;

/// What the window shows, and whether each Inbox message is read: what reading must not change.
fn shown(s: &mut Session) -> (Option<(String, SearchScope)>, Vec<String>, Vec<mailcraft_model::MessageKey>, Option<String>, Vec<bool>) {
    let list = s.list.iter().map(|m| m.key.to_id()).collect();
    let seen = s.folder_state("sample", "INBOX").messages.iter().map(|m| m.flags.seen).collect();
    (s.search.clone(), list, s.selection.clone(), s.reading.as_ref().map(|r| r.key.to_id()), seen)
}

#[test]
fn agents_read_without_changing_anything() {
    let mut s = sample_session("agents-read");
    s.run("folder.open", &json!({"account": "sample", "folder": "Projects"})).unwrap();
    let before = shown(&mut s);
    s.take_jobs();

    let accounts = s.agent_read("list_accounts", &json!({})).unwrap();
    let folders = accounts[0]["folders"].as_array().unwrap();
    assert!(folders.iter().any(|f| f["path"] == "INBOX" && f["role"] == "inbox"), "{accounts}");

    let inbox = s.agent_read("list_messages", &json!({"limit": 3})).unwrap();
    assert_eq!((inbox["folder"].as_str(), inbox["messages"].as_array().map(Vec::len)), (Some("INBOX"), Some(3)), "{inbox}");
    let dates: Vec<i64> = inbox["messages"].as_array().unwrap().iter().map(|m| m["date"].as_i64().unwrap()).collect();
    assert!(dates.windows(2).all(|w| w[0] >= w[1]), "newest first: {dates:?}");

    // Found by its sender, and by a word only its text has.
    let by_sender = s.agent_read("search_mail", &json!({"query": "from:ana"})).unwrap();
    assert!(by_sender["messages"].as_array().unwrap().iter().any(|m| m["subject"] == "Sound mix, version 3"), "{by_sender}");
    let by_text = s.agent_read("search_mail", &json!({"query": "ambience"})).unwrap();
    let [hit] = by_text["messages"].as_array().unwrap().as_slice() else { panic!("{by_text}") };
    assert!(by_text["textsRead"].as_u64().unwrap() > 0, "{by_text}");
    let key = hit["key"].as_str().unwrap().to_string();
    assert_eq!(hit["unread"], true);

    let message = s.agent_read("read_message", &json!({"key": key})).unwrap();
    assert!(message["text"].as_str().unwrap().contains("rain ambience"), "{message}");
    assert_eq!((message["downloaded"].as_bool(), message["attachments"][0]["name"].as_str()), (Some(true), Some("invite.ics")));
    assert_eq!(message["unread"], true, "still unread");

    let people = s.agent_read("find_people", &json!({"query": "zo"})).unwrap();
    assert_eq!(people[0]["email"], "zoe@studio.example", "{people}");
    let zoe = s.agent_read("everything_with", &json!({"email": "zoe@studio.example"})).unwrap();
    assert!(zoe["messages"].as_u64().unwrap() > 0 && !zoe["conversations"].as_array().unwrap().is_empty(), "{zoe}");

    let tasks = s.agent_read("list_tasks", &json!({})).unwrap();
    assert!(tasks.as_array().unwrap().iter().any(|t| t["title"] == "Book the sound stage"), "{tasks}");
    let day = |t: i64| {
        let (y, m, d) = mailcraft_model::dates::civil_from_days(s.clock.local_day(t));
        format!("{y:04}-{m:02}-{d:02}")
    };
    let (from, to) = (day(NOW), day(NOW + 3 * 86_400));
    let events = s.agent_read("list_events", &json!({"from": from, "to": to})).unwrap();
    let mix = events["events"].as_array().unwrap().iter().find(|e| e["title"] == "Sound mix playback").expect("the sample's meeting");
    assert!(mix["guests"].as_array().is_some_and(|g| !g.is_empty()) && mix["yourAnswer"].is_string(), "{mix}");

    assert_eq!(shown(&mut s), before, "the window as it was, the mail as unread as it was");
    assert!(s.take_jobs().is_empty(), "nothing asked of a server");
}

#[test]
fn what_agents_ask_wrongly_is_said() {
    let mut s = sample_session("agents-wrong");
    for (tool, args) in [
        ("no_such_tool", json!({})),
        ("read_message", json!({"key": "not a key"})),
        ("read_message", json!({})),
        ("search_mail", json!({"query": "  "})),
        ("search_mail", json!({"query": "x", "folder": "INBOX"})),
        ("list_messages", json!({"account": "nobody"})),
        ("list_messages", json!({"folder": "No such folder"})),
        ("list_events", json!({"from": "yesterday"})),
        ("list_events", json!({"from": "2026-12-01", "to": "2026-11-01"})),
        ("list_events", json!({"from": "2026-01-01", "to": "2027-06-01"})),
        ("everything_with", json!({"email": "not an address"})),
    ] {
        assert!(s.agent_read(tool, &args).is_err(), "{tool} {args}");
    }
    // Numbers past the most are the most; junk where a number goes is the default.
    let r = s.agent_read("list_messages", &json!({"limit": 1_000_000, "offset": -4})).unwrap();
    assert!(r["messages"].as_array().unwrap().len() <= crate::agents::MESSAGES_MAX);
    // Every tool listed, read only, with a schema of its arguments.
    let tools = crate::agents::agent_tool_list();
    assert_eq!(tools.len(), crate::agents::AGENT_TOOLS.len());
    assert!(tools.iter().all(|t| t["annotations"]["readOnlyHint"] == true && t["inputSchema"]["type"] == "object"));
}

/// Looking someone up asks no organisation's directory (an Exchange account's): what agents read
/// is asked of no server, whatever they look for.
#[test]
fn agents_ask_no_directory_for_people() {
    let mut s = sample_session("agents-directory");
    s.run("account.add", &json!({"email": "riley@contoso.example", "password": "pw", "username": "CONTOSO\\riley", "ews": "mail.contoso.example"}))
        .unwrap();
    s.take_jobs();
    for q in ["zoe", "Morgan Lee", "ana@soundlab.example"] {
        s.agent_read("find_people", &json!({"query": q})).unwrap();
    }
    let asked: Vec<String> = s.take_jobs().iter().map(|j| format!("{j:?}")).filter(|j| j.contains("SearchDirectory")).collect();
    assert!(asked.is_empty(), "{asked:?}");
    // The window's own suggestions still ask it, as they did.
    s.suggest_contacts("Morgan Lee", 10);
    assert!(s.take_jobs().iter().any(|j| format!("{j:?}").contains("SearchDirectory")));
}

/// An account kept from agents (File › Options › AI agents) is left out of all they read, as if
/// it weren't there, and a command naming it isn't run. Commands at all only once the user lets
/// agents ask to run them: until then they read.
#[test]
fn an_account_kept_from_agents_is_left_out_of_all_they_read() {
    let mut s = sample_session("agents-kept");
    let other = s.run("account.add", &json!({"email": "riley@other.example", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    s.take_jobs();
    let inbox = s.agent_read("list_messages", &json!({"account": "sample"})).unwrap();
    let key = inbox["messages"][0]["key"].as_str().unwrap().to_string();
    let day = |t: i64| {
        let (y, m, d) = mailcraft_model::dates::civil_from_days(s.clock.local_day(t));
        format!("{y:04}-{m:02}-{d:02}")
    };
    let week = json!({"from": day(NOW), "to": day(NOW + 6 * 86_400)});
    assert!(!s.agent_read("list_events", &week).unwrap()["events"].as_array().unwrap().is_empty());
    // Read only first: no command, whatever it is.
    let read_only = s.agent_may_run("view.mail", &json!({})).unwrap_err();
    assert!(read_only.contains("read only"), "{read_only}");

    s.run("options.agentsAccount", &json!({"account": "sample", "on": false})).unwrap();
    let accounts = s.agent_read("list_accounts", &json!({})).unwrap();
    let ids: Vec<&str> = accounts.as_array().unwrap().iter().filter_map(|a| a["account"].as_str()).collect();
    assert_eq!(ids, [other.as_str()]);
    assert!(s.agent_read("list_messages", &json!({"account": "sample"})).unwrap_err().contains("no account"));
    assert!(s.agent_read("search_mail", &json!({"query": "x", "account": "sample"})).is_err());
    assert!(s.agent_read("read_message", &json!({"key": key})).is_err());
    for q in ["from:ana", "ambience"] {
        assert_eq!(s.agent_read("search_mail", &json!({"query": q})).unwrap()["found"], 0, "{q}");
    }
    assert!(s.agent_read("list_events", &week).unwrap()["events"].as_array().unwrap().is_empty());
    assert!(s.agent_read("list_tasks", &json!({"done": true})).unwrap().as_array().unwrap().is_empty());
    let people = s.agent_read("find_people", &json!({"query": "zo"})).unwrap();
    assert!(!people.to_string().contains("studio.example"), "{people}");
    let zoe = s.agent_read("everything_with", &json!({"email": "zoe@studio.example"})).unwrap();
    assert_eq!(zoe["messages"], 0, "{zoe}");
    for part in ["conversations", "files", "links", "meetings", "people"] {
        assert!(zoe[part].as_array().unwrap().is_empty(), "{part}: {zoe}");
    }

    // Let ask to run commands: one naming the account kept, anywhere in its params, isn't.
    s.run("options.agentsRun", &json!({"on": true})).unwrap();
    assert!(s.agent_may_run("view.mail", &json!({})).is_ok());
    for params in [json!({"keys": [key]}), json!({"account": "sample"}), json!({"deep": [{"x": {"key": "sample:1:1:INBOX"}}]})] {
        let refused = s.agent_may_run("mail.flag", &params).unwrap_err();
        assert!(refused.contains("kept from AI agents"), "{params}: {refused}");
    }
    assert!(s.agent_may_run("mail.flag", &json!({"note": "sampled"})).is_ok(), "a word that only starts like it");
    // A reminder's key names its account before a bar.
    assert!(s.agent_may_run("reminders.dismiss", &json!({"key": "sample|standup|1795000000"})).is_err());

    // Read again once let back in; an account there isn't can't be kept.
    s.run("options.agentsAccount", &json!({"account": "sample"})).unwrap();
    assert!(s.agent_read("search_mail", &json!({"query": "from:ana"})).unwrap()["found"].as_u64().unwrap() > 0);
    assert!(s.run("options.agentsAccount", &json!({"account": "nobody", "on": false})).is_err());
    assert!(s.prefs.agents_hidden.is_empty());
}

/// Issue #255: a command that acts on the selection takes the messages to act on (`keys`), as an
/// agent can't click: they're selected first, as clicks would. Only from the list shown; none, or
/// one not there, is refused with nothing done. The command list says which take them.
#[test]
fn a_selection_command_takes_the_messages_named() {
    let mut s = sample_session("agents-keys");
    s.run("folder.open", &json!({"account": "sample", "folder": "INBOX"})).unwrap();
    s.selection.clear();
    let keys: Vec<String> = s.list.iter().take(2).map(|m| m.key.to_id()).collect();
    let before = s.list.len();
    assert!(matches!(s.run("mail.delete", &json!({})), Err(CmdError::Disabled(_))), "nothing selected, none named");
    assert!(matches!(s.run("mail.delete", &json!({"keys": []})), Err(CmdError::Params(_))));
    assert!(matches!(s.run("mail.delete", &json!({"keys": ["sample:1:999999:INBOX"]})), Err(CmdError::Params(_))), "not in the list");
    assert_eq!(s.list.len(), before, "nothing deleted");
    s.run("mail.delete", &json!({"keys": keys})).unwrap();
    assert_eq!(s.list.len(), before - 2);
    assert!(!s.list.iter().any(|m| keys.contains(&m.key.to_id())), "those two");
    let listed = s.registry().describe();
    let said = |id: &str| listed.as_array().unwrap().iter().find(|c| c["id"] == id).map(|c| c.get("keys").is_some());
    assert_eq!((said("mail.delete"), said("mail.flag"), said("mail.new")), (Some(true), Some(true), Some(false)));
}
