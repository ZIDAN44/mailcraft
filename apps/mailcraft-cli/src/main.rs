//! MailCraft on the command line.
//!
//! ```text
//! mailcraft-cli [--data DIR] <command>
//!   sample                         create the local sample mailbox in DIR
//!   accounts                       list accounts and folders
//!   add --email E [--name N] [--username U] [--imap HOST:PORT] [--smtp HOST:PORT]
//!       [--imap-security tls|starttls|none] [--smtp-security tls|starttls|none]
//!       [--jmap SESSION-URL]   (a JMAP account instead of IMAP and SMTP)
//!                                  add an account (servers guessed from the address)
//!   test [--account ID]            sign in to IMAP and list the folders
//!   sync [--account ID]            download new mail into the cache
//!   list [--account ID] [--folder PATH] [--limit N]
//!   read KEY                       print a message (KEY from `list`)
//!   search QUERY                   search the cache (from:, subject:, has:attachment…)
//!   send --to ADDR --subject S --body TEXT [--account ID] [--attach FILE]
//!   commands                       every engine command, as JSON
//!   run ID [JSON]                  run an engine command against the cache (its changes
//!                                  to mail go to the server with the next `sync`)
//!   --version                      print the version
//! ```
//!
//! Passwords are read from `MAILCRAFT_PASSWORD` or asked for on the terminal; the CLI never
//! stores them.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::io::{BufRead, Write};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant};

use mailcraft_engine::Session;
use mailcraft_engine::model::dates::{Clock, full_time};
use mailcraft_engine::model::job::Secret;
use mailcraft_engine::model::{Event, Job, SyncStatus};
use mailcraft_engine::store::Store;
use mailcraft_net::worker::{MemoryCredentials, Pool};
use serde_json::{Value, json};

// Output that a closed pipe can't turn into a panic (`mailcraft-cli list | head` closes it
// early; `println!` panicked there).
macro_rules! outln {
    ($($t:tt)*) => {{
        let _ = writeln!(std::io::stdout(), $($t)*);
    }};
}
macro_rules! errln {
    ($($t:tt)*) => {{
        let _ = writeln!(std::io::stderr(), $($t)*);
    }};
}

struct Args {
    rest: Vec<String>,
}

impl Args {
    fn opt(&self, name: &str) -> Option<String> {
        self.rest.iter().position(|a| a == name).and_then(|i| self.rest.get(i + 1)).cloned()
    }
    fn positional(&self, n: usize) -> Option<String> {
        let mut skip = false;
        let mut i = 0;
        for a in &self.rest {
            if skip {
                skip = false;
                continue;
            }
            if a.starts_with("--") {
                skip = true;
                continue;
            }
            if i == n {
                return Some(a.clone());
            }
            i += 1;
        }
        None
    }
}

fn clock() -> Clock {
    let now = chrono::Local::now();
    Clock::new(now.timestamp(), now.offset().local_minus_utc())
}

fn password(prompt_for: &str) -> Result<String, String> {
    if let Ok(p) = std::env::var("MAILCRAFT_PASSWORD")
        && !p.is_empty()
    {
        return Ok(p);
    }
    let _ = write!(std::io::stderr(), "Password for {prompt_for}: ");
    let _ = std::io::stderr().flush();
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line).map_err(|e| e.to_string())?;
    let p = line.trim_end_matches(['\r', '\n']).to_string();
    if p.is_empty() { Err("no password given".into()) } else { Ok(p) }
}

fn pick_account(s: &Session, args: &Args) -> Result<String, String> {
    match args.opt("--account") {
        Some(a) if s.account(&a).is_some() => Ok(a),
        Some(a) => Err(format!("no account `{a}` (see `accounts`)")),
        None => s
            .accounts
            .iter()
            .find(|a| !a.is_local())
            .or(s.accounts.first())
            .map(|a| a.config.id.clone())
            .ok_or_else(|| "no accounts yet: use `add`".into()),
    }
}

