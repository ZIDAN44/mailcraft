//! Dialogs: rules, Search Folders and Quick Steps.

use super::*;

mod quick_steps;
mod search_folders;
mod server_rules;
pub use quick_steps::{QuickStepForm, StepMove};
pub use search_folders::{PresetKind, SEARCH_PRESETS, SearchFolderForm};
pub use server_rules::ServerTarget;

pub(super) fn draw(ui: &mut Ui, d: &mut Dialog, s: &Session, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    match d {
        Dialog::Rules => {
            ui.set_width(560.0);
            title(ui, t, "Rules");
            ui.label(
                RichText::new(tl!("Rules run in this order, on new mail as it arrives in an Inbox, and on mail you send as its copy reaches Sent."))
                    .color(t.dim)
                    .small(),
            );
            ui.add_space(6.0);
            // Many rules: a box to find one by its name or what it does.
            let find_id = egui::Id::unique("rules-find");
            let mut find: String = ui.ctx().data(|d| d.get_temp(find_id)).unwrap_or_default();
            if s.rules.len() > 6 {
                ui.add(egui::TextEdit::singleline(&mut find).hint_text(tl!("Find a rule")).desired_width(f32::INFINITY));
                ui.ctx().data_mut(|d| d.insert_temp(find_id, find.clone()));
                ui.add_space(4.0);
            }
            let needle = find.trim().to_lowercase();
            if s.rules.is_empty() {
                ui.label(RichText::new(tl!("No rules yet.")).color(t.faint));
            }
            let lines = rule_lines(ui.ctx(), s);
            // The rules the find box keeps, in order.
            let rows: Vec<usize> = (0..s.rules.len())
                .filter(|&i| needle.is_empty() || lines.found.get(i).is_some_and(|(name, what)| name.contains(&needle) || what.contains(&needle)))
                .collect();
            let mut toggled = None;
            let mut remove = None;
            // One line a rule (its switch, name, what it does, and its buttons as icons), in a
            // list that scrolls past a dozen or so; only the lines in view are laid out.
            egui::Frame::new().stroke(egui::Stroke::new(1.0, t.border)).corner_radius(6).show(ui, |ui| {
                // With the find box, a steady height, so the dialog doesn't jump while typing.
                let steady = s.rules.len() > 6;
                ui.spacing_mut().item_spacing.y = 0.0;
                egui::ScrollArea::vertical().max_height(360.0).auto_shrink([false, !steady]).show_rows(ui, 32.0, rows.len(), |ui, range| {
                    for (shown, &i) in rows.iter().enumerate().take(range.end).skip(range.start) {
                        let (Some(r), Some(what)) = (s.rules.get(i), lines.what.get(i)) else { continue };
                        let (row, hover) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::hover());
                        if hover.hovered() {
                            ui.painter().rect_filled(row, 0.0, t.hover_row);
                        } else if shown % 2 == 1 {
                            ui.painter().rect_filled(row, 0.0, t.chrome);
                        }
                        // Each rule's own, wherever the list is scrolled to.
                        let mut inner = ui.new_child(
                            egui::UiBuilder::new()
                                .id_salt(("rule", i))
                                .max_rect(row.shrink2(vec2(8.0, 0.0)))
                                .layout(egui::Layout::left_to_right(egui::Align::Center)),
                        );
                        let ui = &mut inner;
                        let mut on = r.enabled;
                        if ui.checkbox(&mut on, "").on_hover_text(tl!("On / off")).changed() {
                            toggled = Some(i);
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.spacing_mut().item_spacing.x = 2.0;
                            if widgets::tiny_icon_button(ui, Icon::Delete, "Delete", t).clicked() {
                                remove = Some(i);
                            }
                            if widgets::tiny_icon_button(ui, Icon::Drafts, "Edit", t).clicked() {
                                acts.push(Action::Dialog(Dialog::EditRule(Box::new(RuleForm::from_rule(i, r)))));
                            }
                            if widgets::tiny_icon_button(ui, Icon::Sync, "Run now", t).clicked() {
                                acts.push(Action::Run("rules.run", json!({"index": i, "inbox": true})));
                            }
                            ui.add_space(6.0);
                            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                let mut job = egui::text::LayoutJob::default();
                                let strong = egui::TextFormat {
                                    font_id: theme::bold(13.0),
                                    color: if r.enabled { t.text } else { t.faint },
                                    ..Default::default()
                                };
                                job.append(&r.name, 0.0, strong);
                                let dim = egui::TextFormat { font_id: theme::regular(12.0), color: t.dim, ..Default::default() };
                                job.append(what, 10.0, dim);
                                ui.add(egui::Label::new(job).selectable(false).truncate()).on_hover_text(format!("{}\n{}", r.name, what));
                            });
                        });
                    }
                    if rows.is_empty() && !s.rules.is_empty() {
                        ui.add_space(8.0);
                        ui.label(RichText::new(tl!("No rule matches.")).color(t.faint));
                        ui.add_space(8.0);
                    }
                });
            });
            if let Some(i) = remove {
                acts.push(Action::Run("rules.delete", json!({"index": i})));
            } else if let Some(i) = toggled {
                // Copied only now, to send with the one switched.
                let mut rules = s.rules.clone();
                if let Some(r) = rules.get_mut(i) {
                    r.enabled = !r.enabled;
                }
                acts.push(Action::Run("rules.set", json!({"rules": rules})));
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button(tl!("New rule…")).clicked() {
                    acts.push(Action::Dialog(Dialog::rule_from("")));
                }
                let any_on = s.rules.iter().any(|r| r.enabled);
                if ui
                    .add_enabled(any_on, egui::Button::new(tl!("Run all")))
                    .on_hover_text(tl!("Run every rule that's on, on the mail already in the Inbox (and in Sent, for mail you send)"))
                    .clicked()
                {
                    acts.push(Action::Run("rules.run", json!({"inbox": true})));
                }
                // As Outlook's Rules options: the rules to a file, or rules from one.
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add_enabled(!s.rules.is_empty(), egui::Button::new(tl!("Export…"))).clicked() {
                        acts.push(Action::ExportRules);
                    }
                    if ui.button(tl!("Import…")).clicked() {
                        acts.push(Action::ImportRules);
                    }
                });
            });
            if !s.server_rule_accounts().is_empty() {
                egui::ScrollArea::vertical().id_salt("server-rules").max_height(320.0).show(ui, |ui| server_rules::sections(ui, s, t, acts));
            }
            ui.add_space(6.0);
            close_row(ui)
        }
        Dialog::EditRule(f) => {
            ui.set_width(440.0);
            title(ui, t, if f.index.is_some() { "Edit rule" } else { "New rule" });
            egui::Grid::new("rule").num_columns(2).spacing(vec2(10.0, 8.0)).show(ui, |ui| {
                ui.label(tl!("Name"));
                ui.add(egui::TextEdit::singleline(&mut f.name).desired_width(300.0));
                ui.end_row();
                // Mail arriving, or mail sent (issue #202): a rule on a server is for arriving mail.
                if f.server.is_none() {
                    let l = ui.label(tl!("Applies to"));
                    let kinds = [false, true];
                    let said = |sent: bool| if sent { tl!("Mail you send") } else { tl!("Mail that arrives") };
                    let r = egui::ComboBox::from_id_salt("rule-sent")
                        .width(300.0)
                        .selected_text(said(f.sent))
                        .show_ui(ui, |ui| {
                            for k in kinds {
                                ui.selectable_value(&mut f.sent, k, said(k));
                            }
                        })
                        .response
                        .labelled_by(l.id);
                    crate::drop_down::keys(ui, &r, &mut f.sent, &kinds);
                    ui.end_row();
                    // Mail sent isn't list mail.
                    if f.sent && !f.test.for_sent() {
                        f.test = RuleTest::To;
                    }
                }
                let l = ui.label(tl!("When mail"));
                let tests: Vec<RuleTest> =
                    RuleTest::ALL.into_iter().filter(|x| (f.server.is_none() || x.on_server()) && (!f.sent || x.for_sent())).collect();
                let r = egui::ComboBox::from_id_salt("rule-test")
                    .width(300.0)
                    .selected_text(f.test.label())
                    .show_ui(ui, |ui| {
                        for x in tests.iter().copied() {
                            ui.selectable_value(&mut f.test, x, x.label());
                        }
                    })
                    .response
                    .labelled_by(l.id);
                crate::drop_down::keys(ui, &r, &mut f.test, &tests);
                ui.end_row();
                if f.test.needs_text() {
                    ui.label("");
                    ui.add(egui::TextEdit::singleline(&mut f.text).hint_text(tl!("Text or address")).desired_width(300.0));
                    ui.end_row();
                }
                let l = ui.label(tl!("Then"));
                let actions: Vec<RuleDo> = RuleDo::ALL.into_iter().filter(|x| f.server.is_none() || x.on_server()).collect();
                let r = egui::ComboBox::from_id_salt("rule-do")
                    .width(300.0)
                    .selected_text(f.action.label())
                    .show_ui(ui, |ui| {
                        for x in actions.iter().copied() {
                            if ui.selectable_value(&mut f.action, x, x.label()).changed() {
                                f.target.clear();
                            }
                        }
                    })
                    .response
                    .labelled_by(l.id);
                let was = f.action;
                crate::drop_down::keys(ui, &r, &mut f.action, &actions);
                if f.action != was {
                    f.target.clear();
                }
                ui.end_row();
                match f.action {
                    RuleDo::MoveTo => {
                        let l = ui.label(tl!("Folder"));
                        let shown = f
                            .account
                            .as_deref()
                            .and_then(|a| s.folder(a, &f.target))
                            .map(|x| {
                                format!("{} ({})", crate::i18n::folder(x), s.account(&x.account).map(|a| a.config.email.as_str()).unwrap_or_default())
                            })
                            .unwrap_or_else(|| tl!("Choose…").into());
                        // A rule on a server moves mail to that mailbox's folders alone.
                        let server = f.server.as_ref().map(|x| x.account.clone());
                        let accounts: Vec<_> = s.accounts.iter().filter(|a| server.as_ref().is_none_or(|x| *x == a.config.id)).collect();
                        let r = egui::ComboBox::from_id_salt("rule-folder")
                            .width(300.0)
                            .selected_text(shown)
                            .show_ui(ui, |ui| {
                                for a in &accounts {
                                    ui.label(RichText::new(&a.config.email).small().color(t.faint));
                                    for x in a.folders.iter().filter(|x| x.selectable) {
                                        let on = f.account.as_deref() == Some(a.config.id.as_str()) && f.target == x.path;
                                        if ui.selectable_label(on, format!("{}{}", "    ".repeat(x.depth()), crate::i18n::folder(x))).clicked() {
                                            f.account = Some(a.config.id.clone());
                                            f.target = x.path.clone();
                                        }
                                    }
                                }
                            })
                            .response
                            .labelled_by(l.id);
                        let folders: Vec<(Option<String>, String)> = accounts
                            .iter()
                            .flat_map(|a| a.folders.iter().filter(|x| x.selectable).map(|x| (Some(a.config.id.clone()), x.path.clone())))
                            .collect();
                        let mut at = (f.account.clone(), f.target.clone());
                        crate::drop_down::keys(ui, &r, &mut at, &folders);
                        (f.account, f.target) = at;
                        ui.end_row();
                    }
                    RuleDo::Categorize => {
                        let l = ui.label(tl!("Category"));
                        let shown = if f.target.is_empty() { tl!("Choose…").to_string() } else { tl!(&f.target).to_string() };
                        let r = egui::ComboBox::from_id_salt("rule-cat")
                            .width(300.0)
                            .selected_text(shown)
                            .show_ui(ui, |ui| {
                                for c in &s.prefs.categories {
                                    ui.selectable_value(&mut f.target, c.name.clone(), &c.name);
                                }
                            })
                            .response
                            .labelled_by(l.id);
                        let names: Vec<String> = s.prefs.categories.iter().map(|c| c.name.clone()).collect();
                        crate::drop_down::keys(ui, &r, &mut f.target, &names);
                        ui.end_row();
                    }
                    _ => {}
                }
            });
            ui.checkbox(&mut f.stop, tl!("Don't run later rules on mail this one matches"));
            if f.server.is_none() {
                let now = if f.sent { tl!("Run it now on the mail already in Sent") } else { tl!("Run it now on the mail already in the Inbox") };
                ui.checkbox(&mut f.run_now, now);
            }
            if let Some(e) = &f.error {
                ui.label(RichText::new(e.as_str()).color(t.danger).small());
            }
            let (ok, cancel) = buttons(ui, t, "Save", true);
            if ok {
                match (f.to_rule(), &f.server) {
                    // A rule on the server: saved there (it's shown as the server has it).
                    (Ok(rule), Some(x)) => {
                        f.error = None;
                        let p = json!({"account": x.account, "id": x.id, "rule": rule, "dropOutlookList": x.drop_outlook_list});
                        acts.push(Action::Submit("rules.serverSave", p));
                    }
                    (Ok(rule), None) => {
                        let mut rules = s.rules.clone();
                        let at = match f.index.filter(|i| *i < rules.len()) {
                            Some(i) => {
                                let enabled = rules.get(i).is_none_or(|r| r.enabled);
                                if let Some(slot) = rules.get_mut(i) {
                                    *slot = Rule { enabled, ..rule };
                                }
                                i
                            }
                            None => {
                                rules.push(rule);
                                rules.len() - 1
                            }
                        };
                        f.error = None;
                        // Run once it's saved, and only then (`runNow`, read where the save is
                        // answered): run whatever came of the save, a save refused left the rule
                        // as it was before, and that one ran over the Inbox.
                        let run = if f.run_now { json!(at) } else { serde_json::Value::Null };
                        acts.push(Action::Submit("rules.set", json!({"rules": rules, "runNow": run})));
                    }
                    (Err(e), _) => f.error = Some(e),
                }
            }
            !cancel
        }
        Dialog::SearchFolder(f) => {
            ui.set_width(460.0);
            title(ui, t, if f.original.is_some() { "Edit Search Folder" } else { "New Search Folder" });
            ui.add(
                egui::Label::new(RichText::new(tl!("Shows the mail it finds in every folder, except Deleted Items and Junk Email.")).color(t.dim))
                    .wrap(),
            );
            ui.add_space(8.0);
            let l = ui.label(RichText::new(tl!("Show")).color(t.dim).small());
            let before = f.preset;
            let r = egui::ComboBox::from_id_salt("search-folder-preset")
                .width(ui.available_width())
                .selected_text(tl!(f.preset.label()))
                .show_ui(ui, |ui| {
                    for (label, kind) in SEARCH_PRESETS {
                        ui.selectable_value(&mut f.preset, kind, tl!(label));
                    }
                })
                .response
                .labelled_by(l.id);
            let kinds: Vec<PresetKind> = SEARCH_PRESETS.iter().map(|(_, kind)| *kind).collect();
            crate::drop_down::keys(ui, &r, &mut f.preset, &kinds);
            if f.preset != before {
                f.apply_preset(before);
            }
            if f.preset == PresetKind::People {
                ui.add_space(6.0);
                ui.label(RichText::new(tl!("From these people (addresses or names, separated by commas)")).color(t.dim).small());
                ui.add(egui::TextEdit::singleline(&mut f.people).hint_text("ana@example.com, Jun").desired_width(f32::INFINITY));
            }
            if f.preset == PresetKind::Custom {
                ui.add_space(6.0);
                ui.label(RichText::new(tl!("Search for")).color(t.dim).small());
                ui.add(egui::TextEdit::singleline(&mut f.query).hint_text("from:ana subject:invoice has:attachment").desired_width(f32::INFINITY));
            }
            ui.add_space(6.0);
            ui.label(RichText::new(tl!("Name")).color(t.dim).small());
            let r = ui.add(egui::TextEdit::singleline(&mut f.name).desired_width(f32::INFINITY));
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if let Some(e) = &f.error {
                ui.add_space(4.0);
                ui.label(RichText::new(e.as_str()).color(t.danger).small());
            }
            let query = f.final_query();
            let ready = !f.name.trim().is_empty() && !query.trim().is_empty();
            let (ok, cancel) = buttons(ui, t, if f.original.is_some() { "Save" } else { "Create" }, ready);
            if (ok || enter) && ready {
                let p = match &f.original {
                    Some(old) => json!({"name": old, "newName": f.name.trim(), "query": query}),
                    None => json!({"name": f.name.trim(), "query": query}),
                };
                acts.push(Action::Submit(if f.original.is_some() { "searchFolder.edit" } else { "searchFolder.create" }, p));
            }
            !cancel
        }
        Dialog::QuickStep(f) => {
            ui.set_width(440.0);
            title(ui, t, if f.original.is_some() { "Edit Quick Step" } else { "New Quick Step" });
            ui.add(
                egui::Label::new(
                    RichText::new(tl!("Done to the selected mail in one click; the first nine also with Ctrl+Shift+1 to 9.")).color(t.dim),
                )
                .wrap(),
            );
            ui.add_space(8.0);
            ui.label(RichText::new(tl!("Name")).color(t.dim).small());
            let r = ui.add(egui::TextEdit::singleline(&mut f.name).hint_text(tl!("Done")).desired_width(f32::INFINITY));
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            ui.add_space(6.0);
            ui.checkbox(&mut f.read, tl!("Mark as read"));
            ui.checkbox(&mut f.flag, tl!("Flag"));
            ui.checkbox(&mut f.pin, tl!("Pin"));
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let l = ui.label(tl!("Categorize"));
                let shown = if f.category.is_empty() { tl!("None").to_string() } else { f.category.clone() };
                let r = egui::ComboBox::from_id_salt("quick-step-category")
                    .width(220.0)
                    .selected_text(shown)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut f.category, String::new(), tl!("None"));
                        for c in &s.prefs.categories {
                            ui.selectable_value(&mut f.category, c.name.clone(), c.name.as_str());
                        }
                    })
                    .response
                    .labelled_by(l.id);
                let names: Vec<String> = std::iter::once(String::new()).chain(s.prefs.categories.iter().map(|c| c.name.clone())).collect();
                crate::drop_down::keys(ui, &r, &mut f.category, &names);
            });
            ui.horizontal(|ui| {
                let l = ui.label(tl!("Then move to"));
                let mut choices =
                    vec![StepMove::Stay, StepMove::Role(FolderRole::Archive), StepMove::Role(FolderRole::Trash), StepMove::Role(FolderRole::Junk)];
                choices.extend(f.folders.iter().map(|p| StepMove::Folder(p.clone())));
                let r = egui::ComboBox::from_id_salt("quick-step-move")
                    .width(220.0)
                    .selected_text(QuickStepForm::move_label(&f.move_to))
                    .show_ui(ui, |ui| {
                        for c in &choices {
                            let label = QuickStepForm::move_label(c);
                            ui.selectable_value(&mut f.move_to, c.clone(), label);
                        }
                    })
                    .response
                    .labelled_by(l.id);
                crate::drop_down::keys(ui, &r, &mut f.move_to, &choices);
            });
            if let Some(e) = &f.error {
                ui.add_space(4.0);
                ui.label(RichText::new(e.as_str()).color(t.danger).small());
            }
            let ready = !f.name.trim().is_empty() && !f.actions().is_empty();
            if let Some(old) = f.original.clone() {
                ui.add_space(6.0);
                if ui.button(RichText::new(tl!("Delete this Quick Step")).color(t.danger)).clicked() {
                    acts.push(Action::Submit("quickStep.delete", json!({"name": old})));
                }
            }
            let (ok, cancel) = buttons(ui, t, if f.original.is_some() { "Save" } else { "Create" }, ready);
            if (ok || enter) && ready {
                let p = match &f.original {
                    Some(old) => json!({"name": old, "newName": f.name.trim(), "actions": f.actions()}),
                    None => json!({"name": f.name.trim(), "actions": f.actions()}),
                };
                acts.push(Action::Submit(if f.original.is_some() { "quickStep.edit" } else { "quickStep.create" }, p));
            }
            !cancel
        }
        // Every other kind is shown by its own group (`body`).
        _ => true,
    }
}

