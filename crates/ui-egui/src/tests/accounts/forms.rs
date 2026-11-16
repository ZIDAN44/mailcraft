//! UI tests: the add-account and server-settings dialogs for every kind of account (IMAP and
//! SMTP, POP and SMTP, JMAP, Microsoft Graph, EWS, calendars and contacts only): the rows each
//! draws, what it asks for, and the command params it gives.

use egui::{Event, PointerButton, Pos2};
use mailcraft_engine::model::{AccountConfig, Security, ServerConfig};

use super::*;
use crate::Action;
use crate::dialogs::{AccountForm, AccountKind};

/// What a dialog drew (each text and where), what it asked for, and the params it gives then.
struct Look {
    texts: Vec<(String, egui::Rect)>,
    acts: Vec<String>,
    params: serde_json::Value,
}

impl Look {
    fn has(&self, what: &str) -> bool {
        self.texts.iter().any(|(t, _)| t == what)
    }

    /// Every text and where it was drawn, a line each.
    fn dump(&self) -> String {
        let mut s = String::new();
        for (t, r) in &self.texts {
            s.push_str(&format!("{t:?} {:.1},{:.1} {:.1}x{:.1}\n", r.min.x, r.min.y, r.width(), r.height()));
        }
        for a in &self.acts {
            s.push_str(&format!("act {a}\n"));
        }
        s.push_str(&format!("params {}\n", self.params));
        s
    }
}

fn act_text(a: &Action) -> String {
    match a {
        Action::Run(id, v) => format!("run {id} {v}"),
        Action::Submit(id, v) => format!("submit {id} {v}"),
        Action::PickAccountDataFile => "pick a data file".into(),
        _ => "another action".into(),
    }
}

fn params(d: &Dialog) -> serde_json::Value {
    match d {
        Dialog::AddAccount(f) if f.calendar_only => f.calendar_params(),
        Dialog::AddAccount(f) => f.params(),
        Dialog::EditAccount(f) => f.params(),
        _ => serde_json::Value::Null,
    }
}

/// The dialog drawn alone (three times: a dialog just opened is measured first, then centred by
/// that size, then by its own), then, when `click` names a text it drew, clicked there.
fn look(s: &Session, d: &mut Dialog, click: Option<&str>) -> Look {
    let ctx = egui::Context::default();
    let t = crate::theme::Tokens::light();
    let mut pics = crate::reading::Pictures::default();
    let mut acts = Vec::new();
    let mut run = |d: &mut Dialog, events: Vec<Event>, acts: &mut Vec<Action>| {
        let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1280.0, 1600.0))), events, ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| {
            crate::dialogs::show(ui.ctx(), d, s, &mut pics, &t, acts);
        });
        out.textures_delta.clear();
        texts_in(&out.shapes).into_iter().map(|d| (d.text, d.rect)).collect::<Vec<_>>()
    };
    run(d, vec![], &mut acts);
    run(d, vec![], &mut acts);
    let mut texts = run(d, vec![], &mut acts);
    // Each text in turn, "a › b" for b in what clicking a showed (a list's choice).
    for what in click.into_iter().flat_map(|c| c.split(" › ")) {
        // Near the text's right end: a link wrapped after other words is laid out from the row's
        // start, so the middle of its text's box can be on the words before it.
        let at = texts.iter().rev().find(|(x, _)| x == what).map(|(_, r)| r.right_center() - egui::vec2(6.0, 0.0)).unwrap_or_else(|| {
            panic!("no {what:?}:\n{}", Look { texts: texts.clone(), acts: acts.iter().map(act_text).collect(), params: params(d) }.dump())
        });
        let press = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        run(d, vec![Event::PointerMoved(at)], &mut acts);
        run(d, vec![press(true)], &mut acts);
        run(d, vec![press(false)], &mut acts);
        // Twice: a list just opened is measured first, unseen.
        run(d, vec![], &mut acts);
        texts = run(d, vec![], &mut acts);
    }
    Look { texts, acts: acts.iter().map(act_text).collect(), params: params(d) }
}

fn session() -> Session {
    Session::open(None, Clock::new(NOW, 0))
}

