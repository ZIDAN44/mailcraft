//! UI tests: a JMAP server behind a single sign-on (OpenID Connect), asked how it's signed in to
//! as its account is added, and an account moved to it and back in its server settings.

use mailcraft_engine::model::{JmapProbe, Job, NO_CLIENT_ID, NO_PASSWORDS, OIDC, OidcSettings};

use super::*;

const SERVER: &str = "https://jmap.example.com/jmap/session";
const ISSUER: &str = "https://sso.example.com";

/// The window in a copy that signs in through single sign-ons, the jobs it starts kept.
fn with_sign_on(name: &str) -> (MailApp, Jobs, std::path::PathBuf) {
    let (mut a, dir) = app(name);
    let jobs = Jobs::default();
    a.backend = Box::new(jobs.clone());
    a.session.oauth_available = vec![OIDC.into()];
    (a, jobs, dir)
}

/// What the server at [`SERVER`] said: bearer tokens only, from [`ISSUER`], and the client id
/// MailCraft was given for it, if any.
fn bearer_only(client_id: Option<&str>) -> MailEvent {
    let probe = JmapProbe {
        bearer: true,
        issuer: Some(ISSUER.into()),
        scope: "openid profile email".into(),
        client_id: client_id.map(str::to_string),
        ..JmapProbe::default()
    };
    MailEvent::JmapProbed { url: SERVER.into(), probe }
}

/// A few frames: a dialog just opened is laid out, then drawn.
fn settle(ctx: &egui::Context, a: &mut MailApp) {
    for _ in 0..3 {
        frame(ctx, a, vec![]);
    }
}

/// The servers asked how they're signed in to, among the jobs started since last looked at.
fn asked(jobs: &Jobs) -> Vec<String> {
    jobs.take()
        .into_iter()
        .filter_map(|j| match j {
            Job::ProbeJmap { url } => Some(url),
            _ => None,
        })
        .collect()
}

/// The controls of a settled frame as a screen reader is told of them: name, middle, and
/// whether on.
fn controls(ctx: &egui::Context, a: &mut MailApp) -> Vec<(String, egui::Pos2, bool)> {
    settle(ctx, a);
    let size = windows::layout::SIZE.with(std::cell::Cell::get);
    let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)), ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| a.ui(ui));
    out.textures_delta.clear();
    let Some(tree) = out.platform_output.accesskit_update else { return Vec::new() };
    tree.nodes
        .iter()
        .filter_map(|(_, n)| {
            let name = n.label().or(n.value())?.to_string();
            let b = n.bounds()?;
            Some((name, egui::pos2(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32), !n.is_disabled()))
        })
        .collect()
}

/// Whether the control named `name` is drawn, and on.
fn shown(ctx: &egui::Context, a: &mut MailApp, name: &str) -> Option<bool> {
    controls(ctx, a).into_iter().find(|(n, _, _)| n == name).map(|(_, _, on)| on)
}

/// Clicks the control named `name`, which must be drawn and on.
fn click(ctx: &egui::Context, a: &mut MailApp, name: &str) {
    let found = controls(ctx, a).into_iter().find(|(n, _, _)| n == name);
    let Some((_, at, true)) = found else { panic!("{name}: {found:?}") };
    let press = |pressed| Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    frame(ctx, a, vec![Event::PointerMoved(at), press(true)]);
    frame(ctx, a, vec![press(false)]);
}

/// The add dialog for `ana@example.com` at the JMAP server's address.
fn add_dialog() -> Dialog {
    let mut d = Dialog::add_account_with("ana@example.com", true);
    if let Dialog::AddAccount(f) = &mut d {
        f.jmap = true;
        f.jmap_url = format!(" {SERVER} ");
    }
    d
}

fn added(a: &MailApp, email: &str) -> mailcraft_engine::model::AccountConfig {
    a.session.accounts.iter().find(|x| x.config.email == email).map(|x| x.config.clone()).unwrap()
}

