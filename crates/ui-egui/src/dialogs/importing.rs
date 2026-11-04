//! Dialogs: a data file's import. What a chosen file holds and where its copy goes (an Outlook
//! file's beside it, or the copy made there before, and whether the file changed since), before
//! it's imported; stopping an import; removing an account while its file is imported; and what
//! an import came to, every loss by its reason.

use mailcraft_engine::model::job::{ImportReport, SkipReason};

use super::*;
use crate::app::importing::{duration, inbox_of};
use crate::progress;

/// The sentence for `n` items left out for `reason`.
pub(crate) fn skip_line(reason: SkipReason, n: u64) -> String {
    match reason {
        SkipReason::MessageUnreadable => tf!("{n} messages couldn't be read", n = n),
        SkipReason::MessageNotRebuilt => tf!("{n} messages couldn't be put back together", n = n),
        SkipReason::NotWritten => tf!("{n} messages couldn't be saved", n = n),
        SkipReason::ListsUnwritten => tf!("{n} folders' lists couldn't be saved (their mail is kept, unlisted)", n = n),
        SkipReason::AttachmentUnreadable => tf!("{n} attachments couldn't be read", n = n),
        SkipReason::AttachmentTooBig => tf!("{n} attachments were too big to bring in", n = n),
        SkipReason::AttachmentLink => tf!("{n} attachments were only links to files elsewhere", n = n),
        SkipReason::AttachedMessageUnreadable => tf!("{n} attached messages couldn't be read", n = n),
        SkipReason::AttachedMessageFilesLeftOut => tf!("{n} attached messages lost some of their files", n = n),
        SkipReason::EmbeddedTooDeep => tf!("{n} messages were attached too deep inside others", n = n),
        SkipReason::RecipientWithoutAddress => tf!("{n} recipients had no address", n = n),
        SkipReason::BodyNotConverted => tf!("{n} message texts couldn't be converted", n = n),
        SkipReason::CharsetUnsupported => tf!("{n} message texts were in a character set MailCraft can't read", n = n),
        SkipReason::FolderUnreadable => tf!("{n} folders couldn't be read", n = n),
        SkipReason::FolderTooDeep => tf!("{n} folders were nested too deep", n = n),
        SkipReason::FolderLeftOut => tf!("{n} folders of a kind MailCraft has no place for", n = n),
        SkipReason::ItemInFolderLeftOut => tf!("{n} items in those folders", n = n),
        SkipReason::ItemUnreadable => tf!("{n} contacts, calendar items, tasks or notes couldn't be read", n = n),
        SkipReason::ItemNotUsable => tf!("{n} items couldn't be used (such as an empty note)", n = n),
        SkipReason::StoppedEarly => tf!("The file is damaged: reading stopped early ({n})", n = n),
        SkipReason::Other => tf!("{n} items for other reasons", n = n),
    }
}

/// A note in the dialog's own colour, wrapped.
fn say(ui: &mut Ui, words: String, color: egui::Color32) {
    ui.add(egui::Label::new(RichText::new(words).color(color)).wrap());
}

/// The data file's name at the end of `path`.
fn file_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

/// The data file on a card: a file's mark, its name (its whole path on hover) and, under it,
/// `holds` (its size and what's in it, or that it's being counted).
fn file_card(ui: &mut Ui, t: &Tokens, path: &str, holds: &str) {
    egui::Frame::new().fill(t.bg).stroke(egui::Stroke::new(1.0, t.border)).corner_radius(8).inner_margin(egui::Margin::symmetric(12, 10)).show(
        ui,
        |ui| {
            ui.set_width(ui.available_width());
            ui.allocate_ui_with_layout(vec2(ui.available_width(), 36.0), crate::bidi::row_from_start(egui::Align::Center), |ui| {
                let (mark, _) = ui.allocate_exact_size(vec2(36.0, 36.0), egui::Sense::hover());
                ui.painter().rect_filled(mark, 8.0, t.accent_soft);
                crate::icons::paint(ui.painter(), egui::Rect::from_center_size(mark.center(), vec2(20.0, 20.0)), Icon::File, t.accent);
                ui.add_space(4.0);
                crate::bidi::vertical(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.add(egui::Label::new(RichText::new(file_name(path)).font(crate::theme::bold(15.0)).color(t.text)).truncate())
                        .on_hover_text(path);
                    if !holds.is_empty() {
                        ui.add(egui::Label::new(RichText::new(holds).color(t.dim)).truncate());
                    }
                });
            });
        },
    );
}