/// What the rule editor offers: one test and one action (rules made elsewhere can have more).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuleTest {
    From,
    To,
    Subject,
    AnyText,
    HasAttachment,
    Bulk,
}

impl RuleTest {
    pub(super) const ALL: [RuleTest; 6] =
        [RuleTest::From, RuleTest::To, RuleTest::Subject, RuleTest::AnyText, RuleTest::HasAttachment, RuleTest::Bulk];
    pub(super) fn label(self) -> &'static str {
        tl!(match self {
            RuleTest::From => "From contains",
            RuleTest::To => "To or Cc contains",
            RuleTest::Subject => "Subject contains",
            RuleTest::AnyText => "Sender, recipients or subject contain",
            RuleTest::HasAttachment => "Has attachments",
            RuleTest::Bulk => "Is a newsletter or list mail",
        })
    }
    pub(super) fn needs_text(self) -> bool {
        !matches!(self, RuleTest::HasAttachment | RuleTest::Bulk)
    }
    /// Whether a rule on an Exchange server can look at it (issue #139).
    pub(super) fn on_server(self) -> bool {
        !matches!(self, RuleTest::AnyText | RuleTest::Bulk)
    }
    /// Whether it says anything of mail the user sends (issue #202).
    pub(super) fn for_sent(self) -> bool {
        self != RuleTest::Bulk
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuleDo {
    MoveTo,
    Archive,
    Delete,
    Categorize,
    MarkRead,
    Flag,
    Pin,
}

impl RuleDo {
    /// Whether a rule on an Exchange server can do it (issue #139).
    pub(super) fn on_server(self) -> bool {
        !matches!(self, RuleDo::Flag | RuleDo::Pin)
    }
    pub(super) const ALL: [RuleDo; 7] =
        [RuleDo::MoveTo, RuleDo::Archive, RuleDo::Delete, RuleDo::Categorize, RuleDo::MarkRead, RuleDo::Flag, RuleDo::Pin];
    pub(super) fn label(self) -> &'static str {
        tl!(match self {
            RuleDo::MoveTo => "Move to folder",
            RuleDo::Archive => "Archive",
            RuleDo::Delete => "Delete",
            RuleDo::Categorize => "Categorize",
            RuleDo::MarkRead => "Mark as read",
            RuleDo::Flag => "Flag",
            RuleDo::Pin => "Pin",
        })
    }
}

