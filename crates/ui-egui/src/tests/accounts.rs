//! UI tests: the account forms, the File page and its options.

use super::*;

pub(super) mod compacting;
mod forms;
mod held_keys;
pub(super) mod importing;
mod offers;

pub(super) fn jmap_form() -> Dialog {
    let mut d = Dialog::add_account_with("zoe@fastmail.example", true);
    if let Dialog::AddAccount(f) = &mut d {
        f.kind = crate::dialogs::AccountKind::Jmap;
        f.jmap_url = " https://api.fastmail.example/jmap/session".into();
        f.password = "pw".into();
    }
    d
}

pub(super) fn pop_form() -> Dialog {
    let mut d = Dialog::add_account_with("ana@example.org", true);
    if let Dialog::AddAccount(f) = &mut d {
        f.imap_host = "imap.example.org".into();
        f.smtp_host = "smtp.example.org".into();
        f.kind = crate::dialogs::AccountKind::Pop;
        f.pop_host = "pop.example.org".into();
        f.pop_port = "995".into();
        f.pop_security = mailcraft_engine::model::Security::Tls;
        f.pst = "C:/Mail/old.pst".into();
        f.password = "pw".into();
    }
    d
}

pub(super) fn calendar_form() -> Dialog {
    let mut d = Dialog::add_account();
    if let Dialog::AddAccount(f) = &mut d {
        f.calendar_only = true;
        f.dav_url = " https://cloud.example.org/remote.php/dav ".into();
        f.email = "zoe@example.org".into();
        f.password = "app-pw".into();
    }
    d
}

pub(super) fn graph_form() -> Dialog {
    let mut d = Dialog::add_account_with("ana@contoso.example", true);
    if let Dialog::AddAccount(f) = &mut d {
        f.kind = crate::dialogs::AccountKind::Graph;
    }
    d
}

#[test]
fn the_account_form_adds_exchange_accounts() {
    let (mut a, _) = app("graph-form");
    let p = match graph_form() {
        Dialog::AddAccount(f) => f.params(),
        _ => serde_json::Value::Null,
    };
    assert_eq!((p["graph"].clone(), p.get("password")), (serde_json::json!(true), None), "Microsoft sign-in, no password");
    a.session.oauth_available = vec!["microsoft".into()];
    let mut p = p;
    p["oauth"] = serde_json::json!("microsoft");
    a.act(crate::Action::Submit("account.add", p));
    let added = a.session.accounts.iter().find(|x| x.config.email == "ana@contoso.example").unwrap();
    assert!(added.config.graph.is_some() && added.config.oauth.is_some());
}

