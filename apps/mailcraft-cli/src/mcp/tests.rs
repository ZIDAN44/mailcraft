//! Issue #255: `mailcraft-cli mcp`, MailCraft's MCP server, on the sample mailbox with no MailCraft
//! open: the MCP lifecycle, the tools listed, the mail read, and what only the open window does
//! refused with what to do.

use super::*;

/// The sample mailbox in a data folder of its own, and its files' folder.
fn sample(name: &str) -> (PathBuf, PathBuf) {
    let root = std::env::temp_dir().join(format!("mailcraft-mcp-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let files = root.join(mailcraft_engine::store::location::FILES_FOLDER);
    let store = crate::open_store(&root, &files).unwrap();
    let c = crate::clock();
    mailcraft_engine::sample::seed_if_empty_at(&store, c.now, i64::from(c.offset)).unwrap();
    (root, files)
}

fn call(id: u64, name: &str, args: Value) -> String {
    json!({"jsonrpc": "2.0", "id": id, "method": "tools/call", "params": {"name": name, "arguments": args}}).to_string()
}

/// The text a tool's result holds, and whether it's an error.
fn text(reply: &Value) -> (String, bool) {
    let r = &reply["result"];
    (r["content"][0]["text"].as_str().unwrap_or_default().to_string(), r["isError"] == true)
}

#[test]
fn the_mcp_server_reads_the_data_folder_without_mailcraft_open() {
    let (root, files) = sample("read");
    settings(&root, json!({"agents": true}));
    let mut server = Server::new(INFO, MailTools::new(root.clone(), files));
    let mut ask = |line: String| -> Value { serde_json::from_str(&server.handle_line(&line).unwrap_or_default()).unwrap_or(Value::Null) };
    let init = ask(json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "test", "version": "1"}}}).to_string());
    assert_eq!(init["result"]["serverInfo"]["name"], "mailcraft", "{init}");
    let listed = ask(json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}).to_string());
    let tools: Vec<&str> = listed["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    for name in [
        "list_accounts",
        "search_mail",
        "read_message",
        "list_events",
        "everything_with",
        "execute_command",
        "list_commands",
        "inspect",
        "screenshot",
    ] {
        assert!(tools.contains(&name), "{name}: {tools:?}");
    }
    let (found, error) = text(&ask(call(3, "search_mail", json!({"query": "ambience"}))));
    assert!(!error && found.contains("Sound mix, version 3"), "{found}");
    assert!(!text(&ask(call(4, "list_events", json!({})))).1);
    // Only the open window runs a command, once the user allows it: said what to do.
    let (said, error) = text(&ask(call(5, "execute_command", json!({"command": "mail.flag"}))));
    assert!(error && said.contains("File › Options › AI agents"), "{said}");
    let (said, error) = text(&ask(call(6, "screenshot", json!({}))));
    assert!(error && said.contains("AI agents"), "{said}");
    assert!(text(&ask(call(7, "read_message", json!({"key": "nonsense"})))).1);
    // Arguments a tool doesn't have: refused before anything runs.
    let wrong = ask(call(8, "search_mail", json!({"query": "x", "nonsense": 1})));
    assert!(wrong.get("error").is_some() || text(&wrong).1, "{wrong}");
    drop(ask);
    let _ = std::fs::remove_dir_all(&root);
}

/// MailCraft's settings, `ui.json` in the data folder as the app writes it.
fn settings(root: &Path, prefs: Value) {
    std::fs::write(root.join("ui.json"), json!({"prefs": prefs}).to_string()).unwrap();
}

#[test]
fn agents_are_let_in_as_mailcrafts_settings_say() {
    let (root, files) = sample("consent");
    let mut tools = MailTools::new(root.clone(), files);
    let refused = |tools: &mut MailTools| tools.run("list_accounts", &json!({})).unwrap_err();
    assert_eq!(refused(&mut tools), NOT_LET_IN, "no settings: nothing read");
    settings(&root, json!({"agents": false}));
    assert_eq!(refused(&mut tools), NOT_LET_IN);
    std::fs::write(root.join("ui.json"), "not json").unwrap();
    assert_eq!(refused(&mut tools), NOT_LET_IN, "unreadable: nothing read");
    settings(&root, json!({"agents": true}));
    assert!(tools.run("list_accounts", &json!({})).is_ok());
    // Which accounts are kept from agents can't be read: refused, not taken as none kept.
    settings(&root, json!({"agents": true, "agentsHidden": 5}));
    assert!(refused(&mut tools).contains("couldn't be read"));
    // Another setting this version can't read is no reason to refuse.
    settings(&root, json!({"agents": true, "density": {"from": "the future"}}));
    assert!(tools.run("list_accounts", &json!({})).is_ok());
    let _ = std::fs::remove_dir_all(&root);
}