#[derive(Clone, Debug)]
pub struct RuleForm {
    /// The rule being changed (`None`: a new one).
    pub index: Option<usize>,
    pub name: String,
    pub test: RuleTest,
    pub text: String,
    pub action: RuleDo,
    /// A folder path (`MoveTo`) or a category name (`Categorize`).
    pub target: String,
    /// The account whose folders `MoveTo` offers (and the only one it applies to).
    pub account: Option<String>,
    pub stop: bool,
    /// Run it on the mail already in the Inbox (or Sent) once saved (Outlook's "run this rule
    /// now").
    pub run_now: bool,
    /// On mail the user sends, not mail arriving (issue #202).
    pub sent: bool,
    pub error: Option<String>,
    /// The rule as it was when opened: what the editor doesn't show of it (more conditions or
    /// actions, "any" of them, the account it's for) is kept, and of what it shows only what was
    /// changed goes in.
    pub original: Option<Rule>,
    /// A rule on an Exchange account's server (issue #139); `None`: MailCraft's own.
    pub server: Option<ServerTarget>,
}

/// What the editor shows of a rule: its first condition and its first action.
fn shown(r: &Rule) -> (RuleTest, String, RuleDo, String) {
    let (test, text) = match r.conditions.first() {
        Some(Condition::From(t)) => (RuleTest::From, t.clone()),
        Some(Condition::To(t)) => (RuleTest::To, t.clone()),
        Some(Condition::Subject(t)) => (RuleTest::Subject, t.clone()),
        Some(Condition::AnyText(t)) => (RuleTest::AnyText, t.clone()),
        Some(Condition::HasAttachment) => (RuleTest::HasAttachment, String::new()),
        Some(Condition::Bulk) | None => (RuleTest::Bulk, String::new()),
    };
    let (action, target) = match r.actions.first() {
        Some(RuleAction::MoveTo(f)) => (RuleDo::MoveTo, f.clone()),
        Some(RuleAction::MoveToRole(FolderRole::Trash)) | Some(RuleAction::Delete) => (RuleDo::Delete, String::new()),
        Some(RuleAction::MoveToRole(_)) => (RuleDo::Archive, String::new()),
        Some(RuleAction::Categorize(c)) => (RuleDo::Categorize, c.clone()),
        Some(RuleAction::MarkRead) => (RuleDo::MarkRead, String::new()),
        Some(RuleAction::Flag) => (RuleDo::Flag, String::new()),
        Some(RuleAction::Pin) | None => (RuleDo::Pin, String::new()),
    };
    (test, text, action, target)
}