/// The room the copy needs (`need` bytes) out of what's free where it goes: a bar of the free
/// space, the copy's share of it filled (red when it doesn't fit), and how much is free.
fn room(ui: &mut Ui, t: &Tokens, need: u64, free: u64, short: bool) {
    let (bar, _) = ui.allocate_exact_size(vec2(ui.available_width(), 6.0), egui::Sense::hover());
    ui.painter().rect_filled(bar, 3.0, t.border);
    let share = if short || free == 0 { 1.0 } else { (need as f64 / free as f64).clamp(0.02, 1.0) as f32 };
    let rtl = crate::i18n::current().rtl();
    let fill = if rtl { bar.with_min_x(bar.max.x - bar.width() * share) } else { bar.with_max_x(bar.min.x + bar.width() * share) };
    ui.painter().rect_filled(fill, 3.0, if short { t.danger } else { t.accent });
    ui.add_space(2.0);
    say(ui, tf!("Free there: {free}", free = widgets::human_bytes(free)), t.dim);
}

pub(super) fn draw(ui: &mut Ui, d: &mut Dialog, s: &Session, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    match d {
        Dialog::OpenDataFile { path } => open_data_file(ui, path, s, t, acts),
        Dialog::StopImport { account, file } => {
            ui.set_width(400.0);
            title(ui, t, &tf!("Stop importing {file}?", file = file));
            say(ui, tl!("What's imported so far is kept. You can continue later.").to_string(), t.dim);
            ui.add_space(10.0);
            let mut keep = true;
            button_row(ui, |ui| {
                if ui.button(tl!("Keep importing")).clicked() {
                    keep = false;
                }
                if widgets::accent_button(ui, Icon::Close, "Stop", t, true).clicked() {
                    acts.push(Action::Run("account.stopImport", json!({"account": account})));
                    keep = false;
                }
            });
            keep
        }
        // Removing an account whose data file is being imported: the part imported goes.
        Dialog::RemoveAccount { account, email } => {
            ui.set_width(400.0);
            title(ui, t, &tf!("Stop importing and remove {file}?", file = email));
            say(ui, tl!("The part imported so far is deleted. The data file itself isn't changed.").to_string(), t.dim);
            let (ok, cancel) = buttons(ui, t, "Remove", true);
            if ok {
                acts.push(Action::Run("account.remove", json!({"account": account})));
                return false;
            }
            !cancel
        }
        Dialog::ImportDone { account, report } => import_done(ui, account, report, s, t, acts),
        _ => true,
    }
}

