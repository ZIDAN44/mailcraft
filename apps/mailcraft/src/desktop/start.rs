//! What MailCraft starts with: its session, made on the `data-open` thread once the data files
//! are open (`boot.rs`), and the `--cmd` commands run at start-up.

use mailcraft_ui_egui::backstage::{Backstage, Page};

use super::*;

/// What a session is made with, besides its store.
#[derive(Clone)]
pub(crate) struct Made {
    pub(crate) clock: Clock,
    /// MailCraft is portable (`portable.rs`): passwords and sign-ins are kept in memory only.
    pub(crate) portable: bool,
    /// A release build's commit, so File › About can tell its test build from a later one.
    pub(crate) build_sha: Option<String>,
}

/// The session over `store`, as MailCraft opens with it (made on the `data-open` thread).
pub(crate) fn session(store: Option<Store>, Made { clock, portable, build_sha }: Made) -> Session {
    let mut session = Session::open(store, clock);
    session.oauth_available = mailcraft_net::oauth::available();
    // S/MIME signers are trusted through the roots this computer trusts.
    session.set_trust_roots(mailcraft_net::trust_roots());
    session.set_card_modules(mailcraft_net::card_modules());
    session.portable = portable;
    session.build_sha = build_sha;
    // A build of a release checks for releases only (File › About too).
    session.updates_stable_only = crate::self_update::stable();
    session
}

/// Run the `--cmd` commands MailCraft was started with (`ui.*` ones open that page or dialog).
pub(crate) fn commands(app: &mut MailApp, args: &[String]) {
    for c in args.windows(2).filter(|w| w.first().is_some_and(|a| a == "--cmd")).filter_map(|w| w.get(1)) {
        let (id, params) = c.split_once('=').unwrap_or((c.as_str(), "{}"));
        match id {
            "ui.addAccount" => {
                let p: serde_json::Value = serde_json::from_str(params).unwrap_or_default();
                let mut d = mailcraft_ui_egui::Dialog::add_account_with(
                    p.get("email").and_then(|v| v.as_str()).unwrap_or_default(),
                    p.get("advanced").and_then(|v| v.as_bool()).unwrap_or(false),
                );
                if let mailcraft_ui_egui::Dialog::AddAccount(f) = &mut d {
                    let flag = |name: &str| p.get(name).and_then(|v| v.as_bool()).unwrap_or(false);
                    if flag("graph") {
                        f.kind = mailcraft_ui_egui::AccountKind::Graph;
                    } else if flag("pop") {
                        f.kind = mailcraft_ui_egui::AccountKind::Pop;
                    }
                    f.calendar_only = flag("calendarOnly");
                }
                app.ui.dialog = Some(d);
            }
            "ui.file" | "ui.settings" => app.ui.backstage = Some(Backstage::new(Page::Info)),
            "ui.options" => app.ui.backstage = Some(Backstage::new(Page::Options)),
            "ui.openExport" => app.ui.backstage = Some(Backstage::new(Page::Open)),
            "ui.about" => app.ui.backstage = Some(Backstage::new(Page::About)),
            "ui.newEvent" => app.ui.dialog = Some(mailcraft_ui_egui::Dialog::new_event(&app.session, None)),
            "ui.newContact" => app.ui.dialog = Some(mailcraft_ui_egui::Dialog::new_contact()),
            "ui.newTask" => app.ui.dialog = Some(mailcraft_ui_egui::Dialog::new_task(&app.session, None)),
            // A task's form, by its uid (`{"uid": …}`).
            "ui.openTask" => {
                let p: serde_json::Value = serde_json::from_str(params).unwrap_or_default();
                app.ui.dialog = mailcraft_ui_egui::Dialog::open_task(&app.session, p.get("uid").and_then(|v| v.as_str()).unwrap_or_default());
            }
            "ui.signature" => app.ui.dialog = mailcraft_ui_egui::Dialog::signature(&app.session),
            "ui.categories" => app.ui.dialog = Some(mailcraft_ui_egui::Dialog::Categories),
            "ui.nameKeyword" => {
                let p: serde_json::Value = serde_json::from_str(params).unwrap_or_default();
                let k = p.get("keyword").and_then(|v| v.as_str()).unwrap_or_default();
                app.ui.dialog = Some(mailcraft_ui_egui::Dialog::name_keyword(k));
            }
            _ => match serde_json::from_str(params) {
                Ok(p) => app.run(id, p),
                Err(e) => say(&format!("--cmd {id}: bad JSON: {e}")),
            },
        }
    }
}