/// With MailCraft closed, an account the user keeps from agents stays kept: the settings are read
/// before each call.
#[test]
fn an_account_kept_from_agents_stays_kept_with_mailcraft_closed() {
    let (root, files) = sample("kept");
    settings(&root, json!({"agents": true, "agentsHidden": ["sample"]}));
    let mut tools = MailTools::new(root.clone(), files);
    assert_eq!(tools.run("list_accounts", &json!({})).unwrap(), json!([]));
    assert_eq!(tools.run("search_mail", &json!({"query": "from:ana"})).unwrap()["found"], 0);
    assert!(tools.run("list_messages", &json!({"account": "sample"})).is_err());
    settings(&root, json!({"agents": true}));
    assert!(tools.run("search_mail", &json!({"query": "from:ana"})).unwrap()["found"].as_u64().unwrap() > 0);
    let _ = std::fs::remove_dir_all(&root);
}

/// An assistant stays connected all day: between its calls, the data folder is free for
/// MailCraft to open, as it does at its start.
#[test]
fn the_data_folder_is_let_go_after_each_call() {
    let (root, files) = sample("let-go");
    settings(&root, json!({"agents": true}));
    let mut tools = MailTools::new(root.clone(), files.clone());
    assert!(tools.run("search_mail", &json!({"query": "ambience"})).unwrap()["found"].as_u64().unwrap() > 0);
    // MailCraft's store, as it opens at its start, and the account's mail in it (a data file is
    // opened as it's first read). A data file being sealed as the call ended is let go of a
    // moment later, as after any `mailcraft-cli` command; never held until the next call.
    let opens = || {
        let app = Store::packed(&root, &files).unwrap();
        app.load_folders("sample").is_ok().then_some(app)
    };
    let mut app = opens();
    for _ in 0..100 {
        if app.is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
        app = opens();
    }
    let app = app.expect("MailCraft opens its mail after an agent's call");
    // And while MailCraft has it, a call says so, rather than reading under it.
    let said = tools.run("search_mail", &json!({"query": "ambience"})).unwrap_err();
    assert!(said.contains("MailCraft has its data folder open"), "{said}");
    drop(app);
    // Calls one after another, each opening the folder afresh.
    for _ in 0..5 {
        assert!(tools.run("list_messages", &json!({"limit": 1})).is_ok());
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// An organisation that turns AI agents off (issue #245) is followed with MailCraft closed too,
/// whatever the person's own setting says.
#[test]
fn an_organisation_turning_agents_off_is_followed_here() {
    let (root, files) = sample("org-off");
    settings(&root, json!({"agents": true}));
    let mut tools = MailTools::new(root.clone(), files);
    let locked = json!({"settings": {"agents": {"value": false, "locked": true}}});
    tools.organisation = mailcraft_engine::policy::Organisation::from_sources(vec![("test".into(), locked, true)]);
    assert_eq!(tools.run("list_accounts", &json!({})).unwrap_err(), NOT_LET_IN);
    // A profile that leaves them on is read, and nothing of it written by a call that reads.
    let signature = json!({"signature": {"name": "Studio", "text": "{name}, Studio", "locked": true}});
    tools.organisation = mailcraft_engine::policy::Organisation::from_sources(vec![("test".into(), signature, true)]);
    let accounts = std::fs::read(root.join("accounts.json")).unwrap();
    assert!(tools.run("list_accounts", &json!({})).is_ok());
    assert_eq!(std::fs::read(root.join("accounts.json")).unwrap(), accounts, "the accounts' settings as they were");
    let _ = std::fs::remove_dir_all(&root);
}
