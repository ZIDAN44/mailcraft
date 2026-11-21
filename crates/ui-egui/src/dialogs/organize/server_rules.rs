//! The Rules dialog: an Exchange mailbox's own rules on its server (issue #139), under MailCraft's
//! own, each account's in a section of its own: turned on or off, moved, edited when MailCraft
//! can say what they do, deleted, and new ones made there.

use mailcraft_engine::ServerRulesState;
use mailcraft_engine::model::organize::ServerRule;

use super::*;

/// Where a rule being edited goes: an account's server (and which of its rules, `None` for a new
/// one), and whether Outlook's own copy of the rules may be removed for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerTarget {
    pub account: String,
    pub id: Option<String>,
    pub drop_outlook_list: bool,
}

/// The id remembering that the user let an account's rules be changed beside Outlook's own copy.
fn allowed_id(account: &str) -> egui::Id {
    egui::Id::unique(("server-rules-allowed", account))
}

/// Each Exchange account's rules on its server, after MailCraft's own.
pub(super) fn sections(ui: &mut Ui, s: &Session, t: &Tokens, acts: &mut Vec<Action>) {
    for account in s.server_rule_accounts() {
        let email = s.account(&account).map(|a| a.config.email.clone()).unwrap_or_default();
        ui.add_space(12.0);
        ui.label(RichText::new(tf!("On the server: {account}", account = email)).strong());
        ui.label(RichText::new(tl!("The server runs these on new mail, even while MailCraft is closed.")).color(t.dim).small());
        ui.add_space(4.0);
        match s.server_rules(&account) {
            ServerRulesState::Unknown => acts.push(Action::Run("rules.server", json!({"account": account}))),
            ServerRulesState::Loading => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(RichText::new(tl!("Reading the rules on the server…")).color(t.dim));
                });
            }
            ServerRulesState::Failed(e) => {
                ui.label(RichText::new(e.as_str()).color(t.faint).small());
                if ui.button(tl!("Try again")).clicked() {
                    acts.push(Action::Run("rules.server", json!({"account": account})));
                }
            }
            ServerRulesState::Loaded { rules, error } => {
                if let Some(e) = error {
                    ui.label(RichText::new(e.as_str()).color(t.danger).small());
                }
                let mut allowed: bool = ui.ctx().data(|d| d.get_temp(allowed_id(&account))).unwrap_or(false);
                if rules.outlook_list {
                    ui.label(
                        RichText::new(tl!(
                            "Outlook keeps its own copy of these rules. Changing them here removes it, with Outlook's rules that are turned off or run only on its computer."
                        ))
                        .color(t.warn)
                        .small(),
                    );
                    if ui.checkbox(&mut allowed, tl!("Change them here anyway")).changed() {
                        ui.ctx().data_mut(|d| d.insert_temp(allowed_id(&account), allowed));
                    }
                }
                let may = !rules.outlook_list || allowed;
                let drop = rules.outlook_list && allowed;
                if rules.rules.is_empty() {
                    ui.label(RichText::new(tl!("No rules yet.")).color(t.faint));
                }
                let last = rules.rules.len().saturating_sub(1);
                egui::Frame::new().stroke(egui::Stroke::new(1.0, t.border)).corner_radius(6).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for (i, r) in rules.rules.iter().enumerate() {
                        rule_row(ui, s, t, &account, r, (i == 0, i == last), may, drop, acts);
                    }
                });
                ui.add_space(4.0);
                if ui.add_enabled(may, egui::Button::new(tl!("New rule on the server…"))).clicked() {
                    let target = ServerTarget { account: account.clone(), id: None, drop_outlook_list: drop };
                    acts.push(Action::Dialog(Dialog::EditRule(Box::new(RuleForm::for_server(target, None)))));
                }
            }
        }
    }
}

/// One rule on the server: its switch, name, what it does, and its buttons.
#[allow(clippy::too_many_arguments)]
fn rule_row(
    ui: &mut Ui,
    s: &Session,
    t: &Tokens,
    account: &str,
    r: &ServerRule,
    (first, last): (bool, bool),
    may: bool,
    drop: bool,
    acts: &mut Vec<Action>,
) {
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::hover());
    let mut inner = ui.new_child(
        egui::UiBuilder::new()
            .id_salt(("server-rule", account, &r.id))
            .max_rect(row.shrink2(vec2(8.0, 0.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    let ui = &mut inner;
    let run = |id: &'static str, extra: serde_json::Value| {
        let mut p = json!({"account": account, "id": r.id, "dropOutlookList": drop});
        if let (Some(p), Some(extra)) = (p.as_object_mut(), extra.as_object()) {
            p.extend(extra.clone());
        }
        Action::Run(id, p)
    };
    let mut on = r.enabled;
    if ui.add_enabled(may, egui::Checkbox::new(&mut on, "")).on_hover_text(tl!("On / off")).changed() {
        acts.push(run("rules.serverSwitch", json!({"on": on})));
    }
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        if !r.not_supported && ui.add_enabled_ui(may, |ui| widgets::tiny_icon_button(ui, Icon::Delete, "Delete", t)).inner.clicked() {
            acts.push(run("rules.serverDelete", json!({})));
        }
        if r.rule.is_some() && !r.not_supported && ui.add_enabled_ui(may, |ui| widgets::tiny_icon_button(ui, Icon::Drafts, "Edit", t)).inner.clicked()
        {
            let target = ServerTarget { account: account.to_string(), id: Some(r.id.clone()), drop_outlook_list: drop };
            acts.push(Action::Dialog(Dialog::EditRule(Box::new(RuleForm::for_server(target, Some(r))))));
        }
        if ui.add_enabled(may && !last, egui::Button::new("↓").small()).on_hover_text(tl!("Move down")).clicked() {
            acts.push(run("rules.serverMove", json!({"up": false})));
        }
        if ui.add_enabled(may && !first, egui::Button::new("↑").small()).on_hover_text(tl!("Move up")).clicked() {
            acts.push(run("rules.serverMove", json!({"up": true})));
        }
        ui.add_space(6.0);
        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
            if r.in_error {
                let (ir, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                crate::icons::paint(ui.painter(), ir, Icon::Warning, t.warn);
            }
            let what = match &r.rule {
                Some(rule) => describe(s, rule),
                None => tl!("Made in another program: it can be turned on or off, moved or deleted here").to_string(),
            };
            let what = if r.in_error { format!("{} · {what}", tl!("The server can't run this rule as it is")) } else { what };
            let mut job = egui::text::LayoutJob::default();
            let strong = egui::TextFormat { font_id: theme::bold(13.0), color: if r.enabled { t.text } else { t.faint }, ..Default::default() };
            job.append(&r.name, 0.0, strong);
            job.append(&what, 10.0, egui::TextFormat { font_id: theme::regular(12.0), color: t.dim, ..Default::default() });
            // Cut short by the label itself (issue #214): a job's own wrapping gives way to the row's,
            // which runs on, and the note went under the row's buttons and past the dialog's edge.
            ui.add(egui::Label::new(job).selectable(false).truncate()).on_hover_text(format!("{}\n{what}", r.name));
        });
    });
}
