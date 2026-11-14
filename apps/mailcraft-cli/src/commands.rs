//! The CLI's commands, one function each (`run()` in `main.rs` picks one); `send` is in
//! `commands/send.rs`.

use super::*;

mod send;

pub(crate) use send::send;

/// An engine command, its error as the CLI says it.
pub(crate) fn call(s: &mut Session, id: &str, params: &Value) -> Result<Value, String> {
    s.run(id, params).map_err(|e| e.to_string())
}

/// `add`: an account, its servers given or looked up.
pub(crate) fn add(s: &mut Session, args: &Args) -> Result<(), String> {
    let email = args.opt("--email").ok_or("--email is required")?;
    let mut p = json!({"email": email});
    for (flag, key) in
        [("--name", "name"), ("--username", "username"), ("--imap-security", "imapSecurity"), ("--smtp-security", "smtpSecurity"), ("--jmap", "jmap")]
    {
        if let Some(v) = args.opt(flag) {
            p[key] = json!(v);
        }
    }
    // No servers given: look them up (the domain's autoconfig, the ISP database, DNS).
    if args.opt("--imap").is_none() && args.opt("--smtp").is_none() && args.opt("--jmap").is_none() {
        match mailcraft_net::autoconfig::discover(&email) {
            Some(f) => {
                errln!("settings found: {}", f.source);
                p["imapHost"] = json!(f.config.imap.host);
                p["imapPort"] = json!(f.config.imap.port);
                p["imapSecurity"] = json!(f.config.imap.security.code());
                p["smtpHost"] = json!(f.config.smtp.host);
                p["smtpPort"] = json!(f.config.smtp.port);
                p["smtpSecurity"] = json!(f.config.smtp.security.code());
                if args.opt("--username").is_none() {
                    p["username"] = json!(f.config.username);
                }
            }
            None => errln!("no published settings for this domain: guessing (check with `accounts`)"),
        }
    }
    for (flag, host, port) in [("--imap", "imapHost", "imapPort"), ("--smtp", "smtpHost", "smtpPort")] {
        if let Some(v) = args.opt(flag) {
            let (h, n) = v.rsplit_once(':').ok_or(format!("{flag} wants HOST:PORT"))?;
            p[host] = json!(h);
            p[port] = json!(n.parse::<u16>().map_err(|_| format!("{flag}: bad port"))?);
        }
    }
    // The CLI never stores passwords: `test`, `sync` and `send` ask each time.
    p["deferSignIn"] = json!(true);
    let r = call(s, "account.add", &p)?;
    print(&r);
    print(&call(s, "account.list", &json!({}))?);
    Ok(())
}

/// `test` (`test` true: sign in and list the folders) and `sync` (download new mail).
pub(crate) fn sync(s: &mut Session, store: &Store, args: &Args, test: bool) -> Result<(), String> {
    let account = pick_account(s, args)?;
    let mut pool = start(s, store, &account)?;
    let timeout = Duration::from_secs(args.number("--timeout", if test { 60 } else { 900 })?);
    let mut folders_seen = false;
    drive(s, &mut pool, timeout, |e| match e {
        Event::Folders { folders, .. } if test && !folders_seen => {
            folders_seen = true;
            outln!("signed in; {} folders:", folders.len());
            for f in folders {
                outln!("  {:<10} {}", f.role.code(), mailcraft_engine::model::utf7::decode(&f.path));
            }
            Some(Ok(()))
        }
        Event::Status { status: SyncStatus::Online { .. }, .. } => Some(Ok(())),
        Event::Status { status: SyncStatus::NeedsPassword { error }, .. } => Some(Err(format!("sign-in failed: {error}"))),
        _ => None,
    })?;
    if !test {
        print(&call(s, "account.list", &json!({}))?);
    }
    Ok(())
}