/// The data file chosen to open, before it's imported: what it holds, where MailCraft's copy goes
/// (an Outlook file's beside it, never into the file itself) and whether there's room for it, the
/// copy made there before, and the account it's open as already, if any.
fn open_data_file(ui: &mut Ui, path: &str, s: &Session, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    ui.set_width(480.0);
    title(ui, t, "Open Outlook data file");
    // The file, and what it holds, on a card of its own: what the rest of the dialog is about.
    let facts = s.data_file_facts(path);
    let holds = match facts {
        None => tl!("Counting items…").to_string(),
        Some(f) if f.error.is_some() => String::new(),
        Some(f) => {
            let items = tf!("{items} items in {folders} folders", items = f.messages.saturating_add(f.pim_items), folders = f.folders);
            if f.size > 0 { format!("{}   ·   {items}", widgets::human_bytes(f.size)) } else { items }
        }
    };
    file_card(ui, t, path, &holds);
    ui.add_space(10.0);
    let mut can_import = false;
    // MailCraft's copy beside the file from before, and whether the file changed since.
    let (mut copy, mut changed, mut rereadable) = (false, false, false);
    // That copy an account in another MailCraft data folder: the folder.
    let mut held = String::new();
    match facts {
        None => {
            // Not looked at (or forgotten since): asked for, once.
            if !s.inspecting(path) {
                acts.push(Action::Run("account.inspectDataFile", json!({"path": path})));
            }
            progress::bar(ui, t, None, vec2(ui.available_width(), 4.0));
        }
        Some(f) if f.error.is_some() => {
            let error = f.error.clone().unwrap_or_default();
            say(ui, tf!("This file couldn't be looked at: {error}", error = error), t.warn);
            can_import = true;
        }
        Some(f) => {
            let outlook = f.kind.starts_with("pst");
            (copy, changed, rereadable) = (!f.copy.is_empty(), f.copy_changed, f.copy_rereadable);
            let about = widgets::human_bytes(f.estimate);
            if copy {
                say(ui, tf!("MailCraft's copy of this file is beside it, with what was changed in MailCraft: {file}", file = f.copy), t.dim);
                if !f.copy_held_by.is_empty() {
                    held.clone_from(&f.copy_held_by);
                    say(ui, tf!("It's an account in another MailCraft data folder: {folder}", folder = held), t.warn);
                    say(
                        ui,
                        tl!("Open it there, or take it over here: it's then this data folder's, and its account there should be removed.")
                            .to_string(),
                        t.dim,
                    );
                }
                if changed {
                    say(ui, tl!("The file has changed since MailCraft read it.").to_string(), t.warn);
                    if !rereadable {
                        say(
                            ui,
                            tl!("MailCraft has no note of what came in from this file before, so its changes can't be read in.").to_string(),
                            t.dim,
                        );
                    }
                }
            } else if f.estimate > 0 && !f.destination.is_empty() {
                let words = if outlook && f.elsewhere {
                    tf!(
                        "This file's folder can't be written: MailCraft's copy, about {size}, goes in {folder}.",
                        size = about,
                        folder = f.destination
                    )
                } else if outlook {
                    tf!(
                        "MailCraft keeps its own copy beside the file, about {size}, in {folder}. The file itself is never changed.",
                        size = about,
                        folder = f.destination
                    )
                } else {
                    tf!("MailCraft makes its own copy, about {size}, in {folder}.", size = about, folder = f.destination)
                };
                say(ui, words, t.dim);
            }
            let short = f.free.is_some_and(|free| free < f.need);
            if let Some(free) = f.free.filter(|_| f.need > 0) {
                ui.add_space(8.0);
                room(ui, t, f.need, free, short);
            }
            if short {
                ui.add_space(4.0);
                // Beside the file, only room on its drive helps.
                let words = if outlook && !f.elsewhere && !changed {
                    tl!("There isn't room for the copy beside the file. Free up space on that drive.")
                } else {
                    tl!("There isn't room for the copy. Free up space, or move MailCraft's data folder (File › Options).")
                };
                say(ui, words.to_string(), t.danger);
            }
            can_import = !short;
        }
    }
    // Open as an account already: go to it, go on with it, or import another copy.
    if let Some(other) = s.data_file_open_as(path) {
        let name = s.account(&other).map(|a| a.config.name.clone()).unwrap_or_default();
        ui.add_space(6.0);
        say(ui, tf!("This data file is already open as {name}.", name = name), t.warn);
        // Changed since its copy was read: what's new can be read into it.
        let stale = s
            .data_file_facts(path)
            .zip(s.data_source(&other))
            .is_some_and(|(f, r)| f.error.is_none() && f.kind.starts_with("pst") && r.size > 0 && (f.size, f.modified) != (r.size, r.modified));
        // Its copy notes what came in from the file before: what's new can be told from what was
        // deleted here.
        let rereadable = stale && s.can_read_changes(&other);
        if stale {
            say(ui, tl!("The file has changed since MailCraft read it.").to_string(), t.warn);
            if !rereadable {
                say(ui, tl!("MailCraft has no note of what came in from this file before, so its changes can't be read in.").to_string(), t.dim);
            }
        }
        let mut keep = true;
        ui.add_space(6.0);
        button_row(ui, |ui| {
            if ui.button(tl!("Cancel")).clicked() {
                keep = false;
            }
            if ui.add_enabled(can_import, egui::Button::new(tl!("Import another copy"))).clicked() {
                acts.push(Action::ImportDataFile { path: path.to_string(), again: true, changes: None, take_over: false });
                keep = false;
            }
            if rereadable && s.import_state(&other).is_none() && ui.button(tl!("Read the changes in")).clicked() {
                acts.push(Action::ImportDataFile { path: path.to_string(), again: false, changes: Some("read"), take_over: false });
                keep = false;
            }
            if s.import_state(&other).is_some_and(|st| !st.live && !st.removing) && ui.button(tl!("Continue importing")).clicked() {
                acts.push(Action::Run("account.resumeImport", json!({"account": other})));
                keep = false;
            }
            if widgets::accent_button(ui, Icon::Inbox, "Go to it", t, true).clicked() {
                if let Some(inbox) = inbox_of(s, &other) {
                    acts.push(Action::Run("folder.open", json!({"account": other, "folder": inbox})));
                }
                keep = false;
            }
        });
        return keep;
    }
    // The copy another data folder has: taken over (as it was, or with what's new read in), or
    // another copy of the file made.
    if !held.is_empty() {
        let mut keep = true;
        ui.add_space(6.0);
        button_row(ui, |ui| {
            if ui.button(tl!("Cancel")).clicked() {
                keep = false;
            }
            if ui.add_enabled(can_import, egui::Button::new(tl!("Import another copy"))).clicked() {
                acts.push(Action::ImportDataFile { path: path.to_string(), again: true, changes: None, take_over: false });
                keep = false;
            }
            if changed && rereadable && ui.add_enabled(can_import, egui::Button::new(tl!("Take it over and read the changes in"))).clicked() {
                acts.push(Action::ImportDataFile { path: path.to_string(), again: false, changes: Some("read"), take_over: true });
                keep = false;
            }
            if ui.add_enabled(can_import, egui::Button::new(tl!("Take it over"))).clicked() {
                acts.push(Action::ImportDataFile { path: path.to_string(), again: false, changes: changed.then_some("keep"), take_over: true });
                keep = false;
            }
        });
        return keep;
    }
    // The copy from before, the file changed since: opened as it was, or what's new read in.
    if copy && changed {
        let mut keep = true;
        ui.add_space(6.0);
        button_row(ui, |ui| {
            if ui.button(tl!("Cancel")).clicked() {
                keep = false;
            }
            if ui.button(tl!("Open as it was")).clicked() {
                acts.push(Action::ImportDataFile { path: path.to_string(), again: false, changes: Some("keep"), take_over: false });
                keep = false;
            }
            if rereadable && ui.add_enabled(can_import, egui::Button::new(tl!("Read the changes in"))).clicked() {
                acts.push(Action::ImportDataFile { path: path.to_string(), again: false, changes: Some("read"), take_over: false });
                keep = false;
            }
        });
        return keep;
    }
    let (ok, cancel) = buttons(ui, t, if copy { "Open" } else { "Import" }, can_import);
    if ok {
        acts.push(Action::ImportDataFile { path: path.to_string(), again: false, changes: None, take_over: false });
        return false;
    }
    !cancel
}

