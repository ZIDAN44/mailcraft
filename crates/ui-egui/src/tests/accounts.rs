//! UI tests: the account forms, the File page and its options.

use super::*;

#[test]
fn the_account_form_adds_exchange_accounts() {
    let (mut a, dir) = app("graph-form");
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
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_file_page_shows_accounts_replies_space_and_options() {
    let (mut a, dir) = app("backstage");
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
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_account_form_adds_jmap_accounts() {
    let (mut a, dir) = app("jmap-form");
    let p = match jmap_form() {
        Dialog::AddAccount(f) => f.params(),
        _ => serde_json::Value::Null,
    };
    assert_eq!(p["jmap"], "https://api.fastmail.example/jmap/session");
    assert!(p.get("imapHost").is_none(), "no IMAP servers for a JMAP account");
    a.act(crate::Action::Submit("account.add", p));
    let added = a.session.accounts.iter().find(|x| x.config.email == "zoe@fastmail.example").unwrap();
    assert!(added.config.jmap.is_some() && !added.is_local());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_account_form_adds_pop_accounts_with_their_old_data_file() {
    let (mut a, dir) = app("pop-form");
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
    let _ = std::fs::remove_dir_all(dir);
}

/// Issue #1: a shared mailbox is a mailbox of its own in the folder pane, after the account's
/// folders, open or collapsed (its unread mail then beside its name); a special folder under the
/// server's own name draws as it is.
#[test]
fn a_shared_mailbox_draws_as_a_mailbox_of_its_own() {
    use mailcraft_engine::model::{Folder, FolderRole};
    let (mut a, dir) = app("shared-mailbox");
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
    let _ = std::fs::remove_dir_all(dir);
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
    let (mut a, dir) = app("online-only");
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
    let _ = std::fs::remove_dir_all(dir);
}
