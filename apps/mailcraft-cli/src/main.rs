//! MailCraft on the command line.
//!
//! Its commands and options are in `help.txt`, which `mailcraft-cli --help` prints.
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

mod commands;

use commands::call;

struct Args {
    rest: Vec<String>,
}

/// The options that take no value. A switch was taken as the others, and swallowed what followed
/// it (`source --headers KEY` lost its KEY).
const SWITCHES: [&str; 6] = ["--headers", "--checked", "--help", "-h", "--version", "-V"];

/// The options that take the word after them, whatever it starts with (`--body "--Riley"`).
const OPTIONS: [&str; 22] = [
    "--data",
    "--email",
    "--name",
    "--username",
    "--imap",
    "--smtp",
    "--imap-security",
    "--smtp-security",
    "--sign-in",
    "--smtp-sign-in",
    "--jmap",
    "--account",
    "--timeout",
    "--folder",
    "--limit",
    "--to",
    "--cc",
    "--subject",
    "--body",
    "--signature",
    "--from",
    "--attach",
];

/// Whether `a` is one of the CLI's options or switches.
fn is_known(a: &str) -> bool {
    OPTIONS.contains(&a) || SWITCHES.contains(&a)
}

/// Whether `a` is written as an option (`--to`), not as a value that starts with dashes
/// (`-- Riley`, a signature's).
fn is_option(a: &str) -> bool {
    a.strip_prefix("--").is_some_and(|n| n.starts_with(|c: char| c.is_ascii_alphabetic()))
}