/// What an import came to: messages and folders, the contacts, calendar, notes and tasks, the
/// time it took, and what it couldn't bring in, a line for each reason.
fn import_done(ui: &mut Ui, account: &str, report: &ImportReport, s: &Session, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    ui.set_width(440.0);
    let file = if report.file.is_empty() {
        s.account(account).map(|a| a.config.email.clone()).unwrap_or_default()
    } else {
        file_name(&report.file).to_string()
    };
    title(ui, t, &tf!("Imported {file}", file = file));
    say(
        ui,
        tf!(
            "{messages} messages in {folders} folders ({time})",
            messages = report.messages,
            folders = report.folders,
            time = duration(report.elapsed_ms)
        ),
        t.text,
    );
    if report.contacts > 0 {
        say(ui, tf!("Contacts: {n}", n = report.contacts), t.dim);
    }
    if report.events > 0 {
        say(ui, tf!("Calendar items: {n}", n = report.events), t.dim);
    }
    if report.notes > 0 {
        say(ui, tf!("Notes: {n}", n = report.notes), t.dim);
    }
    if report.tasks > 0 {
        say(ui, tf!("Tasks: {n}", n = report.tasks), t.dim);
    }
    if !report.skips.is_empty() {
        ui.add_space(8.0);
        ui.label(RichText::new(tl!("Not brought in")).strong().color(t.text));
        for skip in &report.skips {
            say(ui, skip_line(skip.reason, skip.count), t.dim);
        }
    }
    ui.add_space(10.0);
    let mut keep = true;
    button_row(ui, |ui| {
        if ui.button(tl!("Close")).clicked() {
            keep = false;
        }
        if let Some(inbox) = inbox_of(s, account)
            && widgets::accent_button(ui, Icon::Inbox, "Go to Inbox", t, true).clicked()
        {
            acts.push(Action::Run("folder.open", json!({"account": account, "folder": inbox})));
            keep = false;
        }
    });
    keep
}