/// `new` in the place of a list's first, or as its only one.
fn put_first<T>(list: &mut Vec<T>, new: T) {
    match list.first_mut() {
        Some(first) => *first = new,
        None => list.push(new),
    }
}

impl RuleForm {
    pub(crate) fn to_rule(&self) -> Result<Rule, String> {
        if self.name.trim().is_empty() {
            return Err(tl!("Give the rule a name").to_string());
        }
        if self.test.needs_text() && self.text.trim().is_empty() {
            return Err(tl!("Say what to look for").to_string());
        }
        let t = self.text.trim().to_string();
        let condition = match self.test {
            RuleTest::From => Condition::From(t),
            RuleTest::To => Condition::To(t),
            RuleTest::Subject => Condition::Subject(t),
            RuleTest::AnyText => Condition::AnyText(t),
            RuleTest::HasAttachment => Condition::HasAttachment,
            RuleTest::Bulk => Condition::Bulk,
        };
        let action = match self.action {
            RuleDo::MoveTo if self.target.is_empty() => return Err(tl!("Pick a folder").to_string()),
            RuleDo::MoveTo => RuleAction::MoveTo(self.target.clone()),
            RuleDo::Archive => RuleAction::MoveToRole(FolderRole::Archive),
            RuleDo::Delete => RuleAction::Delete,
            RuleDo::Categorize if self.target.is_empty() => return Err(tl!("Pick a category").to_string()),
            RuleDo::Categorize => RuleAction::Categorize(self.target.clone()),
            RuleDo::MarkRead => RuleAction::MarkRead,
            RuleDo::Flag => RuleAction::Flag,
            RuleDo::Pin => RuleAction::Pin,
        };
        let Some(original) = &self.original else {
            return Ok(Rule {
                name: self.name.trim().to_string(),
                enabled: true,
                account: if self.action == RuleDo::MoveTo { self.account.clone() } else { None },
                conditions: vec![condition],
                match_all: true,
                actions: vec![action],
                stop: self.stop,
                sent: self.sent,
            });
        };
        let (test, text, act, target) = shown(original);
        let mut rule = Rule { name: self.name.trim().to_string(), stop: self.stop, sent: self.sent, ..original.clone() };
        if self.test != test || self.text.trim() != text.trim() {
            put_first(&mut rule.conditions, condition);
        }
        let moved_elsewhere = self.action == RuleDo::MoveTo && self.account != original.account;
        if self.action != act || self.target != target || moved_elsewhere {
            put_first(&mut rule.actions, action);
            // The account a move's folder is in is the one it's for; other actions keep theirs.
            if self.action == RuleDo::MoveTo {
                rule.account.clone_from(&self.account);
            } else if act == RuleDo::MoveTo && !rule.actions.iter().any(|a| matches!(a, RuleAction::MoveTo(_))) {
                // The move's account, which goes with the move: no other action shows one, and a
                // new rule of this kind has none.
                rule.account = None;
            }
        }
        Ok(rule)
    }

