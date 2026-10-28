//! Commands: accounts, data files, mailbox settings.

use super::*;

pub(super) fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("folder.empty", "Empty folder", "Folder › Empty", |s, v| {
            let account = account_param(s, v)?;
            let folder = match (p::str(v, "folder"), p::str(v, "role")) {
                (Some(f), _) => f.to_string(),
                (None, Some(r)) => {
                    let role = FolderRole::from_code(r).ok_or_else(|| CmdError::Params(format!("unknown folder role `{r}`")))?;
                    s.account(&account).and_then(|a| a.folder(role)).map(|f| f.path.clone()).ok_or_else(|| CmdError::Params(format!("the account has no {r} folder")))?
                }
                (None, None) => return Err(CmdError::Params("`folder` or `role` is required".into())),
            };
            s.empty_folder(&account, &folder)
        })
        .params(r#"{"account"?: string, "folder"?: string (Deleted Items and Junk are emptied for good; any other folder's mail goes to Deleted Items), "role"?: "trash"|"junk"}"#),
        CommandSpec::new("account.add", "Add account", "File › Accounts", add_account)
            .params(r#"{"email": string, "password"?: string, "oauth"?: "google"|"microsoft", "deferSignIn"?: bool, "local"?: bool, "name"?: string, "username"?: string, "imapHost"?: string, "imapPort"?: number, "imapSecurity"?: "tls"|"starttls"|"none", "smtpHost"?: string, "smtpPort"?: number, "smtpSecurity"?: …, "smtpUsername"?: string and "smtpPassword"?: string (the outgoing server's own sign-in, when it isn't the incoming server's), "jmap"?: string (a JMAP session URL, instead of IMAP and SMTP), "pop"?: true (POP3 instead of IMAP: popHost, popPort, popSecurity, "popDelete"?: bool to remove mail from the server once downloaded, else a copy stays: "popKeepDays"?: number (removed that many days after downloading; 14 when not given, 0 for never), "popDeleteWithTrash"?: bool (removed once deleted here for good)), "pst"?: string (an existing Outlook data file whose mail comes in first), "ews"?: string|true (on-premises Exchange: its EWS address or server name, or true to find it with Autodiscover; username DOMAIN\\user or an address), "graph"?: true|string (Exchange Online through Microsoft Graph; signs in with Microsoft), "userAgent"?: string (with ews: the User-Agent its requests name instead of MailCraft's, for a server that lets in only clients it knows), "onlineOnly"?: bool (keep none of its mail on this computer; not for POP)}"#),
        CommandSpec::new("account.importPst", "Open Outlook data file", "File › Open & Export", |s, v| {
            let path = p::req_str(v, "path")?.to_string();
            s.import_pst(&path)
        })
        .params(r#"{"path": string (a .pst or .mailcraft data file; its mail goes into a new account on this computer)}"#),
        CommandSpec::new("account.exportPst", "Export to Outlook data file", "File › Open & Export", |s, v| {
            let path = p::req_str(v, "path")?.to_string();
            let account = match p::str(v, "account") {
                Some(a) => a.to_string(),
                None => s.current_account().map(|a| a.config.id.clone()).ok_or_else(|| CmdError::Params("there is no account to export".into()))?,
            };
            s.export_pst(&account, &path)
        })
        .params(r#"{"path": string (the .pst file to write; one already there is replaced), "account"?: string (default: the current one)}"#),
        CommandSpec::new("account.discover", "Find server settings", "File › Accounts", |s, v| {
            let email = p::req_str(v, "email")?.trim().to_ascii_lowercase();
            if !mailcraft_model::addr::is_valid_email(&email) {
                return Err(CmdError::Params(format!("`{email}` is not an email address")));
            }
            match s.discovered.get(&email) {
                Some(Some((Some(c), source))) => {
                    Ok(json!({"found": true, "source": source, "imap": c.imap, "smtp": c.smtp, "pop": c.pop, "username": c.username}))
                }
                Some(Some((None, _))) => Ok(json!({"found": false})),
                Some(None) => Ok(json!({"pending": true})),
                None => {
                    s.discovered.insert(email.clone(), None);
                    s.push_job(mailcraft_model::Job::Discover { email });
                    Ok(json!({"pending": true}))
                }
            }
        })
        .params(r#"{"email": string} (asks once; call again for the answer)"#),
        CommandSpec::new("account.move", "Move account", "Folder pane › account menu", |s, v| {
            let id = p::req_str(v, "account")?.to_string();
            let by = p::i64(v, "by").ok_or_else(|| CmdError::Params("`by` is required: -1 (up) or 1 (down)".into()))?;
            s.move_account(&id, by.clamp(-1000, 1000))
        })
        .params(r#"{"account": string, "by": number (-1 moves it up, 1 down)}"#),
        CommandSpec::new("folderPane.move", "Move mailbox", "Folder pane › mailbox menu", |s, v| {
            let key = p::req_str(v, "key")?.to_string();
            let by = p::i64(v, "by").ok_or_else(|| CmdError::Params("`by` is required: -1 (up) or 1 (down)".into()))?;
            let to = s.move_pane_section(&key, by.clamp(-1000, 1000))?;
            Ok(json!({"key": key, "position": to}))
        })
        .params(r#"{"key": string (an account id, or <account>|<address> for a shared mailbox), "by": number (-1 moves it up, 1 down)}"#),
        CommandSpec::new("account.remove", "Remove account", "File › Accounts", |s, v| {
            let id = p::req_str(v, "account")?.to_string();
            s.remove_account(&id)
        })
        .params(r#"{"account": string}"#),
        CommandSpec::new("account.signIn", "Sign in with the browser", "File › Accounts", |s, v| {
            let id = p::req_str(v, "account")?;
            let config = s.account(id).map(|a| a.config.clone()).ok_or_else(|| CmdError::Params(format!("no account `{id}`")))?;
            if config.oauth.is_none() {
                // A Microsoft account signing in with a password (refused there) switches over.
                if mailcraft_model::microsoft_password_account(&config) {
                    if !s.oauth_available.iter().any(|p| p == "microsoft") {
                        return Err(CmdError::Params("signing in with Microsoft isn't set up in this copy of MailCraft".into()));
                    }
                    return s.use_microsoft_sign_in(id);
                }
                return Err(CmdError::Params("this account signs in with a password".into()));
            }
            s.push_job(mailcraft_model::Job::SignIn { account: config });
            Ok(json!({"signingIn": id}))
        })
        .params(r#"{"account": string} (a Microsoft account that signs in with a password switches to Sign in with Microsoft, keeping its mail)"#),
        CommandSpec::new("account.useGraph", "Connect through Microsoft Graph", "File › Accounts", |s, v| {
            let id = p::req_str(v, "account")?.to_string();
            s.use_graph(&id)
        })
        .params(r#"{"account": string} (a Microsoft account over IMAP starts again through Microsoft Graph, for a mailbox Microsoft won't open over IMAP)"#),
        CommandSpec::new("account.setPassword", "Sign in again", "File › Accounts", set_password).params(r#"{"account": string, "password": string}"#),
        CommandSpec::new("account.setSignature", "Signature", "File › Options › Mail", set_signature)
            .params(r#"{"account": string, "signature": string, "html"?: string (its look: bold, colours, links; the text then comes from it)}"#),
        CommandSpec::new("account.dav", "Calendars and contacts", "File › Info › Account settings", set_dav).params(
            r#"{"account": string, "caldav"?: string (the calendar server's address: its own, the home, or one calendar; empty: found from the account's address), "carddav"?: string (the same for contacts), "username"?: string|null (its own sign-in; null: as the mail server), "password"?: string}"#,
        ),
        CommandSpec::new("account.list", "Accounts", "File › Accounts", list_accounts),
        // Asks where MailCraft is published; File › About shows the answer (`Session::update`).
        CommandSpec::new("app.checkUpdates", "Check for updates", "File › About", |s, _| {
            if s.jmap_only {
                return Err(CmdError::Failed("the browser version is always the newest".into()));
            }
            s.check_updates();
            Ok(json!({"checking": true}))
        }),
        CommandSpec::new("account.addSharedMailbox", "Add a shared mailbox", "File › Info › Account settings", |s, v| {
            shared_mailbox(s, v, true)
        })
        .params(r#"{"account": string, "address": string (a mailbox the account has access to on the Exchange server)}"#),
        CommandSpec::new("account.removeSharedMailbox", "Remove a shared mailbox", "File › Info › Account settings", |s, v| {
            shared_mailbox(s, v, false)
        })
        .params(r#"{"account": string, "address": string}"#),
        CommandSpec::new("account.update", "Account settings", "File › Info › Account settings", update_account).params(
            r#"{"account": string, "name"?: string, "username"?: string, "imapHost"?: string, "imapPort"?: number, "imapSecurity"?: …, "smtpHost"?: …, "smtpUsername"?: string|null (the outgoing server's own sign-in; null: the incoming server's), "smtpPassword"?: string, "jmap"?: string|null, "popHost"?: …, "popDelete"?: bool, "popKeepDays"?: number, "popDeleteWithTrash"?: bool, "onlineOnly"?: bool, "ews"?: string, "userAgent"?: string (Exchange: the User-Agent its requests name; "" for MailCraft's)}"#,
        ),
        CommandSpec::new("account.mailbox", "Mailbox settings", "File › Info › Mailbox settings", |s, v| {
            let id = account_param(s, v)?;
            let a = s.account(&id).ok_or_else(|| CmdError::Params(format!("no account `{id}`")))?;
            let quota = a.quota.map(|(used, limit)| json!({"used": used, "limit": limit}));
            Ok(json!({"account": id, "quota": quota, "cached": s.cache_size(&id)}))
        })
        .params(r#"{"account"?: string}"#),
        CommandSpec::new("account.vacation", "Automatic replies", "File › Info › Automatic replies", |s, v| {
            let id = account_param(s, v)?;
            if p::bool(v, "refresh").unwrap_or(true) {
                s.load_vacation(&id)
            } else {
                Ok(s.vacation_json(&id))
            }
        })
        .params(r#"{"account"?: string, "refresh"?: bool} (asks the server; call with refresh=false for the answer)"#),
        CommandSpec::new("account.setVacation", "Turn automatic replies on or off", "File › Info › Automatic replies", |s, v| {
            let id = account_param(s, v)?;
            let day = |k: &str| p::str(v, k).map(str::trim).filter(|d| !d.is_empty()).map(str::to_string);
            let vacation = mailcraft_model::job::Vacation {
                enabled: p::bool(v, "enabled").unwrap_or(true),
                subject: p::str(v, "subject").unwrap_or_default().chars().take(200).collect(),
                text: p::str(v, "text").unwrap_or_default().chars().take(20_000).collect(),
                from: day("from"),
                until: day("until"),
                external: p::str(v, "external").map(|t| t.chars().take(20_000).collect::<String>()).filter(|t| !t.trim().is_empty()),
                outside: match p::str(v, "outside").map(str::trim) {
                    None | Some("" | "all") => String::new(),
                    Some(o @ ("known" | "none")) => o.to_string(),
                    Some(other) => return Err(CmdError::Params(format!("`outside` is all, known or none, not `{other}`"))),
                },
            };
            s.set_vacation(&id, vacation)
        })
        .params(r#"{"account"?: string, "enabled": bool, "subject"?: string, "text": string, "from"?: "YYYY-MM-DD", "until"?: "YYYY-MM-DD", "external"?: string (Exchange: another reply for people outside the organisation), "outside"?: "all"|"known"|"none" (Exchange: who outside gets one)}"#),
    ]
}

pub(super) fn server(v: &Value, prefix: &str, guess: &ServerConfig) -> Result<ServerConfig, CmdError> {
    let host = p::str(v, &format!("{prefix}Host")).map(|h| h.trim().to_string()).unwrap_or_else(|| guess.host.clone());
    let port = match p::u64(v, &format!("{prefix}Port")) {
        Some(n) => u16::try_from(n).map_err(|_| CmdError::Params(format!("{prefix}Port: {n} is not a port")))?,
        None => guess.port,
    };
    let security = match p::str(v, &format!("{prefix}Security")) {
        Some(x) => Security::from_code(x).ok_or_else(|| CmdError::Params(format!("{prefix}Security: unknown `{x}`")))?,
        None => guess.security,
    };
    if host.contains(char::is_whitespace) || host.contains('/') {
        return Err(CmdError::Params(format!("{prefix}Host: `{host}` is not a server name")));
    }
    Ok(ServerConfig { host, port, security })
}

pub(super) fn add_account(s: &mut Session, v: &Value) -> CmdResult {
    let email = p::req_str(v, "email")?;
    let guess = AccountConfig::guess(email);
    let local = p::bool(v, "local").unwrap_or(false);
    let jmap = p::str(v, "jmap").map(str::trim).filter(|u| !u.is_empty()).map(str::to_string);
    // `graph`: true for Exchange Online through Microsoft Graph (or the API's address).
    let graph = match v.get("graph") {
        Some(Value::Bool(true)) => Some(mailcraft_model::GRAPH_URL.to_string()),
        Some(Value::String(u)) if !u.trim().is_empty() => Some(u.trim().to_string()),
        _ => None,
    };
    // `ews`: on-premises Exchange through Exchange Web Services, at this address (a server name
    // alone means its usual `/EWS/Exchange.asmx`).
    // `true`: found with Autodiscover when the account connects.
    let ews = match v.get("ews") {
        Some(Value::Bool(true)) => Some(mailcraft_model::EWS_AUTODISCOVER.to_string()),
        Some(Value::String(u)) if !u.trim().is_empty() => Some(ews_url(u)),
        _ => None,
    };
    if s.jmap_only && jmap.is_none() {
        return Err(CmdError::Params("in the browser, MailCraft works with JMAP accounts only".into()));
    }
    // `pop`: true to download mail over POP3 (popHost, popPort, popSecurity) and send over SMTP.
    let pop = if p::bool(v, "pop").unwrap_or(false) {
        let host = match guess.imap.host.strip_prefix("imap.") {
            Some(rest) => format!("pop.{rest}"),
            None => guess.imap.host.clone(),
        };
        Some(server(v, "pop", &ServerConfig { host, port: 995, security: Security::Tls })?)
    } else {
        None
    };
    let plain = local || jmap.is_some() || graph.is_some() || ews.is_some();
    let user_agent = if ews.is_some() { user_agent_param(v)?.flatten() } else { None };
    let password = p::str(v, "password").map(str::to_string);
    // Graph signs in with Microsoft, unless given a token to use as it is.
    let oauth = p::str(v, "oauth")
        .map(str::to_string)
        .or_else(|| (graph.is_some() && password.as_deref().is_none_or(str::is_empty)).then(|| "microsoft".to_string()));
    let config = AccountConfig {
        id: String::new(),
        name: p::str(v, "name").map(str::to_string).unwrap_or(guess.name.clone()),
        email: email.to_string(),
        username: p::str(v, "username").map(str::to_string).unwrap_or(guess.username.clone()),
        imap: if plain || pop.is_some() { ServerConfig::default() } else { server(v, "imap", &guess.imap)? },
        smtp: if plain { ServerConfig::default() } else { server(v, "smtp", &guess.smtp)? },
        smtp_username: None,
        signature: p::str(v, "signature").unwrap_or_default().to_string(),
        signature_html: String::new(),
        color: 0,
        oauth,
        jmap,
        graph,
        ews,
        pop,
        pop_delete: p::bool(v, "popDelete").unwrap_or(false),
        // A copy stays on the server for 14 days unless asked otherwise (0: for good).
        pop_keep_days: match v.get("popKeepDays").and_then(serde_json::Value::as_u64) {
            None => Some(14),
            Some(0) => None,
            Some(n) => Some(u32::try_from(n).unwrap_or(u32::MAX)),
        },
        pop_delete_with_trash: p::bool(v, "popDeleteWithTrash").unwrap_or(false),
        oauth_user: None,
        shared_mailboxes: Vec::new(),
        online_only: p::bool(v, "onlineOnly").unwrap_or(false),
        dav: None,
        user_agent,
    };
    // A line break or tab pasted in with the user name: no server would ever take it (in an
    // Exchange `DOMAIN\user`, a `\r` read from JSON became part of it, and sign-in failed).
    if config.username.chars().any(char::is_control) {
        return Err(CmdError::Params("the user name has a line break or another invisible character in it".into()));
    }
    if let Some(o) = &config.oauth
        && !s.oauth_available.contains(o)
    {
        return Err(CmdError::Params(format!("signing in with {o} isn't set up in this copy of MailCraft")));
    }
    // `deferSignIn`: add now, ask for the password when the account first connects.
    let defer = p::bool(v, "deferSignIn").unwrap_or(false);
    if !local && !defer && config.oauth.is_none() && password.as_deref().is_none_or(str::is_empty) {
        return Err(CmdError::Params("`password` is required".into()));
    }
    // The outgoing server's own sign-in (IMAP or POP with SMTP).
    let mut config = config;
    let smtp_password = smtp_sign_in(v, &mut config, true)?;
    let id = s.add_account(config, password)?;
    if let Some(pw) = smtp_password {
        s.keep_smtp_password(&id, pw);
    }
    // `pst`: an existing data file whose mail comes into the account first.
    if let Some(path) = p::str(v, "pst").map(str::trim).filter(|p| !p.is_empty()) {
        s.attach_data_file(&id, path)?;
    }
    Ok(json!({"account": id}))
}

/// An Exchange Web Services address from what the user typed: a server name gets `https://` and
/// `/EWS/Exchange.asmx`.
pub(super) fn ews_url(typed: &str) -> String {
    let t = typed.trim().trim_end_matches('/');
    let with_scheme = if t.contains("://") { t.to_string() } else { format!("https://{t}") };
    let after_host = with_scheme.split_once("://").map_or("", |(_, rest)| rest);
    if after_host.contains('/') { with_scheme } else { format!("{with_scheme}/EWS/Exchange.asmx") }
}

/// `userAgent` (issue #9): `None` when it isn't given, `Some(None)` when it's empty (MailCraft's
/// own), else the one to name; refused when it can't go in a header.
fn user_agent_param(v: &Value) -> Result<Option<Option<String>>, CmdError> {
    match v.get("userAgent") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if s.trim().is_empty() => Ok(Some(None)),
        Some(Value::String(s)) => mailcraft_model::user_agent(s).map(|a| Some(Some(a))).ok_or_else(|| {
            CmdError::Params(format!(
                "a User-Agent is up to {} letters, digits, spaces and punctuation (no other characters)",
                mailcraft_model::MAX_USER_AGENT
            ))
        }),
        Some(_) => Err(CmdError::Params("`userAgent` is text".into())),
    }
}

pub(super) fn set_password(s: &mut Session, v: &Value) -> CmdResult {
    let id = p::req_str(v, "account")?;
    let pw = p::req_str(v, "password")?;
    if pw.is_empty() {
        return Err(CmdError::Params("`password` is empty".into()));
    }
    let config = s.account(id).map(|a| a.config.clone()).ok_or_else(|| CmdError::Params(format!("no account `{id}`")))?;
    s.update_account(config, Some(pw.to_string()))
}

pub(super) fn update_account(s: &mut Session, v: &Value) -> CmdResult {
    let id = p::req_str(v, "account")?;
    let mut c = s.account(id).map(|a| a.config.clone()).ok_or_else(|| CmdError::Params(format!("no account `{id}`")))?;
    if c.is_local() {
        return Err(CmdError::Params("the sample mailbox has no server settings".into()));
    }
    if let Some(n) = p::str(v, "name") {
        c.name = n.trim().chars().take(200).collect();
    }
    if let Some(u) = p::str(v, "username").map(str::trim).filter(|u| !u.is_empty()) {
        if u.chars().any(char::is_control) {
            return Err(CmdError::Params("the user name has a line break or another invisible character in it".into()));
        }
        c.username = u.to_string();
    }
    match v.get("jmap") {
        Some(Value::String(u)) if !u.trim().is_empty() => c.jmap = Some(u.trim().to_string()),
        Some(Value::String(_) | Value::Null) => c.jmap = None,
        _ => {}
    }
    if let Some(u) = p::str(v, "ews").map(str::trim).filter(|u| !u.is_empty())
        && c.ews.is_some()
    {
        c.ews = Some(ews_url(u));
    }
    if let Some(agent) = user_agent_param(v)?
        && c.ews.is_some()
    {
        c.user_agent = agent;
    }
    if let Some(pop) = c.pop.clone() {
        c.pop = Some(server(v, "pop", &pop)?);
        c.smtp = server(v, "smtp", &c.smtp)?;
        if let Some(d) = p::bool(v, "popDelete") {
            c.pop_delete = d;
        }
        if let Some(n) = v.get("popKeepDays").and_then(serde_json::Value::as_u64) {
            c.pop_keep_days = (n > 0).then(|| u32::try_from(n).unwrap_or(u32::MAX));
        }
        if let Some(d) = p::bool(v, "popDeleteWithTrash") {
            c.pop_delete_with_trash = d;
        }
    } else if !c.is_api() {
        c.imap = server(v, "imap", &c.imap)?;
        c.smtp = server(v, "smtp", &c.smtp)?;
        if c.imap.host.is_empty() || c.imap.port == 0 || c.smtp.host.is_empty() || c.smtp.port == 0 {
            return Err(CmdError::Params("incoming and outgoing server names and ports are required".into()));
        }
    }
    if let Some(on) = p::bool(v, "onlineOnly") {
        c.online_only = on;
    }
    let smtp_password = smtp_sign_in(v, &mut c, false)?;
    let done = s.update_account(c, None)?;
    if let Some(pw) = smtp_password {
        s.keep_smtp_password(id, pw);
    }
    Ok(done)
}

/// The outgoing server's own sign-in, as Outlook's "Log on using" for SMTP (issue #3: its user
/// name and password aren't the incoming server's): `smtpUsername` (null or empty: signs in as
/// the incoming server does) and `smtpPassword`, which is returned to be kept. A new one needs
/// its password; one already kept keeps it when none is given.
pub(super) fn smtp_sign_in(v: &Value, c: &mut AccountConfig, adding: bool) -> Result<Option<String>, CmdError> {
    let password = p::str(v, "smtpPassword").filter(|p| !p.is_empty()).map(str::to_string);
    let user = match v.get("smtpUsername") {
        None => return Ok(password.filter(|_| c.smtp_username.is_some())),
        Some(Value::Null) => None,
        Some(u) => u.as_str().map(str::trim).filter(|u| !u.is_empty()).map(|u| u.chars().take(320).collect::<String>()),
    };
    let Some(user) = user else {
        c.smtp_username = None;
        return Ok(None);
    };
    if c.is_local() || c.is_api() {
        return Err(CmdError::Params("only accounts that send over SMTP have an outgoing server's sign-in".into()));
    }
    if password.is_none() && (adding || c.smtp_username.is_none()) && !p::bool(v, "deferSignIn").unwrap_or(false) {
        return Err(CmdError::Params("the outgoing server's password is required".into()));
    }
    c.smtp_username = Some(user);
    Ok(password)
}

/// `account.dav`: where the account's calendars and contacts are, given by hand.
pub(super) fn set_dav(s: &mut Session, v: &Value) -> CmdResult {
    let id = p::req_str(v, "account")?;
    let current = s.account(id).and_then(|a| a.config.dav.clone()).unwrap_or_default();
    let caldav = p::str(v, "caldav").map_or(current.caldav.clone(), str::to_string);
    let carddav = p::str(v, "carddav").map_or(current.carddav.clone(), str::to_string);
    let username = match v.get("username") {
        None => current.username.clone(),
        Some(Value::Null) => None,
        Some(u) => u.as_str().map(str::to_string),
    };
    s.set_dav(id, &caldav, &carddav, username.as_deref(), p::str(v, "password"))
}

/// Longest signature HTML taken: its formatting, and its pictures written in.
const SIGNATURE_HTML_MAX: usize = 200_000 + mailcraft_mime::SIGNATURE_PICTURES * (mailcraft_mime::SIGNATURE_PICTURE_MAX / 3 * 4 + 64);

pub(super) fn set_signature(s: &mut Session, v: &Value) -> CmdResult {
    let id = p::req_str(v, "account")?;
    let mut config = s.account(id).map(|a| a.config.clone()).ok_or_else(|| CmdError::Params(format!("no account `{id}`")))?;
    config.signature = p::req_str(v, "signature")?.chars().take(10_000).collect();
    // Its look, as HTML (from the signature editor, or pasted): kept as the editor would send
    // it, with its pictures written in (issue #11), its text the signature's.
    config.signature_html = String::new();
    if let Some(html) = p::str(v, "html").filter(|h| !h.trim().is_empty()) {
        let html: String = html.chars().take(SIGNATURE_HTML_MAX).collect();
        let (text, styles, pictures) = mailcraft_mime::signature_html(&html);
        if !text.trim().is_empty() && !styles.is_plain() {
            config.signature_html = mailcraft_mime::embed_pictures(&mailcraft_mime::rich::to_html(&text, &styles), &pictures);
            config.signature = text.replace(mailcraft_mime::rich::PICTURE, "").chars().take(10_000).collect();
        }
    }
    s.update_account(config, None)
}

pub(super) fn list_accounts(s: &mut Session, _: &Value) -> CmdResult {
    Ok(json!(s
        .accounts
        .iter()
        .map(|a| json!({
            "account": a.config.id,
            "email": a.config.email,
            "name": a.config.name,
            "graph": a.config.graph,
            "pop": a.config.pop.as_ref().map(|p| format!("{}:{} ({})", p.host, p.port, p.security.code())),
            "ews": a.config.ews,
            "userAgent": a.config.user_agent,
            "imap": format!("{}:{} ({})", a.config.imap.host, a.config.imap.port, a.config.imap.security.code()),
            "smtp": format!("{}:{} ({})", a.config.smtp.host, a.config.smtp.port, a.config.smtp.security.code()),
            "status": a.status,
            "folders": a.folders.iter().map(|f| json!({"path": f.path, "name": f.display_name(), "role": f.role.code(), "unread": f.unread, "total": f.total})).collect::<Vec<_>>(),
        }))
        .collect::<Vec<_>>()))
}

/// Add a shared mailbox to an Exchange account by its address (Outlook's "Open these additional
/// mailboxes"), or remove one; the account connects again to list its folders.
pub(super) fn shared_mailbox(s: &mut Session, v: &Value, add: bool) -> CmdResult {
    let id = p::req_str(v, "account")?;
    let mut c = s.account(id).map(|a| a.config.clone()).ok_or_else(|| CmdError::Params(format!("no account `{id}`")))?;
    if c.ews.is_none() {
        return Err(CmdError::Params("shared mailboxes are opened on Exchange accounts (EWS)".into()));
    }
    let address = p::req_str(v, "address")?.trim().to_string();
    let parsed = mailcraft_model::addr::parse_list(&address).map_err(|bad| CmdError::Params(format!("not an email address: {bad}")))?;
    let [one] = parsed.as_slice() else { return Err(CmdError::Params("one address at a time".into())) };
    let email = one.email.trim().to_string();
    let known = c.shared_mailboxes.iter().position(|a| a.eq_ignore_ascii_case(&email));
    if add {
        if email.eq_ignore_ascii_case(&c.email) {
            return Err(CmdError::Params("that's this account's own mailbox".into()));
        }
        if known.is_some() {
            return Err(CmdError::Params(format!("{email} is already open in this account")));
        }
        if c.shared_mailboxes.len() >= 20 {
            return Err(CmdError::Params("at most 20 shared mailboxes".into()));
        }
        c.shared_mailboxes.push(email.clone());
    } else {
        let at = known.ok_or_else(|| CmdError::Params(format!("{email} isn't open in this account")))?;
        c.shared_mailboxes.remove(at);
    }
    s.update_account(c, None)?;
    Ok(json!({ "account": id, "address": email, "added": add }))
}
