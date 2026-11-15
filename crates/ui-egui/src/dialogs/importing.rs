//! Dialogs: a data file's import. What a chosen file holds and where its copy goes (an Outlook
//! file's beside it, or the copy made there before, and whether the file changed since), before
//! it's imported; stopping an import; removing an account while its file is imported; and what
//! an import came to, every loss by its reason.

use mailcraft_engine::Changes;
use mailcraft_engine::model::file_name;
use mailcraft_engine::model::job::{DataFileFacts, ImportReport, SkipReason};

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

/// What the data file dialog offers, by what was found of the file: the first that applies.
enum Offer {
    /// Open as `account` here already: gone to, gone on with, what's new read in (when it
    /// `changed` since its copy was read and that copy is `rereadable`), or another copy made.
    OpenAs { account: String, changed: bool, rereadable: bool },
    /// MailCraft's copy beside it is an account in another data folder: taken over (as it was,
    /// or with what's new read in), or another copy made.
    HeldElsewhere { changed: bool, rereadable: bool },
    /// The copy from before, the file changed since: opened as it was, or what's new read in.
    ChangedCopy { rereadable: bool },
    /// Imported, or the copy from before (`copy`) opened.
    Plain { copy: bool },
}

impl Offer {
    fn of(s: &Session, path: &str) -> Offer {
        let facts = s.data_file_facts(path).filter(|f| f.error.is_none());
        if let Some(account) = s.data_file_open_as(path) {
            // Its copy is the account's: changed since that was read, by the file's size and
            // time then. What's new can be read in when the copy notes what came in from the
            // file before, so it's told from what was deleted here.
            let changed =
                facts.zip(s.data_source(&account)).is_some_and(|(f, r)| f.is_pst() && r.size > 0 && (f.size, f.modified) != (r.size, r.modified));
            let rereadable = changed && s.can_read_changes(&account);
            return Offer::OpenAs { account, changed, rereadable };
        }
        // A copy beside it from before, no account's here: as it was found.
        match facts.filter(|f| !f.copy.is_empty()) {
            None => Offer::Plain { copy: false },
            Some(f) if !f.copy_held_by.is_empty() => Offer::HeldElsewhere { changed: f.copy_changed, rereadable: f.copy_rereadable },
            Some(f) if f.copy_changed => Offer::ChangedCopy { rereadable: f.copy_rereadable },
            Some(_) => Offer::Plain { copy: true },
        }
    }
}