impl Args {
    /// Every option is one the CLI has, and each that takes a value has one: one given none took
    /// the next option for it (`--to --subject x` sent to "--subject"). A value is the word after
    /// its option, dashes or not; only another option's name there says it was left out.
    fn check(&self) -> Result<(), String> {
        let mut it = self.rest.iter();
        while let Some(a) = it.next() {
            if OPTIONS.contains(&a.as_str()) {
                if !it.next().is_some_and(|v| !is_known(v)) {
                    return Err(format!("{a} needs a value"));
                }
            } else if is_option(a) && !SWITCHES.contains(&a.as_str()) {
                return Err(format!("no option {a} (see --help)"));
            }
        }
        Ok(())
    }
    fn opt(&self, name: &str) -> Option<String> {
        self.rest.iter().position(|a| a == name).and_then(|i| self.rest.get(i + 1)).cloned()
    }
    /// A switch given (`--headers`), anywhere among the arguments.
    fn flag(&self, name: &str) -> bool {
        self.rest.iter().any(|a| a == name)
    }
    /// The number given with `name`, else `default`. One that isn't a number is an error: it was
    /// dropped for the default without a word.
    fn number(&self, name: &str, default: u64) -> Result<u64, String> {
        self.opt(name).map_or(Ok(default), |v| v.trim().parse().map_err(|_| format!("{name} takes a number, not {v:?}")))
    }
    fn positional(&self, n: usize) -> Option<String> {
        let mut skip = false;
        let mut i = 0;
        for a in &self.rest {
            if skip {
                skip = false;
                continue;
            }
            if is_known(a) {
                skip = OPTIONS.contains(&a.as_str());
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
    let zone = iana_time_zone::get_timezone().ok().and_then(|z| mailcraft_engine::model::zone::Zone::named(&z));
    Clock::new(now.timestamp(), now.offset().local_minus_utc())
        .in_zone(zone.filter(|z| z.offset_at(now.timestamp()) == i64::from(now.offset().local_minus_utc())))
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
            .find(|a| a.config.has_server())
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
                SyncStatus::Error { error, .. } => errln!("error: {error}"),
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
    if !config.has_server() {
        return Err("the sample account has no server".into());
    }
    let pw = password(&config.email)?;
    let mut pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    // Session::open queued Start jobs without passwords; replace them with one that has it.
    let _ = s.take_jobs();
    // Asked for, whatever the account is set to: one that looks only when asked (issue #104)
    // is asked now, by the command (held back, it never signed in, and the command waited).
    let account = mailcraft_engine::model::AccountConfig { check_by_hand: false, ..config };
    pool.submit(Job::Start { account, password: Some(Secret(pw)) });
    Ok(pool)
}

fn print(v: &Value) {
    outln!("{}", serde_json::to_string_pretty(v).unwrap_or_default());
}

/// The data files sealed as the command ends ([`Store::seal_all`]), so they reopen at once.
struct SealOnExit(Store);

impl Drop for SealOnExit {
    fn drop(&mut self) {
        if let Err(e) = self.0.seal_all() {
            errln!("{e}");
        }
    }
}

/// Say on standard error when a data file is checked as it opens (it wasn't closed properly,
/// which can take minutes for a big one). Data files open when a command first uses them, so
/// `help` and `commands` wait for none; this watches for as long as the command runs.
fn tell_checks(store: &Store) {
    use mailcraft_engine::store::OpenStage;
    let watch = store.open_progress();
    // No thread for it: the command still runs, untold.
    let _ = std::thread::Builder::new().name("checks".into()).spawn(move || {
        let mut told = String::new();
        loop {
            let file = watch.file();
            if watch.stage() == OpenStage::Checking && file != told {
                errln!("Checking {file}: it wasn't closed properly last time (a large file can take a minute or two)…");
                told = file;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    });
}

fn run() -> Result<(), String> {
    // `args()` panics on an argument that isn't Unicode (a file name in another encoding).
    let raw: Vec<String> = std::env::args_os().skip(1).map(|a| a.to_string_lossy().into_owned()).collect();
    if raw.iter().any(|a| a == "--version" || a == "-V") {
        // The version, plus the commit and date of release builds (packaging/common.sh).
        outln!("{}", mailcraft_engine::version_line("mailcraft-cli"));
        return Ok(());
    }
    let args = Args { rest: raw };
    args.check()?;
    if args.flag("--help") || args.flag("-h") {
        // Before the data folder is opened: the help needs none.
        outln!("{}", include_str!("help.txt"));
        return Ok(());
    }
    let root =
        args.opt("--data").map(std::path::PathBuf::from).or_else(mailcraft_engine::store::default_root).ok_or("no data folder: pass --data DIR")?;
    // The same data files as the app: in Documents\MailCraft Files (or where they were moved),
    // or beside a folder given with --data.
    let files = match (args.opt("--data"), std::env::var_os("MAILCRAFT_DATA").filter(|d| !d.is_empty())) {
        (None, None) => mailcraft_engine::store::location::files_dir(&root),
        _ => root.join(mailcraft_engine::store::location::FILES_FOLDER),
    };
    let store = open_store(&root, &files)?;
    tell_checks(&store);
    // Last out, however the command ends: the sync threads that hold the store are let go.
    let _sealed = SealOnExit(store.clone());
    let cmd = args.positional(0).unwrap_or_else(|| "help".into());
    if cmd == "sample" {
        let c = clock();
        let made = mailcraft_engine::sample::seed_if_empty_at(&store, c.now, i64::from(c.offset)).map_err(|e| e.to_string())?;
        outln!("{}", if made { format!("sample mailbox created in {}", root.display()) } else { "the data folder already has accounts".into() });
        return Ok(());
    }
    let mut s = Session::open(Some(store.clone()), clock());
    // An account's data file the app has open can't be read here: say so, where its folders came
    // out empty (found testing issue #20).
    for a in &s.accounts {
        if let Err(e) = store.load_folders(&a.config.id)
            && e.is_open_elsewhere()
        {
            // The data file's own words, without the file inside it that was asked for.
            let e = e.to_string();
            return Err(format!("{}: close MailCraft first", e.split_once(": ").map_or(e.as_str(), |(_, rest)| rest)));
        }
    }
    match cmd.as_str() {
        "accounts" => print(&call(&mut s, "account.list", &json!({}))?),
        "commands" => print(&s.registry().describe()),
        "add" => commands::add(&mut s, &args)?,
        "test" | "sync" => commands::sync(&mut s, &store, &args, cmd == "test")?,
        "list" => commands::list(&mut s, &args)?,
        "read" => commands::read(&mut s, &store, &args)?,
        "source" => commands::source(&mut s, &store, &args)?,
        "search" => commands::search(&mut s, &args)?,
        "send" => commands::send(&mut s, &store, &args)?,
        "run" => commands::run_command(&mut s, &store, &args)?,
        _ => {
            outln!("{}", include_str!("help.txt"));
        }
    }
    Ok(())
}

/// An account's data file measured or compacted (`dataFile.health`, `dataFile.compact`) as the app
/// does it, here, with how it goes on stderr; then what came of it. The other jobs are given back.
fn data_file_jobs(s: &mut Session, store: &Store, jobs: Vec<Job>) -> Vec<Job> {
    let (mine, other): (Vec<Job>, Vec<Job>) = jobs.into_iter().partition(|j| matches!(j, Job::DataFileHealth { .. } | Job::CompactDataFile { .. }));
    let Some(account) = mine.first().map(|j| j.account().to_string()) else { return other };
    let compacting = mine.iter().any(|j| matches!(j, Job::CompactDataFile { .. }));
    let mut pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    for j in mine {
        pool.submit(j);
    }
    // A compaction takes as long as it takes: minutes for a big file.
    loop {
        let Some(e) = pool.wait(Duration::from_millis(250)) else { continue };
        let ended = match &e {
            Event::Compacting { phase, work, exclusive_s, .. } => {
                let pct = mailcraft_engine::model::job::progress::permille(work.bytes_done, work.bytes_total).map(|p| p / 10);
                match (pct, exclusive_s) {
                    (Some(p), _) => errln!("{phase:?}: {p}%"),
                    (None, Some(secs)) => errln!("{phase:?}: about {secs} s"),
                    (None, None) => errln!("{phase:?}…"),
                }
                false
            }
            Event::Compacted { .. } => true,
            Event::DataFileHealth { .. } => !compacting,
            _ => false,
        };
        s.apply(e);
        if ended {
            break;
        }
    }
    // The health measured again after a compaction is the app's to ask: what's known is printed.
    print(&s.compaction_json(&account));
    other.into_iter().chain(s.take_jobs().into_iter().filter(|j| !matches!(j, Job::DataFileHealth { .. }))).collect()
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

/// The data files at `files` for the data folder `root`. Folders from before that the app has
/// open on the folder layout (`--sample`, or its data files failing to open) aren't brought in:
/// it's closed first, as for a data file it has open.
fn open_store(root: &std::path::Path, files: &std::path::Path) -> Result<Store, String> {
    Store::open_packed(root, files).map_err(|e| if e.is_open_elsewhere() { format!("{e}: close MailCraft first") } else { e.to_string() })
}

/// A sender warning (`message.read`'s `warning`) said in a line.
fn sender_warning_text(w: &serde_json::Value) -> String {
    let f = |k: &str| w[k].as_str().unwrap_or("").to_string();
    match w["kind"].as_str().unwrap_or("") {
        "lookalike" => format!("this comes from {}, which looks like {} but isn't it.", f("domain"), f("like")),
        "nameShowsOther" => format!("the sender's name shows {}, but this comes from {}.", f("shown"), f("real")),
        "nameOfKnownPerson" => format!("you know {} as {}, but this comes from {}.", f("name"), f("known"), f("real")),
        other => format!("the sender doesn't add up ({other})."),
    }
}

#[cfg(test)]
mod real_meetings;

#[cfg(test)]
mod real_names;

#[cfg(test)]
mod real_photos;

#[cfg(test)]
mod real_sent_rules;

#[cfg(test)]
mod tests {
    use super::*;
    use mailcraft_engine::model::job::Op;
    use mailcraft_engine::model::{Flag, MessageKey};

    fn args(a: &[&str]) -> Args {
        Args { rest: a.iter().map(|s| s.to_string()).collect() }
    }

    /// Every option the CLI reads is in the help it prints (`--jmap`, `--cc` and `--timeout`
    /// weren't).
    #[test]
    fn the_help_names_every_option() {
        let help = include_str!("help.txt");
        for name in OPTIONS.iter().chain(&SWITCHES) {
            assert!(name.len() <= 2 || help.contains(name), "{name} isn't in help.txt");
        }
    }

    /// A switch anywhere takes no value; an option given none is said; a number that isn't one
    /// is an error rather than the default.
    #[test]
    fn options_and_switches_are_told_apart() {
        let a = args(&["source", "--headers", "a1:1:7:INBOX"]);
        assert_eq!((a.positional(0).as_deref(), a.positional(1).as_deref(), a.flag("--headers")), (Some("source"), Some("a1:1:7:INBOX"), true));
        let a = args(&["send", "--checked", "--to", "zoe@studio.example", "--body", "-- Riley"]);
        assert!(a.check().is_ok());
        assert_eq!((a.opt("--to").as_deref(), a.opt("--body").as_deref(), a.flag("--checked")), (Some("zoe@studio.example"), Some("-- Riley"), true));
        assert_eq!(args(&["send", "--to", "--subject", "x"]).check(), Err("--to needs a value".into()));
        assert_eq!(args(&["list", "--limit"]).check(), Err("--limit needs a value".into()));
        assert_eq!(args(&["send", "--to", "--checked"]).check(), Err("--to needs a value".into()));
        assert_eq!(args(&["list", "--limit", "5"]).number("--limit", 30), Ok(5));
        assert_eq!(args(&["list"]).number("--limit", 30), Ok(30));
        assert!(args(&["list", "--limit", "two"]).number("--limit", 30).is_err());
    }

    /// A value is the word after its option, whatever it starts with: one that began with two
    /// dashes and a letter was refused as an option without a value. The switches for the help
    /// take none, and an option the CLI doesn't have is said.
    #[test]
    fn a_value_may_start_with_dashes() {
        let a = args(&["send", "--to", "zoe@studio.example", "--subject", "--force is deprecated", "--body", "--Riley"]);
        assert_eq!(a.check(), Ok(()));
        assert_eq!((a.opt("--subject").as_deref(), a.opt("--body").as_deref()), (Some("--force is deprecated"), Some("--Riley")));
        assert_eq!((a.positional(0).as_deref(), a.positional(1)), (Some("send"), None));
        let a = args(&["search", "--data", "--mine", "from:zoe"]);
        assert_eq!((a.check(), a.opt("--data").as_deref(), a.positional(1).as_deref()), (Ok(()), Some("--mine"), Some("from:zoe")));
        for help in ["--help", "-h"] {
            let a = args(&[help]);
            assert_eq!((a.check(), a.flag(help), a.positional(0)), (Ok(()), true, None));
        }
        assert_eq!(args(&["list", "--limt", "5"]).check(), Err("no option --limt (see --help)".into()));
    }

    /// Every option the CLI reads is one `check` knows, with or without a value.
    #[test]
    fn every_option_read_is_known() {
        for src in [include_str!("main.rs"), include_str!("commands.rs"), include_str!("commands/send.rs")] {
            for (i, _) in src.match_indices("opt(\"--").chain(src.match_indices("number(\"--")).chain(src.match_indices("flag(\"--")) {
                let name: String = src[i..].chars().skip_while(|c| *c != '-').take_while(|c| *c == '-' || c.is_ascii_alphanumeric()).collect();
                assert!(is_known(&name), "{name} isn't in OPTIONS or SWITCHES");
                assert_eq!(src[i..].starts_with("flag("), SWITCHES.contains(&name.as_str()), "{name}");
            }
        }
    }

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

    /// The app on the folder layout of the data folder: the CLI brings none of its folders into
    /// data files, and says to close MailCraft.
    #[test]
    fn folders_the_app_has_open_say_to_close_it() {
        use mailcraft_engine::model::AccountConfig;
        let dir = std::env::temp_dir().join(format!("mailcraft-cli-folders-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let app = Store::open_alone(&dir).unwrap();
        app.save_accounts(&[AccountConfig { id: "a1".into(), email: "zoe@studio.example".into(), ..Default::default() }]).unwrap();
        let key = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid: 1 };
        app.write_body(&key, b"Subject: one\r\n\r\nbody").unwrap();
        let e = open_store(&dir, &dir.join("Files")).unwrap_err();
        assert!(e.ends_with("open in another copy of MailCraft: close MailCraft first"), "{e}");
        assert!(dir.join("a1").is_dir(), "the app's folder stays");
        drop(app);
        let s = open_store(&dir, &dir.join("Files")).unwrap();
        assert_eq!(s.read_body(&key).unwrap(), b"Subject: one\r\n\r\nbody");
        drop(s);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