/// Start the account's worker and run until `done` says so (or the timeout passes), printing
/// progress and feeding events to the session.
fn drive(s: &mut Session, pool: &mut Pool, timeout: Duration, mut done: impl FnMut(&Event) -> Option<Result<(), String>>) -> Result<(), String> {
    for j in s.take_jobs() {
        pool.submit(j);
    }
    let end = Instant::now() + timeout;
    while Instant::now() < end {
        let Some(e) = pool.wait(Duration::from_millis(250)) else { continue };
        match &e {
            Event::Status { status, .. } => match status {
                SyncStatus::Connecting => errln!("connecting…"),
                SyncStatus::Syncing { folder } => errln!("updating {}…", mailcraft_engine::model::utf7::decode(folder)),
                SyncStatus::Online { .. } => errln!("up to date"),
                SyncStatus::NeedsPassword { error } => errln!("sign-in failed: {error}"),
                SyncStatus::Error { error } => errln!("error: {error}"),
                SyncStatus::Offline => {}
            },
            Event::Notice { message, .. } => errln!("note: {message}"),
            Event::OpFailed { error, .. } => errln!("change refused: {error}"),
            _ => {}
        }
        let verdict = done(&e);
        s.apply(e);
        for j in s.take_jobs() {
            pool.submit(j);
        }
        if let Some(v) = verdict {
            return v;
        }
    }
    Err("timed out".into())
}

fn start(s: &mut Session, store: &Store, account: &str) -> Result<Pool, String> {
    let config = s.account(account).map(|a| a.config.clone()).ok_or("no such account")?;
    if config.is_local() {
        return Err("the sample account has no server".into());
    }
    let pw = password(&config.email)?;
    let mut pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    // Session::open queued Start jobs without passwords; replace them with one that has it.
    let _ = s.take_jobs();
    pool.submit(Job::Start { account: config, password: Some(Secret(pw)) });
    Ok(pool)
}

fn print(v: &Value) {
    outln!("{}", serde_json::to_string_pretty(v).unwrap_or_default());
}

/// `--version`: the version, plus the commit and date of release builds (packaging/common.sh).
fn version_line() -> String {
    let v = format!("mailcraft-cli {}", env!("CARGO_PKG_VERSION"));
    match (option_env!("MAILCRAFT_BUILD_SHA"), option_env!("MAILCRAFT_BUILD_DATE")) {
        (Some(sha), Some(date)) if !sha.is_empty() => format!("{v} ({}, {date})", sha.get(..9).unwrap_or(sha)),
        _ => v,
    }
}