    pub(crate) fn from_rule(index: usize, r: &Rule) -> RuleForm {
        let (test, text, action, target) = shown(r);
        RuleForm {
            index: Some(index),
            name: r.name.clone(),
            test,
            text,
            action,
            target,
            account: r.account.clone(),
            stop: r.stop,
            run_now: false,
            sent: r.sent,
            error: None,
            original: Some(r.clone()),
            server: None,
        }
    }

    /// A rule on an account's server: a new one (moving to one of its folders, the first
    /// offered), or `existing` as MailCraft can say it.
    pub(crate) fn for_server(target: ServerTarget, existing: Option<&mailcraft_engine::model::organize::ServerRule>) -> RuleForm {
        let account = Some(target.account.clone());
        match existing.and_then(|r| r.rule.as_ref()) {
            Some(rule) => RuleForm { server: Some(target), ..RuleForm::from_rule(0, rule) },
            None => RuleForm {
                index: None,
                name: String::new(),
                test: RuleTest::Subject,
                text: String::new(),
                action: RuleDo::MoveTo,
                target: String::new(),
                account,
                stop: false,
                run_now: false,
                sent: false,
                error: None,
                original: None,
                server: Some(target),
            },
        }
    }
}

/// What each rule does, in words, and its name and that in lower case for the find box: kept
/// between frames, and made again only when the rules, the language or the folders change.
struct RuleLines {
    rules: Vec<Rule>,
    key: u64,
    what: Vec<String>,
    found: Vec<(String, String)>,
}