#[test]
fn the_add_dialog_signs_in_through_the_single_sign_on_its_server_names() {
    let (mut a, jobs, dir) = with_sign_on("sso-add");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    a.ui.dialog = Some(add_dialog());
    // Its address typed: the server is asked how it's signed in to, once.
    settle(&ctx, &mut a);
    assert_eq!(asked(&jobs), vec![SERVER.to_string()]);
    settle(&ctx, &mut a);
    assert!(asked(&jobs).is_empty(), "asked once");
    assert_eq!(a.session.jmap_probes.get(SERVER), Some(&None), "waiting for it");
    // It takes no passwords, and MailCraft has no client ID for its service: one is typed.
    a.session.apply(bearer_only(None));
    assert_eq!(shown(&ctx, &mut a, "Connect"), Some(false), "no password to connect with");
    assert_eq!(shown(&ctx, &mut a, "Sign in through the browser"), Some(false), "no client ID yet");
    if let Some(Dialog::AddAccount(f)) = a.ui.dialog.as_mut() {
        f.client_id = " mc-desktop ".into();
        f.password = "typed before it answered".into();
    }
    assert_eq!(shown(&ctx, &mut a, "Connect"), Some(false), "a password typed isn't sent to it");
    jobs.take();
    click(&ctx, &mut a, "Sign in through the browser");
    assert!(a.ui.dialog.is_none(), "added");
    let c = added(&a, "ana@example.com");
    let want = OidcSettings { issuer: ISSUER.into(), client_id: "mc-desktop".into(), scope: "openid profile email".into() };
    assert_eq!((c.oauth.as_deref(), c.oidc.as_ref(), c.jmap.as_deref()), (Some(OIDC), Some(&want), Some(SERVER)));
    let started = jobs.take();
    assert!(started.iter().any(|j| matches!(j, Job::SignIn { account } if account.id == c.id)), "{started:?}");
    assert!(!started.iter().any(|j| matches!(j, Job::Start { .. })), "no password sent: {started:?}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_add_dialog_takes_the_client_id_given_and_still_a_password_where_one_is() {
    let (mut a, jobs, dir) = with_sign_on("sso-add-given");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    // The server takes either; MailCraft was given a client ID for its service.
    a.ui.dialog = Some(add_dialog());
    settle(&ctx, &mut a);
    if let MailEvent::JmapProbed { url, mut probe } = bearer_only(Some("mailcraft")) {
        probe.password = true;
        a.session.apply(MailEvent::JmapProbed { url, probe });
    }
    if let Some(Dialog::AddAccount(f)) = a.ui.dialog.as_mut() {
        f.password = "pw".into();
    }
    assert_eq!(shown(&ctx, &mut a, "Connect"), Some(true), "a password still connects");
    click(&ctx, &mut a, "Sign in through the browser");
    assert_eq!(added(&a, "ana@example.com").oidc.map(|o| o.client_id), Some("mailcraft".into()));
    assert!(jobs.take().iter().any(|j| matches!(j, Job::SignIn { .. })));

    // Where single sign-ons aren't signed in through (the browser app), the server isn't asked.
    let (mut a, jobs, dir2) = with_sign_on("sso-add-none");
    a.session.oauth_available.clear();
    a.ui.dialog = Some(add_dialog());
    settle(&ctx, &mut a);
    assert!(asked(&jobs).is_empty() && a.session.jmap_probes.is_empty());
    // Nor about an address the command wouldn't take.
    a.session.oauth_available = vec![OIDC.into()];
    if let Some(Dialog::AddAccount(f)) = a.ui.dialog.as_mut() {
        f.jmap_url = "https://jmap example.com".into();
    }
    settle(&ctx, &mut a);
    assert!(asked(&jobs).is_empty());
    for d in [dir, dir2] {
        let _ = std::fs::remove_dir_all(d);
    }
}

#[test]
fn the_server_settings_move_an_account_to_its_single_sign_on_and_back() {
    let (mut a, jobs, dir) = with_sign_on("sso-settings");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let id = a.session.run("account.add", &json!({"email": "ana@example.com", "password": "pw", "jmap": SERVER})).unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    a.session.take_jobs();
    // Its server takes no passwords: the status bar says so, and signing in again opens its
    // server settings at signing in through the browser.
    a.session.apply(MailEvent::Status { account: id.clone(), status: SyncStatus::Error { error: NO_PASSWORDS.into() } });
    let state = a.session.account(&id).cloned().unwrap();
    assert!(crate::app::bars::needs_you(&state));
    let t = crate::theme::Tokens::light();
    assert_eq!(crate::app::bars::account_status(&a.session, &t, &state).1, "Sign in through the browser");
    a.sign_in_again(&id);
    assert!(matches!(&a.ui.dialog, Some(Dialog::EditAccount(f)) if f.sign_on.browser && !f.sign_on.was_browser));
    settle(&ctx, &mut a);
    assert_eq!(asked(&jobs), vec![SERVER.to_string()], "its server asked");
    assert_eq!(shown(&ctx, &mut a, "Save"), Some(false), "not before it answers");
    a.session.apply(bearer_only(Some("mailcraft")));
    jobs.take();
    click(&ctx, &mut a, "Save");
    assert!(a.ui.dialog.is_none());
    let c = a.session.account(&id).map(|x| x.config.clone()).unwrap();
    assert_eq!((c.oauth.as_deref(), c.oidc.as_ref().map(|o| (o.issuer.as_str(), o.client_id.as_str()))), (Some(OIDC), Some((ISSUER, "mailcraft"))));
    let started = jobs.take();
    assert!(started.iter().any(|j| matches!(j, Job::SignIn { .. })), "signed in through the browser: {started:?}");
    assert!(!started.iter().any(|j| matches!(j, Job::Start { .. })), "the password kept never sent: {started:?}");

    // Back to a password: Save waits for it.
    a.ui.dialog = Some(Dialog::edit_account(&c));
    if let Some(Dialog::EditAccount(f)) = a.ui.dialog.as_mut() {
        assert!(f.sign_on.was_browser && f.sign_on.client_id == "mailcraft");
        f.sign_on.browser = false;
    }
    assert_eq!(shown(&ctx, &mut a, "Save"), Some(false), "no password yet");
    if let Some(Dialog::EditAccount(f)) = a.ui.dialog.as_mut() {
        f.sign_on.password = "pw2".into();
    }
    jobs.take();
    click(&ctx, &mut a, "Save");
    assert_eq!(a.session.account(&id).and_then(|x| x.config.oauth.clone()), None);
    assert!(jobs.take().iter().any(|j| matches!(j, Job::Start { password: Some(p), .. } if p.0 == "pw2")));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_single_sign_on_without_a_client_id_is_put_right_in_the_server_settings() {
    let (mut a, jobs, dir) = with_sign_on("sso-client-id");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    // Added where MailCraft has no client ID for the server's service.
    a.session.jmap_probes.insert(SERVER.into(), None);
    a.session.apply(bearer_only(None));
    let id = a.session.run("account.add", &json!({"email": "ana@example.com", "jmap": SERVER, "oauth": "oidc"})).unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    a.session.take_jobs();
    a.session.apply(MailEvent::Status { account: id.clone(), status: SyncStatus::Error { error: NO_CLIENT_ID.into() } });
    let state = a.session.account(&id).cloned().unwrap();
    let t = crate::theme::Tokens::light();
    assert!(crate::app::bars::needs_you(&state));
    assert_eq!(crate::app::bars::account_status(&a.session, &t, &state).1, "Client ID needed");
    a.sign_in_again(&id);
    assert!(matches!(&a.ui.dialog, Some(Dialog::EditAccount(f)) if f.sign_on.browser && f.sign_on.was_browser));
    if let Some(Dialog::EditAccount(f)) = a.ui.dialog.as_mut() {
        f.sign_on.client_id = "mc-desktop".into();
    }
    jobs.take();
    click(&ctx, &mut a, "Save");
    let c = a.session.account(&id).map(|x| x.config.clone()).unwrap();
    assert_eq!(c.oidc.map(|o| o.client_id), Some("mc-desktop".into()));
    assert!(jobs.take().iter().any(|j| matches!(j, Job::SignIn { .. })), "signed in under it");
    let _ = std::fs::remove_dir_all(dir);
}

/// Every state of the add dialog and the server settings, light and dark: drawn without fault.
#[test]
fn every_single_sign_on_state_is_drawn() {
    let (mut a, _jobs, dir) = with_sign_on("sso-states");
    let ctx = egui::Context::default();
    let id = a.session.run("account.add", &json!({"email": "bo@example.com", "password": "pw", "jmap": SERVER})).unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    let config = a.session.account(&id).map(|x| x.config.clone()).unwrap();
    let not_found = JmapProbe { bearer: true, error: Some("HTTP 404".into()), ..JmapProbe::default() };
    for dark in [false, true] {
        a.ui.dark = Some(dark);
        for answer in [
            None,
            Some(bearer_only(None)),
            Some(bearer_only(Some("mailcraft"))),
            Some(MailEvent::JmapProbed { url: SERVER.into(), probe: not_found.clone() }),
        ] {
            a.session.jmap_probes.clear();
            a.session.jmap_probes.insert(SERVER.into(), None);
            if let Some(e) = answer {
                a.session.apply(e);
            }
            for d in [add_dialog(), Dialog::edit_account(&config)] {
                a.ui.dialog = Some(d);
                if let Some(Dialog::EditAccount(f)) = a.ui.dialog.as_mut() {
                    f.sign_on.browser = true;
                }
                settle(&ctx, &mut a);
            }
        }
    }
    let _ = std::fs::remove_dir_all(dir);
}