/// That the file changed since MailCraft read it, and when what's new can't be read in, why.
fn changed_notes(ui: &mut Ui, t: &Tokens, rereadable: bool) {
    say(ui, tl!("The file has changed since MailCraft read it.").to_string(), t.warn);
    if !rereadable {
        say(ui, tl!("MailCraft has no note of what came in from this file before, so its changes can't be read in.").to_string(), t.dim);
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
    let can_import = match facts {
        None => {
            // Not looked at (or forgotten since): asked for, once.
            if !s.inspecting(path) {
                acts.push(Action::Run("account.inspectDataFile", json!({"path": path})));
            }
            progress::bar(ui, t, None, vec2(ui.available_width(), 4.0));
            false
        }
        Some(f) if f.error.is_some() => {
            let error = f.error.clone().unwrap_or_default();
            say(ui, tf!("This file couldn't be looked at: {error}", error = error), t.warn);
            true
        }
        Some(f) => facts_notes(ui, t, f),
    };
    let offer = Offer::of(s, path);
    if let Offer::OpenAs { account, changed, rereadable } = &offer {
        let name = s.account(account).map(|a| a.config.name.clone()).unwrap_or_default();
        ui.add_space(6.0);
        say(ui, tf!("This data file is already open as {name}.", name = name), t.warn);
        if *changed {
            changed_notes(ui, t, *rereadable);
        }
    }
    offer_buttons(ui, t, s, path, &offer, can_import, acts)
}

/// What was found of a file looked at: the copy made beside it before (another data folder's,
/// changed since), or where its copy goes; and the room for it. Whether there's room to import.
fn facts_notes(ui: &mut Ui, t: &Tokens, f: &DataFileFacts) -> bool {
    let outlook = f.is_pst();
    if !f.copy.is_empty() {
        say(ui, tf!("MailCraft's copy of this file is beside it, with what was changed in MailCraft: {file}", file = f.copy), t.dim);
        if !f.copy_held_by.is_empty() {
            say(ui, tf!("It's an account in another MailCraft data folder: {folder}", folder = f.copy_held_by), t.warn);
            say(
                ui,
                tl!("Open it there, or take it over here: it's then this data folder's, and its account there should be removed.").to_string(),
                t.dim,
            );
        }
        if f.copy_changed {
            changed_notes(ui, t, f.copy_rereadable);
        }
    } else if f.estimate > 0 && !f.destination.is_empty() {
        let about = widgets::human_bytes(f.estimate);
        let words = if outlook && f.elsewhere {
            tf!("This file's folder can't be written: MailCraft's copy, about {size}, goes in {folder}.", size = about, folder = f.destination)
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
        let words = if outlook && !f.elsewhere && !f.copy_changed {
            tl!("There isn't room for the copy beside the file. Free up space on that drive.")
        } else {
            tl!("There isn't room for the copy. Free up space, or move MailCraft's data folder (File › Options).")
        };
        say(ui, words.to_string(), t.danger);
    }
    !short
}

/// A button of the data file dialog, clicked (`enabled`: it can be).
fn offered(ui: &mut Ui, words: &str, enabled: bool) -> bool {
    ui.add_enabled(enabled, egui::Button::new(words)).clicked()
}

/// The dialog's buttons for `offer` (`can_import`: there's room for a new copy); whether it stays.
fn offer_buttons(ui: &mut Ui, t: &Tokens, s: &Session, path: &str, offer: &Offer, can_import: bool, acts: &mut Vec<Action>) -> bool {
    let import =
        |again: bool, changes: Option<Changes>, take_over: bool| Action::ImportDataFile { path: path.to_string(), again, changes, take_over };
    if let Offer::Plain { copy } = offer {
        let (ok, cancel) = buttons(ui, t, if *copy { "Open" } else { "Import" }, can_import);
        if ok {
            acts.push(import(false, None, false));
            return false;
        }
        return !cancel;
    }
    // The button clicked closes the dialog, doing what it says (Cancel and Go to it with no
    // Inbox: nothing).
    let mut picked: Option<Option<Action>> = None;
    ui.add_space(6.0);
    button_row(ui, |ui| {
        if ui.button(tl!("Cancel")).clicked() {
            picked = Some(None);
        }
        match offer {
            Offer::OpenAs { account, rereadable, .. } => {
                let state = s.import_state(account);
                if offered(ui, tl!("Import another copy"), can_import) {
                    picked = Some(Some(import(true, None, false)));
                }
                if *rereadable && state.is_none() && offered(ui, tl!("Read the changes in"), true) {
                    picked = Some(Some(import(false, Some(Changes::Read), false)));
                }
                if state.is_some_and(|st| !st.live && !st.removing) && offered(ui, tl!("Continue importing"), true) {
                    picked = Some(Some(Action::Run("account.resumeImport", json!({"account": account}))));
                }
                if widgets::accent_button(ui, Icon::Inbox, "Go to it", t, true).clicked() {
                    picked = Some(inbox_of(s, account).map(|inbox| Action::Run("folder.open", json!({"account": account, "folder": inbox}))));
                }
            }
            Offer::HeldElsewhere { changed, rereadable } => {
                if offered(ui, tl!("Import another copy"), can_import) {
                    picked = Some(Some(import(true, None, false)));
                }
                if *changed && *rereadable && offered(ui, tl!("Take it over and read the changes in"), can_import) {
                    picked = Some(Some(import(false, Some(Changes::Read), true)));
                }
                if offered(ui, tl!("Take it over"), can_import) {
                    picked = Some(Some(import(false, changed.then_some(Changes::Keep), true)));
                }
            }
            Offer::ChangedCopy { rereadable } => {
                if offered(ui, tl!("Open as it was"), true) {
                    picked = Some(Some(import(false, Some(Changes::Keep), false)));
                }
                if *rereadable && offered(ui, tl!("Read the changes in"), can_import) {
                    picked = Some(Some(import(false, Some(Changes::Read), false)));
                }
            }
            Offer::Plain { .. } => {}
        }
    });
    match picked {
        None => true,
        Some(act) => {
            acts.extend(act);
            false
        }
    }
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
