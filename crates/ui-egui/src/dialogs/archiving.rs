//! Dialogs: archiving old mail to a data file on this computer (issue #179), File › Info ›
//! Mailbox settings › Clean up old items: a folder and those in it, a day, and the file. And
//! AutoArchive's (issue #180): its settings, a folder's own (in its Properties), and its
//! question when it's due.

use mailcraft_engine::model::dates;
use mailcraft_engine::{AgeUnit, ArchiveRule};

use super::*;

/// Old mail about to be archived.
#[derive(Clone, Debug, Default)]
pub struct ArchiveForm {
    pub account: String,
    /// The folder whose mail, and its folders', is archived; none: every folder.
    pub folder: Option<String>,
    /// Mail received before this day is archived (`YYYY-MM-DD`, or what was typed).
    pub before: String,
    /// The data file it goes to.
    pub path: String,
    /// The order dates are typed and shown in.
    pub dates: dates::DateOrder,
    pub error: Option<String>,
}

impl ArchiveForm {
    /// For `account`'s mail, all of its folders, received before six months ago.
    pub fn new(s: &Session, account: &str) -> ArchiveForm {
        let (y, m, d) = dates::civil_from_days(dates::today(&s.clock));
        let month = y * 12 + i64::from(m) - 1 - 6;
        let back = dates::days_from_civil(month.div_euclid(12), u32::try_from(month.rem_euclid(12) + 1).unwrap_or(1), d.min(28));
        ArchiveForm { account: account.to_string(), before: dates::day_string(back), dates: s.clock.style.order, ..Default::default() }
    }

    /// As `dataFile.archive` takes it: the day as the instant it starts here.
    pub(crate) fn params(&self, s: &Session) -> Result<serde_json::Value, String> {
        let day = crate::pim_dialogs::pattern_day(&self.before).ok_or_else(|| tl!("That's not a date.").to_string())?;
        let mut p = json!({ "account": self.account, "path": self.path.trim(), "before": s.clock.midnight(day) });
        if let Some(f) = &self.folder {
            p["folder"] = json!(f);
        }
        Ok(p)
    }
}

/// A folder's age and what's done with its old mail, being edited.
#[derive(Clone, Debug, Default)]
pub struct AgeForm {
    pub older_than: u32,
    pub unit: AgeUnit,
    pub delete: bool,
    pub path: String,
}

impl AgeForm {
    /// `rule`, its file `default_path` when it has none.
    pub fn of(rule: &ArchiveRule, default_path: &str) -> AgeForm {
        let path = if rule.path.trim().is_empty() { default_path.to_string() } else { rule.path.clone() };
        AgeForm { older_than: rule.older_than, unit: rule.unit, delete: rule.delete, path }
    }

    fn put(&self, p: &mut serde_json::Value) {
        p["olderThan"] = json!(self.older_than);
        p["unit"] = json!(self.unit.code());
        p["delete"] = json!(self.delete);
        p["path"] = json!(self.path.trim());
    }
}

/// AutoArchive's settings being edited.
#[derive(Clone, Debug, Default)]
pub struct AutoArchiveForm {
    pub on: bool,
    pub every: u32,
    pub prompt: bool,
    pub show: bool,
    pub rule: AgeForm,
    pub error: Option<String>,
}

impl AutoArchiveForm {
    /// The settings as they are; `default_path`, the file to suggest when none is chosen.
    pub fn new(s: &Session, default_path: &str) -> AutoArchiveForm {
        let a = &s.prefs.auto_archive;
        AutoArchiveForm {
            on: a.every_days > 0,
            every: if a.every_days > 0 { a.every_days } else { 14 },
            prompt: a.prompt,
            show: a.show,
            rule: AgeForm::of(&a.rule, default_path),
            error: None,
        }
    }

    /// As `autoArchive.set` takes it.
    pub(crate) fn params(&self) -> serde_json::Value {
        let mut p = json!({ "everyDays": if self.on { self.every } else { 0 }, "prompt": self.prompt, "show": self.show });
        self.rule.put(&mut p);
        p
    }
}

/// What a folder's AutoArchive does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FolderMode {
    Off,
    #[default]
    Default,
    Own,
}

/// A folder's own AutoArchive setting being edited (its Properties).
#[derive(Clone, Debug, Default)]
pub struct FolderArchiveForm {
    pub account: String,
    pub folder: String,
    pub mode: FolderMode,
    pub rule: AgeForm,
}

