//! Dialogs: archiving old mail to a data file on this computer (issue #179), File › Info ›
//! Mailbox settings › Clean up old items: a folder and those in it, a day, and the file.

use mailcraft_engine::model::dates;

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

pub(super) fn draw(ui: &mut Ui, d: &mut Dialog, s: &Session, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    match d {
        Dialog::Archive(f) => archive(ui, f, s, t, acts),
        _ => true,
    }
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