/// The Rules dialog's lines for `s.rules` (see `RuleLines`).
fn rule_lines(ctx: &egui::Context, s: &Session) -> std::sync::Arc<RuleLines> {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    // The language the lines are in, itself: two languages can word a template alike.
    crate::i18n::current().code().hash(&mut h);
    // A move names its folder as the folder pane does, by each folder's delimiter.
    for f in s.accounts.iter().flat_map(|a| &a.folders) {
        (&f.path, f.delimiter).hash(&mut h);
    }
    let key = h.finish();
    let id = egui::Id::unique("rules-lines");
    if let Some(kept) = ctx.data(|d| d.get_temp::<std::sync::Arc<RuleLines>>(id))
        && kept.key == key
        && kept.rules == s.rules
    {
        return kept;
    }
    let what: Vec<String> = s.rules.iter().map(|r| describe(s, r)).collect();
    let found = s.rules.iter().zip(&what).map(|(r, w)| (r.name.to_lowercase(), w.to_lowercase())).collect();
    let lines = std::sync::Arc::new(RuleLines { rules: s.rules.clone(), key, what, found });
    ctx.data_mut(|d| d.insert_temp(id, lines.clone()));
    lines
}

/// A folder as the folder pane names it, with the folders it's in: `Inbox › gzamer`.
pub(super) fn folder_path_label(s: &Session, path: &str) -> String {
    // The folder's own delimiter; "/" (the commonest) for one not synced yet.
    let delimiter = s.accounts.iter().flat_map(|a| &a.folders).find(|f| f.path == path).map_or(Some('/'), |f| f.delimiter);
    let decoded = mailcraft_engine::model::utf7::decode(path);
    let parts: Vec<&str> = match delimiter {
        Some(d) => decoded.split(d).collect(),
        None => vec![decoded.as_str()],
    };
    parts
        .iter()
        .enumerate()
        .map(|(i, p)| if i == 0 && p.eq_ignore_ascii_case("INBOX") { tl!("Inbox").to_string() } else { crate::i18n::tc("folder", p).to_string() })
        .collect::<Vec<_>>()
        .join(" › ")
}