impl FolderArchiveForm {
    /// `folder` of `account`'s, as it's set; none where AutoArchive doesn't take the account's
    /// mail (kept online only, or an Outlook data file's).
    pub fn new(s: &Session, account: &str, folder: &str, default_path: &str) -> Option<FolderArchiveForm> {
        if !s.auto_archives(account) {
            return None;
        }
        let a = &s.prefs.auto_archive;
        let (mode, rule) = match a.folder(account, folder) {
            None => (FolderMode::Default, &a.rule),
            Some(None) => (FolderMode::Off, &a.rule),
            Some(Some(own)) => (FolderMode::Own, own),
        };
        Some(FolderArchiveForm { account: account.to_string(), folder: folder.to_string(), mode, rule: AgeForm::of(rule, default_path) })
    }

    /// As `autoArchive.folder` takes it.
    pub(crate) fn params(&self) -> serde_json::Value {
        let mode = match self.mode {
            FolderMode::Off => "off",
            FolderMode::Default => "default",
            FolderMode::Own => "own",
        };
        let mut p = json!({ "account": self.account, "folder": self.folder, "archive": mode });
        if self.mode == FolderMode::Own {
            self.rule.put(&mut p);
        }
        p
    }
}

pub(super) fn draw(ui: &mut Ui, d: &mut Dialog, s: &Session, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    match d {
        Dialog::Archive(f) => archive(ui, f, s, t, acts),
        Dialog::AutoArchive(f) => auto_archive(ui, f, t, acts),
        Dialog::AutoArchiveDue => auto_archive_due(ui, s, t, acts),
        _ => true,
    }
}

/// An age and what's done with old mail: the default settings, or a folder's own.
pub(super) fn rule_fields(ui: &mut Ui, r: &mut AgeForm, id: &str, acts: &mut Vec<Action>) {
    let unit = |u: AgeUnit| match u {
        AgeUnit::Days => crate::i18n::tc("interval", "days"),
        AgeUnit::Weeks => crate::i18n::tc("interval", "weeks"),
        AgeUnit::Months => crate::i18n::tc("interval", "months"),
    };
    ui.horizontal(|ui| {
        ui.label(tl!("Archive mail older than"));
        ui.add(egui::DragValue::new(&mut r.older_than).range(1..=999));
        egui::ComboBox::from_id_salt(("archive-unit", id)).selected_text(unit(r.unit)).show_ui(ui, |ui| {
            for u in AgeUnit::ALL {
                ui.selectable_value(&mut r.unit, u, unit(u));
            }
        });
    });
    ui.radio_value(&mut r.delete, false, tl!("Move old mail to this file"));
    ui.add_enabled_ui(!r.delete, |ui| {
        ui.horizontal(|ui| {
            ui.add_space(22.0);
            let width = (ui.available_width() - 100.0).max(120.0);
            ui.add_sized([width, 26.0], egui::TextEdit::singleline(&mut r.path).hint_text("archive.pst"));
            if ui.button(tl!("Browse…")).clicked() {
                acts.push(Action::PickArchiveFile);
            }
        });
    });
    ui.radio_value(&mut r.delete, true, tl!("Delete old mail for good"));
}

/// File › Options › Data files › AutoArchive settings.
fn auto_archive(ui: &mut Ui, f: &mut AutoArchiveForm, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    ui.set_width(500.0);
    title(ui, t, "AutoArchive");
    ui.add(
        egui::Label::new(
            RichText::new(tl!(
                "Every so many days, each folder's old mail is archived to a data file on this computer, or deleted, by the folder's own settings (in its Properties) or these."
            ))
            .color(t.dim),
        )
        .wrap(),
    );
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.checkbox(&mut f.on, tl!("Run AutoArchive every"));
        ui.add_enabled(f.on, egui::DragValue::new(&mut f.every).range(1..=60));
        ui.label(crate::i18n::tc("interval", "days"));
    });
    ui.add_enabled_ui(f.on, |ui| {
        ui.checkbox(&mut f.prompt, tl!("Ask before AutoArchive runs"));
    });
    ui.checkbox(&mut f.show, tl!("Show the archive file in the folder list"));
    ui.add_space(8.0);
    ui.label(RichText::new(tl!("For folders without settings of their own")).strong());
    rule_fields(ui, &mut f.rule, "auto", acts);
    ui.add_space(6.0);
    let all =
        ui.button(tl!("Apply these settings to all folders now")).on_hover_text(tl!("Folders with settings of their own follow these instead."));
    if all.clicked() {
        acts.push(Action::Run("autoArchive.applyToAll", json!({})));
    }
    if let Some(e) = &f.error {
        ui.add_space(4.0);
        ui.add(egui::Label::new(RichText::new(e.as_str()).color(t.danger).small()).wrap());
    }
    let ready = f.rule.delete || !f.rule.path.trim().is_empty();
    let (ok, cancel) = buttons(ui, t, "Save", ready);
    if ok {
        f.error = None;
        acts.push(Action::Submit("autoArchive.set", f.params()));
    }
    !cancel
}