fn run() -> Result<(), String> {
    // `args()` panics on an argument that isn't Unicode (a file name in another encoding).
    let raw: Vec<String> = std::env::args_os().skip(1).map(|a| a.to_string_lossy().into_owned()).collect();
    if raw.iter().any(|a| a == "--version" || a == "-V") {
        outln!("{}", version_line());
        return Ok(());
    }
    let args = Args { rest: raw };
    let root =
        args.opt("--data").map(std::path::PathBuf::from).or_else(mailcraft_engine::store::default_root).ok_or("no data folder: pass --data DIR")?;
    // The same data files as the app: in Documents\MailCraft Files (or where they were moved),
    // or beside a folder given with --data.
    let files = match (args.opt("--data"), std::env::var_os("MAILCRAFT_DATA").filter(|d| !d.is_empty())) {
        (None, None) => mailcraft_engine::store::location::files_dir(&root),
        _ => root.join(mailcraft_engine::store::location::FILES_FOLDER),
    };
    let store = Store::open_packed(&root, &files).map_err(|e| e.to_string())?;
    let cmd = args.positional(0).unwrap_or_else(|| "help".into());
    if cmd == "sample" {
        let c = clock();
        let made = mailcraft_engine::sample::seed_if_empty_at(&store, c.now, i64::from(c.offset)).map_err(|e| e.to_string())?;
        outln!("{}", if made { format!("sample mailbox created in {}", root.display()) } else { "the data folder already has accounts".into() });
        return Ok(());
    }
    let mut s = Session::open(Some(store.clone()), clock());
    match cmd.as_str() {
        "accounts" => print(&s.run("account.list", &json!({})).map_err(|e| e.to_string())?),
        "commands" => print(&s.registry().describe()),
        "add" => {
            let email = args.opt("--email").ok_or("--email is required")?;
            let mut p = json!({"email": email});
            for (flag, key) in [
                ("--name", "name"),
                ("--username", "username"),
                ("--imap-security", "imapSecurity"),
                ("--smtp-security", "smtpSecurity"),
                ("--jmap", "jmap"),
            ] {
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
            let r = s.run("account.add", &p).map_err(|e| e.to_string())?;
            print(&r);
            print(&s.run("account.list", &json!({})).map_err(|e| e.to_string())?);
        }
        "test" | "sync" => {
            let account = pick_account(&s, &args)?;
            let mut pool = start(&mut s, &store, &account)?;
            let test = cmd == "test";
            let timeout = Duration::from_secs(args.opt("--timeout").and_then(|t| t.parse().ok()).unwrap_or(if test { 60 } else { 900 }));
            let mut folders_seen = false;
            drive(&mut s, &mut pool, timeout, |e| match e {
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
                print(&s.run("account.list", &json!({})).map_err(|e| e.to_string())?);
            }
        }
        "list" => {
            if let Some(folder) = args.opt("--folder") {
                let account = pick_account(&s, &args)?;
                s.run("folder.open", &json!({"account": account, "folder": folder})).map_err(|e| e.to_string())?;
            } else if let Some(a) = args.opt("--account") {
                let inbox = s
                    .account(&a)
                    .and_then(|x| x.folder(mailcraft_engine::model::FolderRole::Inbox))
                    .map(|f| f.path.clone())
                    .unwrap_or_else(|| "INBOX".into());
                s.run("folder.open", &json!({"account": a, "folder": inbox})).map_err(|e| e.to_string())?;
            }
            let limit = args.opt("--limit").and_then(|l| l.parse::<u64>().ok()).unwrap_or(30);
            let r = s.run("list.messages", &json!({"limit": limit})).map_err(|e| e.to_string())?;
            outln!("{} ({} messages)", r["view"].as_str().unwrap_or(""), r["total"]);
            for m in r["messages"].as_array().into_iter().flatten() {
                let mark = if m["unread"].as_bool() == Some(true) { "●" } else { " " };
                let flag = if m["flagged"].as_bool() == Some(true) { "⚑" } else { " " };
                let att = if m["attachments"].as_bool() == Some(true) { "📎" } else { "  " };
                let when = full_time(m["date"].as_i64().unwrap_or(0), &s.clock);
                outln!("{mark}{flag}{att} {when}  {:<28}  {}", m["from"].as_str().unwrap_or(""), m["subject"].as_str().unwrap_or(""));
                outln!("      key: {}", m["key"].as_str().unwrap_or(""));
            }
        }
        "read" => {
            let key = args.positional(1).ok_or("read KEY")?;
            let k = mailcraft_engine::model::MessageKey::from_id(&key).ok_or("bad key")?;
            s.open_view(mailcraft_engine::View::Folder { account: k.account.clone(), folder: k.folder.clone() });
            let mut r = s.run("message.read", &json!({"key": key})).map_err(|e| e.to_string())?;
            if r.get("pending").is_some() {
                // Not cached: download it.
                let mut pool = start(&mut s, &store, &k.account)?;
                pool.submit(Job::FetchBody { key: k.clone() });
                drive(&mut s, &mut pool, Duration::from_secs(120), |e| match e {
                    Event::Body { key, .. } if *key == k => Some(Ok(())),
                    Event::BodyFailed { key, error } if *key == k => Some(Err(error.clone())),
                    _ => None,
                })?;
                r = s.run("message.read", &json!({})).map_err(|e| e.to_string())?;
            }
            outln!("From:    {}", r["from"].as_str().unwrap_or(""));
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
        }
        "search" => {
            let q = args.positional(1).ok_or("search QUERY")?;
            let r = s.run("search.run", &json!({"query": q})).map_err(|e| e.to_string())?;
            outln!("{} results", r["results"]);
            let r = s.run("list.messages", &json!({"limit": 50})).map_err(|e| e.to_string())?;
            for m in r["messages"].as_array().into_iter().flatten() {
                outln!(
                    "{}  {:<28}  {}\n      key: {}",
                    full_time(m["date"].as_i64().unwrap_or(0), &s.clock),
                    m["from"].as_str().unwrap_or(""),
                    m["subject"].as_str().unwrap_or(""),
                    m["key"].as_str().unwrap_or("")
                );
            }
        }
        "send" => {
            let account = pick_account(&s, &args)?;
            s.run("mail.new", &json!({"account": account})).map_err(|e| e.to_string())?;
            let body = args.opt("--body").unwrap_or_default();
            s.run("compose.update", &json!({"to": args.opt("--to").ok_or("--to is required")?, "cc": args.opt("--cc").unwrap_or_default(), "subject": args.opt("--subject").unwrap_or_default(), "body": body}))
                .map_err(|e| e.to_string())?;
            if let Some(f) = args.opt("--attach") {
                s.run("compose.attach", &json!({"path": f})).map_err(|e| e.to_string())?;
            }
            let local = s.account(&account).is_some_and(|a| a.is_local());
            if local {
                print(&s.run("compose.send", &json!({"allowEmptySubject": true})).map_err(|e| e.to_string())?);
                return Ok(());
            }
            let mut pool = start(&mut s, &store, &account)?;
            s.run("compose.send", &json!({"allowEmptySubject": true})).map_err(|e| e.to_string())?;
            drive(&mut s, &mut pool, Duration::from_secs(180), |e| match e {
                Event::Sent { .. } => Some(Ok(())),
                Event::SendFailed { error, .. } => Some(Err(error.clone())),
                _ => None,
            })?;
            outln!("sent");
        }
        "run" => {
            let id = args.positional(1).ok_or("run ID [JSON]")?;
            let params: Value = match args.positional(2) {
                Some(p) => serde_json::from_str(&p).map_err(|e| format!("params: {e}"))?,
                None => json!({}),
            };
            print(&s.run(&id, &params).map_err(|e| e.to_string())?);
            let (queued, other) = keep_changes(&store, s.take_jobs())?;
            if queued > 0 {
                errln!("{queued} change(s) queued for the server: `sync` sends them");
            }
            if other > 0 {
                errln!("{other} other network job(s) skipped: they run in the app");
            }
        }
        _ => {
            outln!("{}", include_str!("help.txt"));
        }
    }
    Ok(())
}

/// Changes to mail (moves, flags, deletions, folders) kept as the app keeps them: made in the
/// cache and queued for the server, which the next `sync` sends (they were dropped before).
/// How many were kept, and how many other jobs (sending, fetching) were left to the app.
fn keep_changes(store: &Store, jobs: Vec<Job>) -> Result<(usize, usize), String> {
    let (mut queued, mut other) = (0, 0);
    for job in jobs {
        match job {
            Job::Apply { account, op } => {
                store.apply_op(&account, &op).map_err(|e| e.to_string())?;
                store.add_op(&account, op).map_err(|e| e.to_string())?;
                queued += 1;
            }
            _ => other += 1,
        }
    }
    Ok((queued, other))
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            errln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mailcraft_engine::model::job::Op;
    use mailcraft_engine::model::{Flag, MessageKey};

    /// A change made with `run` waits for `sync` instead of being dropped.
    #[test]
    fn changes_made_with_run_are_kept_for_sync() {
        let dir = std::env::temp_dir().join(format!("mailcraft-cli-keep-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = Store::open(&dir).unwrap();
        let key = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid: 7 };
        let op = Op::SetFlag { keys: vec![key], flag: Flag::Flagged, on: true };
        let jobs = vec![Job::Apply { account: "a1".into(), op: op.clone() }, Job::SyncAll { account: "a1".into() }];
        assert_eq!(keep_changes(&store, jobs).unwrap(), (1, 1));
        assert_eq!(store.load_ops("a1").unwrap(), vec![op]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