/// One line describing a rule.
pub(super) fn describe(s: &Session, r: &Rule) -> String {
    let cond = r
        .conditions
        .iter()
        .map(|c| match c {
            Condition::From(t) => tf!("from “{text}”", text = t),
            Condition::To(t) => tf!("to “{text}”", text = t),
            Condition::Subject(t) => tf!("subject has “{text}”", text = t),
            Condition::AnyText(t) => tf!("mentions “{text}”", text = t),
            Condition::HasAttachment => tl!("has attachments").to_string(),
            Condition::Bulk => tl!("is list mail").to_string(),
        })
        .collect::<Vec<_>>()
        .join(&format!(" {} ", if r.match_all { tl!("and") } else { tl!("or") }));
    let act = r
        .actions
        .iter()
        .map(|a| match a {
            RuleAction::MoveTo(f) => tf!("move to {folder}", folder = folder_path_label(s, f)),
            RuleAction::MoveToRole(role) => tf!("move to {folder}", folder = tl!(role.label().unwrap_or("folder"))),
            RuleAction::MarkRead => tl!("mark read").to_string(),
            RuleAction::Flag => tl!("flag").to_string(),
            RuleAction::Pin => tl!("pin").to_string(),
            RuleAction::Categorize(c) => tf!("categorize {category}", category = c),
            RuleAction::Delete => tl!("delete").to_string(),
        })
        .collect::<Vec<_>>()
        .join(", ");
    if r.sent {
        tf!("If mail you send {condition}: {action}", condition = cond, action = act)
    } else {
        tf!("If it {condition}: {action}", condition = cond, action = act)
    }
}