/// AutoArchive is due, and asks first.
fn auto_archive_due(ui: &mut Ui, s: &Session, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    ui.set_width(420.0);
    title(ui, t, "AutoArchive");
    let days = s.prefs.auto_archive.every_days;
    let asked = tf!("It's been {days} days: archive old mail now, as AutoArchive is set to?", days = days);
    ui.add(egui::Label::new(RichText::new(asked).color(t.dim)).wrap());
    let (ok, cancel) = buttons(ui, t, "Archive now", true);
    if ok {
        acts.push(Action::Run("autoArchive.run", json!({})));
        return false;
    }
    if cancel {
        acts.push(Action::Run("autoArchive.later", json!({})));
    }
    !cancel
}

fn archive(ui: &mut Ui, f: &mut ArchiveForm, s: &Session, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    ui.set_width(480.0);
    title(ui, t, "Clean up old items");
    ui.add(
        egui::Label::new(
            RichText::new(tl!(
                "Old mail goes into an Outlook data file on this computer, into folders of the same names, and is then deleted from the mailbox, which gets its space back. The file shows in the folder list, where its mail can be read, searched and moved back."
            ))
            .color(t.dim),
        )
        .wrap(),
    );
    ui.add_space(8.0);
    ui.label(RichText::new(tl!("Archive this folder and the folders in it")).color(t.text));
    let folders: Vec<&Folder> =
        s.account(&f.account).map(|a| a.folders.iter().filter(|x| x.selectable && !x.path.starts_with('\0')).collect()).unwrap_or_default();
    let shown = |path: Option<&str>| match path.and_then(|p| folders.iter().find(|x| x.path == p)) {
        Some(x) => crate::i18n::folder(x),
        None => tl!("All folders").to_string(),
    };
    egui::ComboBox::from_id_salt("archive-folder").width(ui.available_width()).selected_text(shown(f.folder.as_deref())).show_ui(ui, |ui| {
        ui.selectable_value(&mut f.folder, None, tl!("All folders"));
        for x in &folders {
            let indent = "   ".repeat(x.depth());
            ui.selectable_value(&mut f.folder, Some(x.path.clone()), format!("{indent}{}", crate::i18n::folder(x)));
        }
    });
    ui.add_space(6.0);
    ui.label(RichText::new(tl!("Archive mail received before")).color(t.text));
    ui.horizontal(|ui| {
        crate::pim_dialogs::date_field(ui, t, "archive-before", &mut f.before, f.dates, None);
    });
    ui.add_space(6.0);
    ui.label(RichText::new(tl!("Archive file")).color(t.text));
    ui.horizontal(|ui| {
        let browse = tl!("Browse…");
        let width = (ui.available_width() - 100.0).max(120.0);
        ui.add_sized([width, 26.0], egui::TextEdit::singleline(&mut f.path).hint_text("archive.pst"));
        if ui.button(browse).clicked() {
            acts.push(Action::PickArchiveFile);
        }
    });
    if let Some(e) = &f.error {
        ui.add_space(4.0);
        ui.add(egui::Label::new(RichText::new(e.as_str()).color(t.danger).small()).wrap());
    }
    let ready = !f.path.trim().is_empty() && crate::pim_dialogs::pattern_day(&f.before).is_some();
    let (ok, cancel) = buttons(ui, t, "Archive", ready);
    if ok {
        match f.params(s) {
            Ok(p) => {
                f.error = None;
                acts.push(Action::Submit("dataFile.archive", p));
            }
            Err(e) => f.error = Some(e),
        }
    }
    !cancel
}