#[test]
fn the_file_page_shows_accounts_replies_space_and_options() {
    let (mut a, _) = app("backstage");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    backstage_tour(&mut a, &ctx);
    // A server account: its automatic replies are asked for once, and shown when they arrive.
    let id = a.session.run("account.add", &json!({"email": "zoe@example.org", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    a.session.take_jobs();
    a.act(crate::Action::Backstage(crate::backstage::Page::Info));
    if let Some(b) = a.ui.backstage.as_mut() {
        b.account = Some(id.clone());
    }
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    assert!(matches!(a.session.account(&id).unwrap().vacation, mailcraft_engine::VacationState::Loading), "asked the server");
    a.session.apply(mailcraft_engine::model::Event::Vacation {
        account: id.clone(),
        vacation: Some(mailcraft_engine::model::job::Vacation {
            enabled: true,
            text: "Away".into(),
            until: Some("2026-10-20".into()),
            ..Default::default()
        }),
        error: None,
    });
    a.session.apply(mailcraft_engine::model::Event::Quota { account: id.clone(), used: 3 << 30, limit: 15 << 30 });
    frame(&ctx, &mut a, vec![]);
    // The replies dialog saves through the command; bad dates are reported in it.
    let mut d = Dialog::vacation(&id, &Default::default(), false);
    if let Dialog::Vacation(f) = &mut d {
        f.enabled = true;
        f.text = "Back Monday".into();
        f.dated = true;
        f.until = "20 October".into();
    }
    a.ui.dialog = Some(d);
    let p = match &a.ui.dialog {
        Some(Dialog::Vacation(f)) => f.params(),
        _ => serde_json::Value::Null,
    };
    a.act(crate::Action::Submit("account.setVacation", p));
    assert!(matches!(&a.ui.dialog, Some(Dialog::Vacation(f)) if f.error.as_deref().is_some_and(|e| e.contains("not a date"))));
    frame(&ctx, &mut a, vec![key(Key::Escape, Modifiers::NONE)]);
    // Escape leaves the File page too.
    frame(&ctx, &mut a, vec![key(Key::Escape, Modifiers::NONE)]);
    assert!(a.ui.backstage.is_none() && a.ui.dialog.is_none());
}

/// An Exchange account's automatic replies: who outside the organisation gets one is shown as the
/// server keeps it and saved as `account.setVacation` takes it; a reply of their own goes only to
/// someone.
#[test]
fn automatic_replies_keep_who_outside_gets_one() {
    use crate::dialogs::Outside;
    for (kept, shown, sent) in [("", Outside::Anyone, "all"), ("known", Outside::Known, "known"), ("none", Outside::NoOne, "none")] {
        let v = mailcraft_engine::model::job::Vacation {
            enabled: true,
            text: "Away".into(),
            external: Some("Out".into()),
            outside: kept.into(),
            ..Default::default()
        };
        let Dialog::Vacation(f) = Dialog::vacation("sample", &v, true) else { panic!("not the replies dialog") };
        assert_eq!(f.outside, shown, "{kept:?}");
        let p = f.params();
        assert_eq!(p["outside"], sent, "{kept:?}");
        assert_eq!(p.get("external").is_some(), sent != "none", "{kept:?}");
    }
}

/// Issue #61: calendars and contacts only, from a Nextcloud or Radicale address, with no email.
#[test]
fn the_account_form_adds_calendar_only_accounts() {
    let (mut a, _) = app("calendar-form");
    let p = match calendar_form() {
        Dialog::AddAccount(f) => f.calendar_params(),
        _ => serde_json::Value::Null,
    };
    assert_eq!(
        (p["dav"].clone(), p["email"].clone(), p["username"].clone()),
        (json!("https://cloud.example.org/remote.php/dav"), json!("zoe@example.org"), json!(""))
    );
    a.act(crate::Action::Submit("account.add", p));
    let added = a.session.accounts.iter().find(|x| x.config.email == "zoe@example.org").unwrap();
    assert!(added.config.is_calendar_only());
    assert_eq!(added.config.username, "zoe@example.org", "signs in with the address when no user name is given");
    let id = added.config.id.clone();
    // Issue #86: it has no mailbox, so the folder pane doesn't list it; File › Info does.
    a.run("view.module", json!({"name": "mail"}));
    let ctx = egui::Context::default();
    let size = egui::vec2(1280.0, 800.0);
    drawn_text(&ctx, &mut a, size);
    let pane: Vec<String> = drawn_strings(&ctx, &mut a, size);
    assert!(pane.iter().any(|t| t.contains("(sample)")), "{pane:?}");
    assert!(!pane.iter().any(|t| t.contains("zoe@example.org") || t == "Calendars and contacts"), "{pane:?}");
    assert!(!pane.iter().any(|t| t == "All Inboxes"), "one mailbox: nothing to gather, {pane:?}");
    a.act(crate::Action::Backstage(crate::backstage::Page::Info));
    // (Picked in the page's list of accounts.)
    if let Some(page) = a.ui.backstage.as_mut() {
        page.account = Some(id);
    }
    drawn_text(&ctx, &mut a, size);
    let info: Vec<String> = drawn_strings(&ctx, &mut a, size);
    assert!(info.iter().any(|t| t.contains("zoe@example.org")), "{info:?}");
    // With no mailbox at all, the pane says where what there is can be found.
    a.ui.backstage = None;
    a.session.accounts.retain(|x| x.config.is_calendar_only());
    drawn_text(&ctx, &mut a, size);
    let pane: Vec<String> = drawn_strings(&ctx, &mut a, size);
    assert!(pane.iter().any(|t| t == "Calendars and contacts") && !pane.iter().any(|t| t.contains("zoe@example.org")), "{pane:?}");
}

#[test]
fn the_account_form_adds_jmap_accounts() {
    let (mut a, _) = app("jmap-form");
    let p = match jmap_form() {
        Dialog::AddAccount(f) => f.params(),
        _ => serde_json::Value::Null,
    };
    assert_eq!(p["jmap"], "https://api.fastmail.example/jmap/session");
    assert!(p.get("imapHost").is_none(), "no IMAP servers for a JMAP account");
    a.act(crate::Action::Submit("account.add", p));
    let added = a.session.accounts.iter().find(|x| x.config.email == "zoe@fastmail.example").unwrap();
    assert!(added.config.jmap.is_some() && !added.is_local());
}

#[test]
fn the_account_form_adds_pop_accounts_with_their_old_data_file() {
    let (mut a, _) = app("pop-form");
    let p = match pop_form() {
        Dialog::AddAccount(f) => f.params(),
        _ => serde_json::Value::Null,
    };
    assert_eq!((p["pop"].clone(), p["popHost"].clone(), p["popPort"].clone()), (json!(true), json!("pop.example.org"), json!(995)));
    assert_eq!((p["popDelete"].clone(), p["pst"].clone()), (json!(false), json!("C:/Mail/old.pst")), "a copy stays on the server");
    assert_eq!((p["popKeepDays"].clone(), p["popDeleteWithTrash"].clone()), (json!(14), json!(false)), "for 14 days, as Outlook's default");
    assert!(p.get("imapHost").is_none(), "no IMAP server for a POP account");
    a.act(crate::Action::Submit("account.add", p));
    let added = a.session.accounts.iter().find(|x| x.config.email == "ana@example.org").unwrap();
    assert!(added.config.pop.is_some() && !added.is_local());
    assert_eq!(added.config.pop_keep_days, Some(14));
    // Its server settings edit the POP server, and when the server's copy goes.
    let Dialog::EditAccount(mut e) = Dialog::edit_account(&added.config) else { panic!() };
    assert_eq!(e.params()["popHost"], "pop.example.org");
    (e.pop_keep.for_days, e.pop_keep.until_deleted) = (false, true);
    assert_eq!((e.params()["popKeepDays"].clone(), e.params()["popDeleteWithTrash"].clone()), (json!(0), json!(true)));
    e.pop_keep.keep = false;
    assert_eq!((e.params()["popDelete"].clone(), e.params()["popDeleteWithTrash"].clone()), (json!(true), json!(false)));
}

/// Issue #1: a shared mailbox is a mailbox of its own in the folder pane, after the account's
/// folders, open or collapsed (its unread mail then beside its name); a special folder under the
/// server's own name draws as it is.
#[test]
fn a_shared_mailbox_draws_as_a_mailbox_of_its_own() {
    use mailcraft_engine::model::{Folder, FolderRole};
    let (mut a, _) = app("shared-mailbox");
    let mut folders = a.session.account("sample").unwrap().folders.clone();
    if let Some(inbox) = folders.iter_mut().find(|f| f.role == FolderRole::Inbox) {
        inbox.name = Some("Postvak IN".into());
    }
    let shared = |path: &str, role: FolderRole, unread: u32| Folder {
        account: "sample".into(),
        path: path.into(),
        delimiter: Some('/'),
        selectable: true,
        unread,
        mailbox: Some("test@studio.example".into()),
        mailbox_role: role,
        ..Default::default()
    };
    folders.push(shared("Test Grommunio/Postvak IN", FolderRole::Inbox, 3));
    folders.push(shared("Test Grommunio", FolderRole::Other, 0));
    folders.push(shared("Test Grommunio/Postvak IN/Orders", FolderRole::Other, 0));
    a.session.apply(MailEvent::Folders { account: "sample".into(), folders });
    let paths: Vec<String> = a.session.account("sample").unwrap().folders.iter().map(|f| f.path.clone()).collect();
    assert_eq!(&paths[paths.len() - 3..], ["Test Grommunio", "Test Grommunio/Postvak IN", "Test Grommunio/Postvak IN/Orders"]);
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.run("folder.open", json!({"account": "sample", "folder": "Test Grommunio/Postvak IN"}));
    frame(&ctx, &mut a, vec![]);
    // In Favorites it says whose Inbox it is.
    a.run("folder.favorite", json!({"account": "sample", "folder": "Test Grommunio/Postvak IN", "on": true}));
    a.run("view.collapse", json!({"key": "folder:sample/Test Grommunio"}));
    frame(&ctx, &mut a, vec![]);
    let e = a.session.run("folder.rename", &json!({"account": "sample", "folder": "Test Grommunio/Postvak IN", "name": "X"})).unwrap_err();
    assert!(e.to_string().contains("can't be renamed"), "{e}");
}

/// Issue #3: Server settings show the outgoing server's own sign-in; left alone, its kept
/// password stays (none is sent), and turned off it goes back to the incoming server's.
#[test]
fn server_settings_keep_the_outgoing_servers_own_sign_in() {
    let mut c = mailcraft_engine::model::AccountConfig::guess("zoe@example.org");
    c.id = "a1".into();
    c.smtp_username = Some("relay-user".into());
    let Dialog::EditAccount(mut f) = Dialog::edit_account(&c) else { panic!("the server settings") };
    assert!(f.smtp_sign_in.own && f.smtp_sign_in.kept);
    let p = f.params();
    assert_eq!(p["smtpUsername"], "relay-user");
    assert!(p.get("smtpPassword").is_none(), "the kept password stays");
    f.smtp_sign_in.password = "new".into();
    assert_eq!(f.params()["smtpPassword"], "new");
    f.smtp_sign_in.own = false;
    assert!(f.params()["smtpUsername"].is_null());
}

/// Issue #4: online only, from the account forms and File › Options. Added with it ticked (on
/// from the start when portable), the account's mail is kept in memory; Options lists a switch
/// for each account that can be online only (not the sample mailbox), and Account settings keeps it.
#[test]
fn an_account_is_kept_online_only_from_the_forms() {
    let (mut a, _) = app("online-only");
    let ctx = egui::Context::default();
    a.session.portable = true;
    a.ui.dialog = Some(Dialog::add_account_with("zoe@example.org", false));
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    let mut p = match &mut a.ui.dialog {
        Some(Dialog::AddAccount(f)) => {
            assert_eq!(f.online_only, Some(true), "ticked when portable");
            f.password = "pw".into();
            f.params()
        }
        _ => serde_json::Value::Null,
    };
    assert_eq!(p["onlineOnly"], json!(true));
    p["imapHost"] = json!("mail.example.org");
    a.act(crate::Action::Submit("account.add", p));
    let id = a.session.accounts.iter().find(|x| x.config.email == "zoe@example.org").map(|x| x.config.id.clone()).unwrap();
    assert!(a.session.store.as_ref().unwrap().is_online_only(&id));

    // File › Options and Info draw it; the switch turns it off.
    crate::tests::backstage_tour(&mut a, &ctx);
    a.act(crate::Action::Run("account.onlineOnly", json!({"account": id, "on": false})));
    assert!(!a.session.store.as_ref().unwrap().is_online_only(&id));

    // Account settings: the box is as the account is; ticked and saved, it's online only again.
    let config = a.session.account(&id).unwrap().config.clone();
    let mut d = Dialog::edit_account(&config);
    let p = match &mut d {
        Dialog::EditAccount(f) => {
            assert!(!f.online_only);
            f.online_only = true;
            f.params()
        }
        _ => serde_json::Value::Null,
    };
    a.ui.dialog = Some(d);
    frame(&ctx, &mut a, vec![]);
    a.act(crate::Action::Submit("account.update", p));
    assert!(a.session.store.as_ref().unwrap().is_online_only(&id));
    // Not portable: a new account isn't, unless asked.
    a.session.portable = false;
    a.ui.dialog = Some(Dialog::add_account_with("ana@example.org", false));
    frame(&ctx, &mut a, vec![]);
    assert!(matches!(&a.ui.dialog, Some(Dialog::AddAccount(f)) if f.online_only == Some(false) && f.params().get("onlineOnly").is_none()));
}

/// A dozen accounts waiting for their password: the status bar shows as many as fit and "+N"
/// for the rest, never over "Items" and "Unread" on its left (they ran over them).
#[test]
fn many_accounts_waiting_dont_cover_the_counts() {
    let (mut a, _) = app("status-many");
    let ctx = egui::Context::default();
    for n in 0..12 {
        let c = mailcraft_engine::model::AccountConfig::guess(&format!("waiting{n}@studio.example"));
        a.session.add_account(c, None).unwrap();
    }
    let ids: Vec<String> = a.session.accounts.iter().filter(|x| x.config.email.starts_with("waiting")).map(|x| x.config.id.clone()).collect();
    assert_eq!(ids.len(), 12);
    let mut texts = Vec::new();
    for _ in 0..3 {
        for id in &ids {
            a.session.apply(MailEvent::Status { account: id.clone(), status: SyncStatus::NeedsPassword { error: String::new() } });
        }
        let drawn = drawn_text(&ctx, &mut a, egui::vec2(1000.0, 700.0));
        texts = drawn.into_iter().filter(|d| d.rect.min.y > 675.0).map(|d| (d.text, d.rect.min.x, d.rect.max.x)).collect();
    }
    let counts_end = texts.iter().filter(|(s, ..)| s.starts_with("Items") || s.starts_with("Unread")).map(|(_, _, r)| *r).fold(0.0, f32::max);
    let waiting: Vec<_> = texts.iter().filter(|(s, ..)| s.contains("waiting") && s.contains(':') || s.starts_with('+')).collect();
    assert!(counts_end > 0.0, "{texts:?}");
    assert!(waiting.iter().any(|(s, ..)| s.starts_with('+')), "the rest as +N: {texts:?}");
    assert!(waiting.iter().all(|(_, l, _)| *l > counts_end), "nothing over the counts: {texts:?}");
}

/// An account's trouble that needs you is told apart from one that only waits to be tried again:
/// the status bar says which (and no other), links to what helps, and Sign in again opens it.
#[test]
fn an_account_s_trouble_is_said_and_offers_what_helps() {
    use mailcraft_engine::model::{FailureKind, MICROSOFT_IMAP_CLOSED, MICROSOFT_NO_PASSWORDS, local_certificate_refusal};
    let (mut a, _) = app("troubles");
    let ctx = egui::Context::default();
    let size = egui::vec2(1280.0, 800.0);
    a.act(crate::Action::Submit("account.add", json!({"email": "zoe@example.org", "password": "pw", "imapHost": "mail.example.org"})));
    let id = a.session.accounts.iter().find(|x| x.config.email == "zoe@example.org").map(|x| x.config.id.clone()).unwrap();
    let print = "ab".repeat(32);
    let closed = format!("{MICROSOFT_IMAP_CLOSED}. Turn on IMAP on Outlook.com.");
    let words = ["Sign in with Microsoft", "Mailbox won't open", "Certificate not trusted yet"];
    let no_passwords = format!("{MICROSOFT_NO_PASSWORDS} (Basic authentication is disabled)");
    let certificate = format!("secure connection: {}", local_certificate_refusal("127.0.0.1", &print));
    let local = FailureKind::LocalCertificate { host: "127.0.0.1".into(), fingerprint: print.clone() };
    let cases = [
        (SyncStatus::Error { error: no_passwords.clone(), kind: FailureKind::MicrosoftNoPasswords }, Some(words[0])),
        (SyncStatus::Error { error: closed.clone(), kind: FailureKind::MicrosoftImapClosed }, Some(words[1])),
        (SyncStatus::Error { error: certificate.clone(), kind: local }, Some(words[2])),
        (SyncStatus::error("the server closed the connection: bye"), None),
        // Told by its kind, not its words: these only wait to be tried again.
        (SyncStatus::error(no_passwords), None),
        (SyncStatus::error(closed.clone()), None),
        (SyncStatus::error(certificate), None),
    ];
    for (status, said) in cases {
        a.session.apply(MailEvent::Status { account: id.clone(), status });
        a.ui.dialog = None;
        drawn_text(&ctx, &mut a, size);
        let texts = drawn_strings(&ctx, &mut a, size);
        let link = said.map(|w| format!("zoe@example.org: {w}"));
        // Waiting for you: a link saying what. Otherwise counted among the disconnected.
        assert_eq!(texts.iter().any(|t| Some(t) == link.as_ref()), said.is_some(), "{said:?}: {texts:?}");
        assert_eq!(texts.iter().any(|t| t == "1 account disconnected"), said.is_none(), "{said:?}: {texts:?}");
        assert!(words.iter().filter(|w| Some(**w) != said).all(|w| !texts.iter().any(|t| t.ends_with(w))), "{said:?}: {texts:?}");
        a.act(crate::Action::SignInAgain(id.clone()));
        match (said, &a.ui.dialog) {
            (Some("Mailbox won't open"), Some(Dialog::MailboxClosed { error, .. })) => assert_eq!(error, &closed),
            (Some("Certificate not trusted yet"), Some(Dialog::TrustCertificate { host, fingerprint, .. })) => {
                assert_eq!((host.as_str(), fingerprint.as_str()), ("127.0.0.1", print.as_str()));
            }
            (Some("Sign in with Microsoft") | None, Some(Dialog::Password { .. })) => {}
            (said, d) => panic!("{said:?}: {:?}", d.as_ref().map(std::mem::discriminant)),
        }
    }
}

/// Issue #58: File › Options gives each account with a server a time of its own to look for mail.
#[test]
fn options_gives_each_account_its_time_to_look_for_mail() {
    let (mut a, _) = app("check-every");
    let ctx = egui::Context::default();
    a.act(crate::Action::Submit("account.add", json!({"email": "zoe@example.org", "password": "pw", "imapHost": "mail.example.org"})));
    let id = a.session.accounts.iter().find(|x| x.config.email == "zoe@example.org").map(|x| x.config.id.clone()).unwrap();
    a.act(crate::Action::Backstage(crate::backstage::Page::Options));
    let tall = egui::vec2(1280.0, 3000.0);
    drawn_text(&ctx, &mut a, tall);
    let texts = drawn_strings(&ctx, &mut a, tall);
    assert!(has(&texts, "Checking for new mail") && has(&texts, "zoe@example.org") && has(&texts, "Automatically"), "{texts:?}");
    a.act(crate::Action::Run("account.checkEvery", json!({"account": id, "minutes": 30})));
    drawn_text(&ctx, &mut a, tall);
    let texts = drawn_strings(&ctx, &mut a, tall);
    assert!(has(&texts, "Every 30 minutes"), "{texts:?}");
}

/// File › Info for a mailbox kept on this computer that isn't the sample: its automatic replies
/// aren't available, and it isn't called the sample mailbox.
#[test]
fn a_local_mailbox_isnt_called_the_sample() {
    let (mut a, _) = app("local-no-server");
    let ctx = egui::Context::default();
    a.act(crate::Action::Submit("account.add", json!({"email": "me@home.example", "local": true})));
    let id = a.session.accounts.iter().find(|x| x.config.email == "me@home.example").map(|x| x.config.id.clone()).unwrap();
    a.act(crate::Action::Backstage(crate::backstage::Page::Info));
    if let Some(page) = a.ui.backstage.as_mut() {
        page.account = Some(id);
    }
    let tall = egui::vec2(1280.0, 3000.0);
    drawn_text(&ctx, &mut a, tall);
    let texts = drawn_strings(&ctx, &mut a, tall);
    assert!(texts.iter().any(|t| t == "Not available: this mailbox has no server."), "{texts:?}");
    assert!(!texts.iter().any(|t| t.contains("sample mailbox")), "{texts:?}");
}

/// What the status bar costs a frame with 100,000 messages in the list, then with a dozen
/// accounts more; and a fingerprint of what it draws (the same before and after a change that
/// only makes it faster). Run in release under the machine's lock:
/// `cargo test --release -p mailcraft-ui-egui --lib measure_status_bar -- --ignored --nocapture`.
#[test]
#[ignore = "a measurement: run in release"]
fn measure_status_bar() {
    use std::hash::{Hash, Hasher};
    let (mut a, _) = app("measure-status-bar");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let first = a.session.list.first().map(|m| (**m).clone()).unwrap();
    a.session.list = (0..100_000u32)
        .map(|n| {
            let mut m = first.clone();
            m.key.uid = n + 1;
            m.flags.seen = n % 3 != 0;
            std::sync::Arc::new(m)
        })
        .collect();
    a.session.revision = a.session.revision.wrapping_add(1);
    let bar = |a: &mut MailApp| {
        let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 26.0))), ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| a.status_bar(ui));
        out.textures_delta.clear();
        texts_in(&out.shapes)
    };
    for case in ["100,000 messages listed", "and 12 accounts more"] {
        if case == "and 12 accounts more" {
            for n in 0..12 {
                let c = mailcraft_engine::model::AccountConfig::guess(&format!("more{n}@studio.example"));
                let id = a.session.add_account(c, None).unwrap();
                a.session.apply(MailEvent::Status { account: id, status: SyncStatus::Online { at: NOW - 600 } });
            }
        }
        let drawn = bar(&mut a);
        let mut h = std::collections::hash_map::DefaultHasher::new();
        for d in &drawn {
            (&d.text, d.rect.min.x.to_bits(), d.rect.min.y.to_bits()).hash(&mut h);
        }
        let t = std::time::Instant::now();
        for _ in 0..200 {
            let _ = bar(&mut a);
        }
        let each = t.elapsed().as_secs_f64() * 1000.0 / 200.0;
        println!("MEASURE status bar, {case}: {each:.3} ms a frame; drawn {} texts, {:016x}", drawn.len(), h.finish());
    }
}

/// Saved settings with one a later (or earlier) version wrote otherwise: that one starts as it does
/// for everyone, the others are kept; and settings that aren't settings at all change nothing.
#[test]
fn saved_settings_keep_what_can_be_read() {
    let (mut a, _) = app("kept-settings");
    let mut saved = a.saved_state();
    saved["prefs"]["conversations"] = json!(true);
    saved["prefs"]["keepMessages"] = json!(250);
    saved["prefs"]["readingPane"] = json!(42);
    saved["prefs"]["compact"] = json!("very");
    a.restore_state(&saved);
    let p = &a.session.prefs;
    assert!(p.conversations, "kept");
    assert_eq!(p.keep_messages, 250, "kept");
    let d = mailcraft_engine::Prefs::default();
    assert_eq!((p.reading_pane, p.compact), (d.reading_pane, d.compact), "as they start");
    let before = a.session.prefs.clone();
    a.restore_state(&json!({"prefs": "not settings"}));
    assert_eq!(a.session.prefs, before);
}