/// `list`: a folder's messages, newest first.
pub(crate) fn list(s: &mut Session, args: &Args) -> Result<(), String> {
    if let Some(folder) = args.opt("--folder") {
        let account = pick_account(s, args)?;
        call(s, "folder.open", &json!({"account": account, "folder": folder}))?;
    } else if let Some(a) = args.opt("--account") {
        let inbox = s
            .account(&a)
            .and_then(|x| x.folder(mailcraft_engine::model::FolderRole::Inbox))
            .map(|f| f.path.clone())
            .unwrap_or_else(|| "INBOX".into());
        call(s, "folder.open", &json!({"account": a, "folder": inbox}))?;
    }
    let limit = args.number("--limit", 30)?;
    let r = call(s, "list.messages", &json!({"limit": limit}))?;
    outln!("{} ({} messages)", r["view"].as_str().unwrap_or(""), r["total"]);
    for m in r["messages"].as_array().into_iter().flatten() {
        let mark = if m["unread"].as_bool() == Some(true) { "●" } else { " " };
        let flag = if m["flagged"].as_bool() == Some(true) {
            "⚑"
        } else if m["complete"].as_bool() == Some(true) {
            "✓"
        } else {
            " "
        };
        let att = if m["attachments"].as_bool() == Some(true) { "📎" } else { "  " };
        let when = full_time(m["date"].as_i64().unwrap_or(0), &s.clock);
        // Who it's from doesn't add up, or it's from outside the organisation (issue #29).
        let check = if m["senderWarning"].as_bool() == Some(true) { "⚠ " } else { "" };
        let external = if m["external"].as_bool() == Some(true) { " [External]" } else { "" };
        outln!("{mark}{flag}{att} {when}  {check}{:<28}  {}{external}", m["from"].as_str().unwrap_or(""), m["subject"].as_str().unwrap_or(""));
        let cats: Vec<&str> = m["categories"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
        if !cats.is_empty() {
            outln!("      categories: {}", cats.join(", "));
        }
        outln!("      key: {}", m["key"].as_str().unwrap_or(""));
    }
    Ok(())
}

/// `read KEY`: a message, downloaded first when it isn't here.
pub(crate) fn read(s: &mut Session, store: &Store, args: &Args) -> Result<(), String> {
    let key = args.positional(1).ok_or("read KEY")?;
    let k = mailcraft_engine::model::MessageKey::from_id(&key).ok_or("bad key")?;
    s.open_view(mailcraft_engine::View::Folder { account: k.account.clone(), folder: k.folder.clone() });
    let mut r = call(s, "message.read", &json!({"key": key}))?;
    if r.get("pending").is_some() {
        // Not cached: download it.
        let mut pool = start(s, store, &k.account)?;
        pool.submit(Job::FetchBody { key: k.clone() });
        drive(s, &mut pool, Duration::from_secs(120), |e| match e {
            Event::Body { key, .. } if *key == k => Some(Ok(())),
            Event::BodyFailed { key, error } if *key == k => Some(Err(error.clone())),
            _ => None,
        })?;
        r = call(s, "message.read", &json!({}))?;
    }
    let external = if r["external"].as_bool() == Some(true) { " [External]" } else { "" };
    outln!("From:    {}{external}", r["from"].as_str().unwrap_or(""));
    if let Some(w) = r.get("warning").filter(|w| !w.is_null()) {
        outln!("Careful: {}", sender_warning_text(w));
    }
    outln!("To:      {}", r["to"].as_str().unwrap_or(""));
    if r["cc"].as_str().is_some_and(|c| !c.is_empty()) {
        outln!("Cc:      {}", r["cc"].as_str().unwrap_or(""));
    }
    outln!("Date:    {}", full_time(r["date"].as_i64().unwrap_or(0), &s.clock));
    outln!("Subject: {}", r["subject"].as_str().unwrap_or(""));
    for a in r["attachments"].as_array().into_iter().flatten() {
        outln!("Attached: [{}] {} ({}, {} bytes)", a["index"], a["name"].as_str().unwrap_or(""), a["mime"].as_str().unwrap_or(""), a["size"]);
    }
    outln!("\n{}", r["text"].as_str().unwrap_or(""));
    Ok(())
}

/// `source KEY [--headers]`: a message as it came, headers and all (issue #27): what
/// View message source shows.
pub(crate) fn source(s: &mut Session, store: &Store, args: &Args) -> Result<(), String> {
    let key = args.positional(1).ok_or("source KEY")?;
    let k = mailcraft_engine::model::MessageKey::from_id(&key).ok_or("bad key")?;
    s.open_view(mailcraft_engine::View::Folder { account: k.account.clone(), folder: k.folder.clone() });
    let params = json!({"key": key, "headers": args.flag("--headers")});
    let mut r = call(s, "message.source", &params)?;
    if r["pending"].as_bool() == Some(true) {
        // Not cached: download it (its text first, from Exchange, isn't it).
        let mut pool = start(s, store, &k.account)?;
        pool.submit(Job::FetchBody { key: k.clone() });
        drive(s, &mut pool, Duration::from_secs(300), |e| match e {
            Event::Body { key, raw } if *key == k && !mailcraft_engine::mime::is_text_first(raw) => Some(Ok(())),
            Event::BodyFailed { key, error } if *key == k => Some(Err(error.clone())),
            _ => None,
        })?;
        r = call(s, "message.source", &params)?;
    }
    if let Some(e) = r["error"].as_str() {
        return Err(e.to_string());
    }
    let _ = write!(std::io::stdout(), "{}", r["text"].as_str().unwrap_or(""));
    if r["truncated"].as_bool() == Some(true) {
        errln!("(only the start of {} bytes is shown)", r["size"]);
    }
    Ok(())
}

/// `search QUERY`: what the cache finds.
pub(crate) fn search(s: &mut Session, args: &Args) -> Result<(), String> {
    let q = args.positional(1).ok_or("search QUERY")?;
    let r = call(s, "search.run", &json!({"query": q}))?;
    outln!("{} results", r["results"]);
    let r = call(s, "list.messages", &json!({"limit": 50}))?;
    for m in r["messages"].as_array().into_iter().flatten() {
        outln!(
            "{}  {:<28}  {}\n      key: {}",
            full_time(m["date"].as_i64().unwrap_or(0), &s.clock),
            m["from"].as_str().unwrap_or(""),
            m["subject"].as_str().unwrap_or(""),
            m["key"].as_str().unwrap_or("")
        );
    }
    Ok(())
}

/// `run ID [JSON]`: an engine command against the cache; calendars subscribed to by their
/// links are read now, and changes to mail are kept for `sync`.
pub(crate) fn run_command(s: &mut Session, store: &Store, args: &Args) -> Result<(), String> {
    let id = args.positional(1).ok_or("run ID [JSON]")?;
    let params: Value = match args.positional(2) {
        Some(p) => serde_json::from_str(&p).map_err(|e| format!("params: {e}"))?,
        None => json!({}),
    };
    print(&call(s, &id, &params)?);
    // Calendars subscribed to by their links (issue #7) are read now, needing no account's
    // sign-in: what came of each is said.
    let jobs = s.take_jobs();
    let jobs = data_file_jobs(s, store, jobs);
    let (calendars, jobs): (Vec<Job>, Vec<Job>) = jobs.into_iter().partition(|j| matches!(j, Job::FetchCalendar { .. }));
    fetch_calendars(s, store, calendars);
    let (queued, other) = keep_changes(store, jobs)?;
    if queued > 0 {
        errln!("{queued} change(s) queued for the server: `sync` sends them");
    }
    if other > 0 {
        errln!("{other} other network job(s) skipped: they run in the app");
    }
    Ok(())
}

/// Calendars subscribed to by their links read, each said as it comes (two minutes at most).
/// Not through [`drive`]: that hands the session's new jobs to the pool, and here no account is
/// signed in for them; what the session asks for meanwhile is left to the app.
fn fetch_calendars(s: &mut Session, store: &Store, calendars: Vec<Job>) {
    if calendars.is_empty() {
        return;
    }
    let mut pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    let mut waiting: Vec<String> =
        calendars.iter().filter_map(|j| if let Job::FetchCalendar { calendar, .. } = j { Some(calendar.clone()) } else { None }).collect();
    for j in calendars {
        pool.submit(j);
    }
    let end = Instant::now() + Duration::from_secs(120);
    while !waiting.is_empty() && Instant::now() < end {
        let Some(e) = pool.wait(Duration::from_millis(250)) else { continue };
        if let Event::CalendarFetched { account, calendar, error, unchanged, .. } = &e {
            waiting.retain(|c| c != calendar);
            let (account, calendar) = (account.clone(), calendar.clone());
            let (error, unchanged) = (error.clone(), *unchanged);
            s.apply(e);
            let events = s.pim.get(&account).map_or(0, |p| p.events.iter().filter(|x| x.calendar == calendar).count());
            let name = s.pim.get(&account).and_then(|p| p.calendars.iter().find(|c| c.id == calendar)).map(|c| c.name.clone()).unwrap_or_default();
            match error {
                Some(e) => errln!("{name}: couldn't be read: {e}"),
                None if unchanged => errln!("{name}: unchanged, {events} event(s)"),
                None => errln!("{name}: read, {events} event(s)"),
            }
        }
    }
    if !waiting.is_empty() {
        errln!("{} calendar(s) didn't answer in time", waiting.len());
    }
}