fn add(email: &str, advanced: bool, fill: impl FnOnce(&mut AccountForm)) -> Dialog {
    let mut d = Dialog::add_account_with(email, advanced);
    if let Dialog::AddAccount(f) = &mut d {
        f.name = " Zoe Park ".into();
        f.password = "pw".into();
        fill(f);
    }
    d
}

fn edit(fill: impl FnOnce(&mut AccountConfig)) -> Dialog {
    let mut c = AccountConfig::guess("zoe@example.org");
    c.id = "zoe".into();
    c.name = "Zoe Park".into();
    c.username = "zoe".into();
    fill(&mut c);
    Dialog::edit_account(&c)
}

/// A dialog as it's filled in and used: its name, the session, the dialog and what to click.
type Case = (&'static str, Session, Dialog, Option<&'static str>);

/// Every kind of account, added and changed.
fn cases() -> Vec<Case> {
    let plain = session;
    let with_oauth = || {
        let mut s = session();
        s.oauth_available = vec!["microsoft".into()];
        s
    };
    let found = |answer: Option<(Option<AccountConfig>, String)>| {
        let mut s = session();
        s.discovered.insert("zoe@example.org".into(), answer);
        s
    };
    let mut provider = AccountConfig::guess("zoe@example.org");
    provider.imap.host = "imap.provider.example".into();
    provider.smtp.host = "out.example.org".into();
    provider.smtp.port = 587;
    provider.smtp.security = Security::StartTls;
    provider.username = "ZOE@example.org".into();
    let mut provider_pop = provider.clone();
    provider_pop.username = "zoe.park".into();
    provider_pop.pop = Some(ServerConfig { host: "pop3.example.org".into(), port: 110, security: Security::StartTls });
    let browser = || {
        let mut s = session();
        s.jmap_only = true;
        s
    };
    let portable = || {
        let mut s = session();
        s.portable = true;
        s
    };
    vec![
        (
            "imap",
            plain(),
            add("zoe@example.org", true, |f| {
                f.smtp_sign_in.own = true;
                f.smtp_sign_in.username = " out ".into();
                f.smtp_sign_in.password = "spw".into();
            }),
            Some("Connect"),
        ),
        (
            "imap in the clear",
            plain(),
            add("zoe@example.org", true, |f| {
                f.imap_security = Security::None;
                f.imap_port = " 143 ".into();
                f.smtp_security = Security::StartTls;
                f.username = " zoe ".into();
                f.online_only = Some(false);
            }),
            None,
        ),
        ("imap, settings closed", plain(), add("zoe@example.org", false, |_| {}), Some("Connect")),
        (
            "imap, no servers yet",
            plain(),
            add("zoe@example.org", false, |f| {
                f.imap_host.clear();
                f.smtp_host.clear();
            }),
            Some("Connect"),
        ),
        ("imap, settings found", found(Some((Some(provider), "the provider".into()))), add("zoe@example.org", true, |_| {}), None),
        (
            "pop, settings found",
            found(Some((Some(provider_pop), "the provider".into()))),
            add("zoe@example.org", true, |f| f.kind = AccountKind::Pop),
            None,
        ),
        ("looking the settings up", found(None), add("zoe@example.org", false, |_| {}), None),
        ("no settings found", found(Some((None, "x".into()))), add("zoe@example.org", false, |_| {}), None),
        (
            "pop",
            plain(),
            add("zoe@example.org", true, |f| {
                f.kind = AccountKind::Pop;
                f.pop_host = " pop.example.org ".into();
                f.pop_port = "995".into();
                f.smtp_security = Security::None;
                f.pst = " C:/Mail/old.pst ".into();
                f.pop_keep.days = " 30 ".into();
                f.pop_keep.until_deleted = true;
                f.online_only = Some(true);
            }),
            Some("Connect"),
        ),
        (
            "pop, no server yet",
            plain(),
            add("zoe@example.org", false, |f| {
                f.kind = AccountKind::Pop;
                f.pop_host.clear();
                f.pop_keep.keep = false;
            }),
            Some("Connect"),
        ),
        (
            "pop, settings closed",
            plain(),
            add("zoe@example.org", false, |f| {
                f.kind = AccountKind::Pop;
                f.pop_keep.for_days = false;
            }),
            None,
        ),
        (
            "imap, switched to pop",
            plain(),
            add("zoe@example.org", true, |f| {
                f.imap_host = "imap.mail.example.org".into();
                f.pop_host.clear();
            }),
            Some("IMAP and SMTP › POP and SMTP"),
        ),
        ("pop, switched to jmap", plain(), add("zoe@example.org", true, |f| f.kind = AccountKind::Pop), Some("POP and SMTP › JMAP")),
        ("pop, copy on the server opened", plain(), add("zoe@example.org", true, |f| f.kind = AccountKind::Pop), Some("A copy on the server")),
        ("pop, data file opened", plain(), add("zoe@example.org", true, |f| f.kind = AccountKind::Pop), Some("Existing data file (optional)")),
        (
            "jmap",
            plain(),
            add("zoe@fastmail.example", true, |f| {
                f.kind = AccountKind::Jmap;
                f.jmap_url = " https://api.fastmail.example/jmap/session ".into();
                f.online_only = Some(true);
            }),
            Some("Connect"),
        ),
        (
            "jmap, settings closed",
            plain(),
            add("zoe@fastmail.example", false, |f| {
                f.kind = AccountKind::Jmap;
                f.jmap_url = "https://api.fastmail.example/jmap/session".into();
                f.username = "zoe".into();
            }),
            None,
        ),
        ("jmap, no address", plain(), add("zoe@fastmail.example", false, |f| f.kind = AccountKind::Jmap), None),
        (
            "graph",
            with_oauth(),
            add("ana@contoso.example", true, |f| {
                f.kind = AccountKind::Graph;
                f.online_only = Some(true);
            }),
            Some("Sign in with Microsoft"),
        ),
        ("graph, no Microsoft sign-in", plain(), add("ana@contoso.example", false, |f| f.kind = AccountKind::Graph), None),
        (
            "ews",
            plain(),
            add("ana@contoso.example", true, |f| {
                f.kind = AccountKind::Ews;
                f.imap_host.clear();
                f.user_agent = "bad\u{1}agent".into();
            }),
            Some("Connect"),
        ),
        (
            "ews, settings closed",
            plain(),
            add("ana@contoso.example", false, |f| {
                f.kind = AccountKind::Ews;
                f.ews_url = " mail.contoso.example ".into();
                f.user_agent = " Outlook/16.0 ".into();
                f.username = "CONTOSO\\ana".into();
            }),
            None,
        ),
        ("outlook.com without Microsoft sign-in", plain(), add("ana@outlook.com", false, |_| {}), None),
        ("outlook.com", with_oauth(), add("ana@outlook.com", false, |_| {}), Some("Sign in with Microsoft")),
        ("outlook.com over ews", with_oauth(), add("ana@outlook.com", false, |f| f.kind = AccountKind::Ews), None),
        ("in the browser", browser(), add("zoe@fastmail.example", false, |_| {}), None),
        ("portable", portable(), add("zoe@example.org", false, |_| {}), Some("Connect")),
        ("to calendars only", plain(), add("zoe@example.org", false, |_| {}), Some("Add a CalDAV and CardDAV account")),
        (
            "calendars and contacts only",
            plain(),
            add("zoe@example.org", false, |f| {
                f.kind = AccountKind::Pop;
                f.calendar_only = true;
                f.dav_url = " https://cloud.example.org/remote.php/dav ".into();
                f.username = " zoe ".into();
            }),
            Some("Connect"),
        ),
        (
            "back from calendars only",
            plain(),
            add("zoe@example.org", true, |f| {
                f.kind = AccountKind::Pop;
                f.calendar_only = true;
            }),
            Some("Add an email account instead"),
        ),
        (
            "edit imap",
            plain(),
            edit(|c| {
                c.imap.security = Security::None;
                c.smtp_username = Some("out".into());
                c.online_only = true;
                // Issue #123: NTLM in, no sign-in out.
                c.sign_in = mailcraft_engine::model::SignInMethod::Ntlm;
                c.smtp_sign_in = mailcraft_engine::model::SignInMethod::None;
            }),
            Some("Save"),
        ),
        ("edit imap, switched to jmap", plain(), edit(|_| {}), Some("JMAP")),
        ("edit imap signed in with google", plain(), edit(|c| c.oauth = Some("google".into())), Some("Save")),
        ("edit jmap", plain(), edit(|c| c.jmap = Some("https://api.example.org/jmap/session".into())), Some("Save")),
        ("edit jmap, switched to imap", plain(), edit(|c| c.jmap = Some("https://api.example.org/jmap/session".into())), Some("IMAP and SMTP")),
        (
            "edit pop",
            plain(),
            edit(|c| {
                c.pop = Some(ServerConfig { host: "pop.example.org".into(), port: 995, security: Security::Tls });
                c.pop_keep_days = Some(7);
                c.smtp_username = Some("out".into());
            }),
            Some("Save"),
        ),
        (
            "edit pop in the clear",
            plain(),
            edit(|c| {
                c.pop = Some(ServerConfig { host: "pop.example.org".into(), port: 110, security: Security::None });
                c.pop_delete = true;
            }),
            None,
        ),
        (
            "edit pop, sending in the clear",
            plain(),
            edit(|c| {
                c.pop = Some(ServerConfig { host: "pop.example.org".into(), port: 995, security: Security::Tls });
                c.smtp.security = Security::None;
            }),
            None,
        ),
        (
            "edit ews",
            plain(),
            edit(|c| {
                c.ews = Some("https://mail.contoso.example/EWS/Exchange.asmx".into());
                c.user_agent = Some("Outlook/16.0".into());
            }),
            Some("Save"),
        ),
    ]
}

