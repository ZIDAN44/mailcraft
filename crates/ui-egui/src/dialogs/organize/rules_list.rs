//! Dialogs: the Rules list, each rule with its switch, name, what it does and its buttons, then
//! the rules on the server.

use super::*;

pub(super) fn rules_list(ui: &mut Ui, s: &Session, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    ui.set_width(crate::sizes::DIALOG_L);
    title(ui, t, "Rules");
    ui.label(
        RichText::new(tl!("Rules run in this order, on new mail as it arrives in an Inbox, and on mail you send as its copy reaches Sent."))
            .color(t.dim)
            .small(),
    );
    ui.add_space(6.0);
    // What the list is worked with, over it; Close alone under it (Close was on a row of its own
    // under them).
    ui.horizontal(|ui| {
        if ui.button(tl!("New rule…")).clicked() {
            acts.push(Action::Dialog(Dialog::rule_from("", None)));
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
    ui.add_space(4.0);
    // The rule just deleted, with Undo (deleting one asks nothing first).
    if let Some((name, rules)) = deleted::last::<(String, Vec<Rule>)>(ui, "rules")
        && deleted::undo_line(ui, t, &name)
    {
        deleted::forget::<(String, Vec<Rule>)>(ui, "rules");
        acts.push(Action::Run("rules.set", json!({"rules": rules})));
    }
    // Many rules: a box to find one by its name or what it does.
    let find_id = egui::Id::unique("rules-find");
    let mut find: String = ui.ctx().data(|d| d.get_temp(find_id)).unwrap_or_default();
    if s.rules.len() > 6 {
        ui.add(egui::TextEdit::singleline(&mut find).hint_text(tl!("Find a rule")).desired_width(f32::INFINITY));
        ui.ctx().data_mut(|d| d.insert_temp(find_id, find.clone()));
        ui.add_space(4.0);
    }
    let needle = find.trim().to_lowercase();
    // None yet: what a rule is for, in the list's place (it was "No rules yet." alone, over an
    // empty line).
    if s.rules.is_empty() {
        egui::Frame::new().stroke(egui::Stroke::new(1.0, t.border)).corner_radius(6).inner_margin(Margin::symmetric(16, 20)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.vertical_centered(|ui| {
                ui.label(RichText::new(tl!("No rules yet.")).color(t.text));
                ui.label(RichText::new(tl!("A rule files, flags or answers mail as it arrives. New rule… makes one.")).color(t.dim).small());
            });
        });
    }
    let lines = rule_lines(ui.ctx(), s);
    // The rules the find box keeps, in order.
    let rows: Vec<usize> = (0..s.rules.len())
        .filter(|&i| needle.is_empty() || lines.found.get(i).is_some_and(|(name, what)| name.contains(&needle) || what.contains(&needle)))
        .collect();
    let mut toggled = None;
    let mut remove = None;
    // A rule moved up or down the list (they run in its order): from where, to where.
    let mut moved: Option<(usize, usize)> = None;
    // One line a rule (its switch, name, what it does, and its buttons as icons), in a
    // list that scrolls past a dozen or so; only the lines in view are laid out.
    let listed = !s.rules.is_empty();
    egui::Frame::new().stroke(egui::Stroke::new(if listed { 1.0 } else { 0.0 }, t.border)).corner_radius(6).show(ui, |ui| {
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
                    // Apart enough that Delete isn't hit for Edit beside it; seen while the row
                    // is pointed at.
                    ui.spacing_mut().item_spacing.x = 6.0;
                    widgets::row_actions(ui, row, |ui| {
                        if widgets::tiny_icon_button(ui, Icon::Delete, "Delete", t).clicked() {
                            remove = Some(i);
                        }
                        if widgets::tiny_icon_button(ui, Icon::Drafts, "Edit", t).clicked() {
                            acts.push(Action::Dialog(Dialog::EditRule(Box::new(RuleForm::from_rule(i, r)))));
                        }
                        if widgets::tiny_icon_button(ui, Icon::Sync, "Run now", t).clicked() {
                            acts.push(Action::Run("rules.run", json!({"index": i, "inbox": true})));
                        }
                        let last = i + 1 >= s.rules.len();
                        if ui.add_enabled_ui(!last, |ui| widgets::tiny_icon_button(ui, Icon::ChevronDown, "Move down", t)).inner.clicked() {
                            moved = Some((i, i + 1));
                        }
                        if ui.add_enabled_ui(i > 0, |ui| widgets::tiny_icon_button(ui, Icon::ChevronUp, "Move up", t)).inner.clicked() {
                            moved = i.checked_sub(1).map(|to| (i, to));
                        }
                    });
                    ui.add_space(6.0);
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        let mut job = egui::text::LayoutJob::default();
                        let strong = egui::TextFormat {
                            font_id: theme::semibold(crate::sizes::BODY),
                            color: if r.enabled { t.text } else { t.faint },
                            ..Default::default()
                        };
                        job.append(&r.name, 0.0, strong);
                        let dim = egui::TextFormat { font_id: theme::regular(crate::sizes::CAPTION), color: t.dim, ..Default::default() };
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
        if let Some(r) = s.rules.get(i) {
            deleted::remember(ui, "rules", (r.name.clone(), s.rules.clone()));
        }
        acts.push(Action::Run("rules.delete", json!({"index": i})));
    } else if let Some((from, to)) = moved.filter(|&(f, t)| f < s.rules.len() && t < s.rules.len()) {
        let mut rules = s.rules.clone();
        rules.swap(from, to);
        deleted::forget::<(String, Vec<Rule>)>(ui, "rules");
        acts.push(Action::Run("rules.set", json!({"rules": rules})));
    } else if let Some(i) = toggled {
        // Copied only now, to send with the one switched.
        let mut rules = s.rules.clone();
        if let Some(r) = rules.get_mut(i) {
            r.enabled = !r.enabled;
        }
        acts.push(Action::Run("rules.set", json!({"rules": rules})));
    }
    if !s.server_rule_accounts().is_empty() {
        egui::ScrollArea::vertical().id_salt("server-rules").max_height(320.0).show(ui, |ui| server_rules::sections(ui, s, t, acts));
    }
    ui.add_space(6.0);
    close_row(ui)
}