/// Each kind of account draws its own rows, asks for what it should and gives its own params.
#[test]
fn every_kind_of_account_draws_its_rows_and_gives_its_params() {
    let mut looks = Vec::new();
    for (name, s, mut d, click) in cases() {
        looks.push((name, look(&s, &mut d, click)));
    }
    let get = |name: &str| looks.iter().find(|(n, _)| *n == name).map(|(_, l)| l).unwrap();
    let rows = |name: &str, want: &[&str], not: &[&str]| {
        let l = get(name);
        for w in want {
            assert!(l.has(w), "{name}: no {w:?}\n{}", l.dump());
        }
        for n in not {
            assert!(!l.has(n), "{name}: {n:?}\n{}", l.dump());
        }
    };
    let clear = "Without encryption your password travels in the clear. Use only for local test servers.";
    let imap_rows = ["Incoming mail (IMAP)", "Outgoing mail (SMTP)", "User name", "mail.example.org", "993", "465", "Sign in with"];
    rows("imap", &imap_rows, &["Incoming mail (POP)", clear, "JMAP session address"]);
    rows("imap in the clear", &[clear, "None (unsafe)", "STARTTLS"], &[]);
    rows("pop", &["Incoming mail (POP)", "Outgoing mail (SMTP)", clear], &["Incoming mail (IMAP)"]);
    // What most leave as it is, POP's copy on the server and a data file to start from, in parts
    // of their own, closed, saying what they're set to; opened, their choices.
    let copy = "Keep a copy of downloaded mail on the server";
    let pst_hint = "Mail from an old .pst or .mailcraft file comes in first";
    rows(
        "pop",
        &["A copy on the server", "Kept for 30 days, or until deleted from Deleted Items", "Existing data file (optional)", "C:/Mail/old.pst"],
        &[copy, "Browse…", pst_hint],
    );
    rows("pop, copy on the server opened", &[copy, "Remove it from the server after"], &["Browse…"]);
    rows("pop, data file opened", &["Browse…", "Kept for 14 days"], &[copy]);
    // The kind of server is a list: closed, only the one chosen shows.
    rows("imap", &["Mail server", "IMAP and SMTP"], &["POP and SMTP", "Exchange server (EWS)"]);
    rows("jmap", &["JMAP session address", "User name"], &["Outgoing mail (SMTP)", "Incoming mail (IMAP)"]);
    rows("graph", &["Sign in with Microsoft"], &["Password", "User name", "Outgoing mail (SMTP)"]);
    rows("graph, no Microsoft sign-in", &["Exchange (Microsoft 365)", "Signing in with Microsoft isn't set up in this copy of MailCraft."], &[]);
    rows(
        "ews",
        &["Exchange server", "User name", "User-Agent (optional)", "Only letters, digits, spaces and punctuation, up to 256 characters."],
        &[],
    );
    rows("ews, settings closed", &["Server: mail.contoso.example (EWS)"], &[]);
    rows("jmap, settings closed", &["Server: https://api.fastmail.example/jmap/session (JMAP)"], &[]);
    rows("imap, settings closed", &["Servers: mail.example.org (IMAP 993), mail.example.org (SMTP 465)"], &[]);
    rows("pop, settings closed", &["Servers: mail.example.org (POP 995), mail.example.org (SMTP 465)"], &[]);
    rows("imap, settings found", &["Settings found: the provider", "imap.provider.example", "out.example.org", "587"], &[]);
    rows("pop, settings found", &["pop3.example.org", "110", "zoe.park"], &[]);
    rows("looking the settings up", &["Looking up the server settings…"], &[]);
    rows(
        "calendars and contacts only",
        &["Add calendars and contacts", "Calendar and contacts server", "Add an email account instead"],
        &["Server settings"],
    );
    rows("to calendars only", &["Add calendars and contacts", "Calendar and contacts server"], &["Add an email account", "Server settings"]);
    // The way to calendars alone is offered under the email account's form, not before it.
    rows("imap, settings closed", &["Only calendars and contacts, with no email?", "Add a CalDAV and CardDAV account"], &[]);
    // Each kind says what it is; Microsoft 365 has no servers to look up, so nothing is said of that.
    rows("imap", &["Mail stays on the server and looks the same on every device."], &[]);
    rows("pop", &["Mail is downloaded to this computer. The server can keep a copy for a while."], &[]);
    rows("jmap", &["A newer way to reach your mail, offered by some providers such as Fastmail."], &[]);
    rows("graph", &[], &["Connect", "No published settings: the usual server names are filled in. Check them under Server settings."]);
    rows("back from calendars only", &["Incoming mail (POP)"], &["Calendar and contacts server"]);
    rows("outlook.com", &["Sign in with Microsoft"], &["Password"]);
    rows("outlook.com over ews", &["Password", "Sign in with Microsoft"], &[]);
    rows("in the browser", &["JMAP session address"], &["Server settings", "IMAP and SMTP"]);
    rows("edit imap", &imap_rows, &["JMAP session address"]);
    // No sign-in for sending (#123): its own user name for that has nothing to do, and isn't shown.
    rows("edit imap", &[clear], &["The outgoing server has its own user name and password"]);
    rows("edit pop", &["The outgoing server has its own user name and password", "out"], &[]);
    rows("edit imap", &["NTLM (Secure Password Authentication)", "No sign-in"], &["Automatic"]);
    rows("imap", &["Automatic"], &["NTLM (Secure Password Authentication)", "No sign-in"]);
    // Signed in through the browser: no password, so no way to sign in with one to choose.
    rows("edit imap signed in with google", &["Outgoing mail (SMTP)"], &["Sign in with"]);
    rows("edit imap, switched to jmap", &["JMAP session address"], &["Incoming mail (IMAP)", "Outgoing mail (SMTP)"]);
    rows("edit jmap, switched to imap", &["Incoming mail (IMAP)", "Outgoing mail (SMTP)"], &["JMAP session address"]);
    rows("edit pop", &["Incoming mail (POP)", "Outgoing mail (SMTP)", "pop.example.org", "A copy on the server", "Kept for 7 days"], &[clear, copy]);
    // A POP account's servers without encryption are warned of, as when it was added.
    rows("edit pop in the clear", &[clear], &[]);
    rows("edit pop, sending in the clear", &[clear], &[]);
    rows("edit ews", &["Exchange server", "User-Agent (optional)", "Outlook/16.0"], &["Outgoing mail (SMTP)"]);
    let outlook = get("outlook.com without Microsoft sign-in");
    assert!(outlook.texts.iter().any(|(t, _)| t.starts_with("Outlook.com, Hotmail and Live addresses only connect")), "{}", outlook.dump());

    // What each asks for, and the params it gives.
    let acts = |name: &str| get(name).acts.clone();
    let submitted = |name: &str| {
        let l = get(name);
        let p = l.acts.iter().rev().find_map(|a| a.strip_prefix("submit ")).unwrap_or_else(|| panic!("{name}: nothing sent\n{}", l.dump()));
        let (id, v) = p.split_once(' ').unwrap();
        (id.to_string(), serde_json::from_str::<serde_json::Value>(v).unwrap())
    };
    // The settings are looked up as the server settings are open (once a frame, until found).
    assert_eq!(acts("imap in the clear"), vec![r#"run account.discover {"email":"zoe@example.org"}"#; 3]);
    assert_eq!(
        submitted("imap"),
        (
            "account.add".into(),
            json!({"email": "zoe@example.org", "name": "Zoe Park", "password": "pw", "username": "zoe@example.org",
                "imapHost": "mail.example.org", "imapPort": 993, "imapSecurity": "tls",
                "smtpHost": "mail.example.org", "smtpPort": 465, "smtpSecurity": "tls",
                "smtpUsername": "out", "signIn": "automatic", "smtpSignIn": "automatic", "smtpPassword": "spw"})
        )
    );
    assert_eq!(
        get("imap in the clear").params,
        json!({"email": "zoe@example.org", "name": "Zoe Park", "password": "pw", "username": "zoe",
            "imapHost": "mail.example.org", "imapPort": 143, "imapSecurity": "none",
            "smtpHost": "mail.example.org", "smtpPort": 465, "smtpSecurity": "starttls", "smtpUsername": null, "signIn": "automatic", "smtpSignIn": "automatic"})
    );
    let guessed = json!({"email": "zoe@example.org", "name": "Zoe Park", "password": "pw", "username": "zoe@example.org",
        "imapHost": "mail.example.org", "imapPort": 993, "imapSecurity": "tls",
        "smtpHost": "mail.example.org", "smtpPort": 465, "smtpSecurity": "tls", "smtpUsername": null, "signIn": "automatic", "smtpSignIn": "automatic"});
    assert_eq!(submitted("imap, settings closed"), ("account.add".into(), guessed.clone()));
    assert_eq!(submitted("imap, no servers yet"), ("account.add".into(), guessed.clone()));
    let mut on = guessed.clone();
    on["onlineOnly"] = json!(true);
    assert_eq!(submitted("portable"), ("account.add".into(), on));
    assert_eq!(
        get("imap, settings found").params,
        json!({"email": "zoe@example.org", "name": "Zoe Park", "password": "pw", "username": "zoe@example.org",
            "imapHost": "imap.provider.example", "imapPort": 993, "imapSecurity": "tls",
            "smtpHost": "out.example.org", "smtpPort": 587, "smtpSecurity": "starttls", "smtpUsername": null, "signIn": "automatic", "smtpSignIn": "automatic"})
    );
    assert_eq!(
        get("pop, settings found").params,
        json!({"email": "zoe@example.org", "name": "Zoe Park", "password": "pw", "username": "zoe.park", "pop": true,
            "popHost": "pop3.example.org", "popPort": 110, "popSecurity": "starttls",
            "smtpHost": "out.example.org", "smtpPort": 587, "smtpSecurity": "starttls",
            "popDelete": false, "popKeepDays": 14, "popDeleteWithTrash": false, "smtpUsername": null, "signIn": "automatic", "smtpSignIn": "automatic"})
    );
    assert_eq!(
        submitted("pop"),
        (
            "account.add".into(),
            json!({"email": "zoe@example.org", "name": "Zoe Park", "password": "pw", "username": "zoe@example.org", "pop": true,
                "popHost": "pop.example.org", "popPort": 995, "popSecurity": "tls",
                "smtpHost": "mail.example.org", "smtpPort": 465, "smtpSecurity": "none",
                "popDelete": false, "popKeepDays": 30, "popDeleteWithTrash": true, "smtpUsername": null, "signIn": "automatic", "smtpSignIn": "automatic",
                "pst": "C:/Mail/old.pst"})
        )
    );
    assert_eq!(
        submitted("pop, no server yet"),
        (
            "account.add".into(),
            json!({"email": "zoe@example.org", "name": "Zoe Park", "password": "pw", "username": "zoe@example.org", "pop": true,
                "popHost": "mail.example.org", "popPort": 995, "popSecurity": "tls",
                "smtpHost": "mail.example.org", "smtpPort": 465, "smtpSecurity": "tls",
                "popDelete": true, "popKeepDays": 0, "popDeleteWithTrash": false, "smtpUsername": null, "signIn": "automatic", "smtpSignIn": "automatic"})
        )
    );
    assert_eq!(get("pop, settings closed").params["popKeepDays"], json!(0));
    assert_eq!(get("imap, switched to pop").params["popHost"], json!("pop.mail.example.org"));
    assert_eq!(get("imap, switched to pop").params["pop"], json!(true));
    assert_eq!(get("pop, switched to jmap").params["jmap"], json!(""));
    assert_eq!(
        submitted("jmap"),
        (
            "account.add".into(),
            json!({"email": "zoe@fastmail.example", "name": "Zoe Park", "password": "pw", "username": "zoe@fastmail.example",
                "jmap": "https://api.fastmail.example/jmap/session", "onlineOnly": true})
        )
    );
    assert_eq!(get("jmap, settings closed").params["username"], json!("zoe"));
    assert_eq!(
        submitted("graph"),
        ("account.add".into(), json!({"email": "ana@contoso.example", "name": "Zoe Park", "graph": true, "oauth": "microsoft", "onlineOnly": true}))
    );
    assert_eq!(
        submitted("ews"),
        (
            "account.add".into(),
            json!({"email": "ana@contoso.example", "name": "Zoe Park", "password": "pw", "username": "ana@contoso.example",
                "ews": true, "userAgent": "bad\u{1}agent"})
        )
    );
    assert_eq!(
        get("ews, settings closed").params,
        json!({"email": "ana@contoso.example", "name": "Zoe Park", "password": "pw", "username": "CONTOSO\\ana",
            "ews": "mail.contoso.example", "userAgent": "Outlook/16.0"})
    );
    assert_eq!(
        submitted("outlook.com"),
        (
            "account.add".into(),
            json!({"email": "ana@outlook.com", "name": "Zoe Park", "username": "ana@outlook.com",
                "imapHost": "outlook.office365.com", "imapPort": 993, "imapSecurity": "tls",
                "smtpHost": "smtp.office365.com", "smtpPort": 587, "smtpSecurity": "starttls", "smtpUsername": null, "oauth": "microsoft"})
        )
    );
    assert_eq!(get("in the browser").params["jmap"], json!(""));
    assert_eq!(
        submitted("calendars and contacts only"),
        (
            "account.add".into(),
            json!({"email": "zoe@example.org", "name": "Zoe Park", "username": "zoe", "password": "pw", "dav": "https://cloud.example.org/remote.php/dav"})
        )
    );
    assert_eq!(get("back from calendars only").params["pop"], json!(true));
    assert_eq!(
        submitted("edit imap"),
        (
            "account.update".into(),
            json!({"account": "zoe", "name": "Zoe Park", "username": "zoe", "jmap": null,
                "imapHost": "mail.example.org", "imapPort": 993, "imapSecurity": "none",
                "smtpHost": "mail.example.org", "smtpPort": 465, "smtpSecurity": "tls", "smtpUsername": "out", "signIn": "ntlm", "smtpSignIn": "none", "onlineOnly": true})
        )
    );
    assert_eq!(get("edit imap, switched to jmap").params["jmap"], json!(""));
    let google = submitted("edit imap signed in with google").1;
    assert!(google.get("signIn").is_none() && google.get("smtpSignIn").is_none(), "{google}");
    assert_eq!(
        submitted("edit jmap"),
        (
            "account.update".into(),
            json!({"account": "zoe", "name": "Zoe Park", "username": "zoe", "jmap": "https://api.example.org/jmap/session", "onlineOnly": false})
        )
    );
    assert_eq!(get("edit jmap, switched to imap").params["jmap"], json!(null));
    assert_eq!(
        submitted("edit pop"),
        (
            "account.update".into(),
            json!({"account": "zoe", "name": "Zoe Park", "username": "zoe",
                "popHost": "pop.example.org", "popPort": 995, "popSecurity": "tls",
                "smtpHost": "mail.example.org", "smtpPort": 465, "smtpSecurity": "tls",
                "popDelete": false, "popKeepDays": 7, "popDeleteWithTrash": false, "smtpUsername": "out", "signIn": "automatic", "smtpSignIn": "automatic"})
        )
    );
    assert_eq!(
        submitted("edit ews"),
        (
            "account.update".into(),
            json!({"account": "zoe", "name": "Zoe Park", "username": "zoe",
                "ews": "https://mail.contoso.example/EWS/Exchange.asmx", "userAgent": "Outlook/16.0", "onlineOnly": false})
        )
    );
}
