//! A data file's import as the window shows it: a segment of the status bar (how far it is, the
//! time left) that opens its details, a line under its account in the folder pane, the account's
//! status and menu, closing while it runs, and what it came to (the questions and the summary
//! are drawn in `dialogs/importing.rs`).

use egui::PopupCloseBehavior;
use mailcraft_engine::ImportState;
use mailcraft_engine::model::FolderRole;
use mailcraft_engine::model::job::{Blocked, ImportPhase};

use crate::*;

/// The time left as it's said: under a minute; minutes, rounded up, below an hour and a half;
/// then hours and minutes, the minutes rounded to five.
pub(crate) fn time_left(s: u64) -> String {
    if s < 60 {
        tl!("Less than a minute left").to_string()
    } else if s < 90 * 60 {
        tf!("About {n} min left", n = s.div_ceil(60))
    } else {
        let minutes = s.saturating_add(150) / 300 * 5;
        tf!("About {h} h {m} min left", h = minutes / 60, m = minutes % 60)
    }
}

/// A time taken (ms): seconds below a minute, minutes below an hour, then hours and minutes.
pub(crate) fn duration(ms: u64) -> String {
    let s = ms / 1000;
    if s < 60 {
        tf!("{n} s", n = s)
    } else if s < 3600 {
        tf!("{n} min", n = s / 60)
    } else {
        tf!("{h} h {m} min", h = s / 3600, m = s % 3600 / 60)
    }
}

/// The import the status bar shows: one being read first, else one waiting on the user.
pub(crate) fn shown(s: &Session) -> Option<(&str, &ImportState)> {
    s.imports().filter(|(_, st)| !st.removing).min_by_key(|(_, st)| !st.live)
}

/// Whether closing now should first say that a data file is being imported: one is, and it
/// hasn't been asked to stop.
pub(crate) fn ask_on_close(s: &Session) -> bool {
    s.imports().any(|(_, st)| st.live && !st.removing && st.stopping.is_none())
}

/// An import waiting on the user is paused when something stopped it (the file gone, the drive
/// full), and stopped when the user did, or MailCraft closed: said so in the status bar, the
/// folder pane and the account's status alike.
fn paused(st: &ImportState) -> bool {
    st.progress.blocked.is_some()
}

/// Whether an import waiting on the user can go on: where its data file is is known (not one
/// found at start whose record was lost).
fn can_go_on(s: &Session, account: &str, st: &ImportState) -> bool {
    !st.path.is_empty() || s.data_source(account).is_some_and(|d| !d.path.is_empty())
}

/// What the status bar says of an import: its words (with the time left, once known), how far
/// it is in thousandths, and what hovering it says.
pub(crate) fn status_words(s: &Session, account: &str, st: &ImportState) -> (String, Option<u16>, String) {
    let p = &st.progress;
    let permille = p.permille();
    let mut text = if !st.live {
        if paused(st) { tf!("Import paused: {file}", file = st.file) } else { tf!("Import stopped: {file}", file = st.file) }
    } else if st.stopping.is_some() || p.phase == ImportPhase::Stopping {
        tf!("Stopping the import of {file}…", file = st.file)
    } else {
        match permille {
            Some(m) if p.phase != ImportPhase::Planning => tf!("Importing {file}… {percent}%", file = st.file, percent = progress::percent(m)),
            _ => tf!("Importing {file}…", file = st.file),
        }
    };
    if let Some(left) = s.import_eta_s(account) {
        text = format!("{text} – {}", time_left(left));
    }
    let hover = if p.work.total > 0 {
        tf!("Messages: {done} of {total}", done = p.work.done, total = p.work.total)
    } else {
        tl!("Counting items…").to_string()
    };
    (text, permille, hover)
}

/// Why an import waits on the user, said in full.
fn blocked_words(b: &Blocked) -> String {
    match b {
        Blocked::SourceMissing => tl!("The data file isn't where it was.").to_string(),
        Blocked::SourceChanged => tl!("The data file changed after the import began.").to_string(),
        Blocked::DiskFull { free } => tf!("There's too little room left on the drive ({free} free).", free = widgets::human_bytes(*free)),
        Blocked::Locked => tl!("Another program is using the data file.").to_string(),
    }
}

/// The status bar's segment for the import shown, and its details while they're open
/// (`details`: whose). Whether an import was shown (the search index's build then isn't).
pub(crate) fn status_segment(ui: &mut Ui, s: &Session, t: &Tokens, details: &mut Option<String>, acts: &mut Vec<Action>) -> bool {
    let Some((account, st)) = shown(s) else {
        *details = None;
        return false;
    };
    let (text, permille, hover) = status_words(s, account, st);
    ui.add_space(6.0);
    let resp = progress::segment(ui, t, &text, permille, &hover);
    if resp.clicked() {
        *details = if details.is_some() { None } else { Some(account.to_string()) };
    }
    let Some(whose) = details.clone() else { return true };
    let Some(st) = s.import_state(&whose).filter(|st| !st.removing) else {
        *details = None;
        return true;
    };
    let mut open = true;
    let before = acts.len();
    egui::Popup::from_response(&resp)
        .id(egui::Id::unique("import-details"))
        .open_bool(&mut open)
        .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
        .align(egui::RectAlign::TOP_START)
        .show(|ui| details_ui(ui, s, t, &whose, st, acts));
    // Closed, or something in it asked for (a question, the account removed): put away.
    if !open || acts.len() > before {
        *details = None;
    }
    true
}

/// The details of `account`'s import: how far in messages, data and folders, the time so far
/// and left, what it couldn't bring in, where its copy is; and what can be done with it.
fn details_ui(ui: &mut Ui, s: &Session, t: &Tokens, account: &str, st: &ImportState, acts: &mut Vec<Action>) {
    ui.set_width(340.0);
    let p = &st.progress;
    let (text, permille, _) = status_words(s, account, st);
    ui.label(RichText::new(text).strong().color(t.text));
    ui.add_space(4.0);
    progress::bar(ui, t, permille, vec2(ui.available_width(), 6.0));
    ui.add_space(6.0);
    let line = |ui: &mut Ui, words: String| {
        ui.add(egui::Label::new(RichText::new(words).color(t.dim).small()).wrap());
    };
    if p.work.total > 0 {
        line(ui, tf!("Messages: {done} of {total}", done = p.work.done, total = p.work.total));
    } else if st.live {
        line(ui, tl!("Counting items…").to_string());
    }
    if p.work.bytes_total > 0 {
        line(ui, tf!("Data: {done} of {total}", done = widgets::human_bytes(p.work.bytes_done), total = widgets::human_bytes(p.work.bytes_total)));
    }
    if st.live && !p.folder.is_empty() {
        let folder = s.folder(account, &p.folder).map_or_else(|| p.folder.clone(), i18n::folder);
        let n = p.folders_done.saturating_add(1).min(p.folders_total.max(1));
        line(ui, tf!("Folder: {folder} ({n} of {total})", folder = folder, n = n, total = p.folders_total));
    }
    if p.elapsed_ms > 0 {
        let so_far = tf!("{time} so far", time = duration(p.elapsed_ms));
        match s.import_eta_s(account) {
            Some(left) => line(ui, format!("{so_far} – {}", time_left(left))),
            None => line(ui, so_far),
        }
    }
    if p.skipped > 0 {
        // Why, on hover, once a run has stopped and said (a run going on only counts them).
        let why: Vec<String> = s
            .data_source(account)
            .and_then(|d| d.report.as_ref())
            .map(|r| r.skips.iter().map(|k| dialogs::skip_line(k.reason, k.count)).collect())
            .unwrap_or_default();
        let words = tf!("Not brought in so far: {n}", n = p.skipped);
        let shown = ui.add(egui::Label::new(RichText::new(words).color(t.dim).small()).wrap());
        if !why.is_empty() {
            shown.on_hover_text(why.join(
                "
",
            ));
        }
    }
    if let Some(b) = &p.blocked {
        ui.add(egui::Label::new(RichText::new(blocked_words(b)).color(t.warn)).wrap());
    }
    if let Some(file) = s.store.as_ref().and_then(|store| store.data_file(account)) {
        line(ui, tf!("Saved in: {path}", path = file.display()));
        if file.exists() && ui.link(RichText::new(tl!("Open file location")).small()).clicked() {
            let dir = file.parent().map_or_else(|| file.clone(), std::path::Path::to_path_buf);
            acts.push(Action::OpenFolder(dir));
        }
    }
    ui.add_space(8.0);
    ui.horizontal_wrapped(|ui| {
        if st.live {
            if st.stopping.is_none() && ui.button(tl!("Stop importing…")).clicked() {
                acts.push(Action::Dialog(Dialog::StopImport { account: account.to_string(), file: st.file.clone() }));
            }
            return;
        }
        // Waiting on the user: go on, keep it as it is, read the file again, or let it go.
        let known = can_go_on(s, account, st);
        if known && ui.button(tl!("Continue importing")).clicked() {
            acts.push(Action::Run("account.resumeImport", json!({"account": account})));
        }
        if ui.button(tl!("Keep what's imported")).clicked() {
            acts.push(Action::Run("account.keepImported", json!({"account": account})));
        }
        if known && ui.button(tl!("Start again")).on_hover_text(tl!("What came in goes, and the data file is read from its start.")).clicked() {
            acts.push(Action::Run("account.restartImport", json!({"account": account})));
        }
        let email = s.account(account).map(|a| a.config.email.clone()).unwrap_or_default();
        if ui.button(RichText::new(tl!("Remove account…")).color(t.danger)).clicked() {
            acts.push(Action::Dialog(Dialog::RemoveAccount { account: account.to_string(), email }));
        }
    });
}

/// The line under an account in the folder pane while its data file is imported (a thin bar
/// and where it is; a click shows the details). Whether there was one.
pub(crate) fn nav_line(s: &Session, t: &Tokens, ui: &mut Ui, acts: &mut Vec<Action>, account: &str) -> bool {
    let Some(st) = s.import_state(account) else { return false };
    let p = &st.progress;
    let text = if st.removing {
        tl!("Removing…").to_string()
    } else if !st.live {
        match (paused(st), p.work.total > 0) {
            (true, true) => tf!("Import paused: {done} of {total}", done = p.work.done, total = p.work.total),
            (true, false) => tl!("Import paused").to_string(),
            (false, true) => tf!("Import stopped: {done} of {total}", done = p.work.done, total = p.work.total),
            (false, false) => tl!("Import stopped").to_string(),
        }
    } else if st.stopping.is_some() || p.phase == ImportPhase::Stopping {
        tl!("Stopping…").to_string()
    } else if p.phase == ImportPhase::Planning {
        tl!("Counting items…").to_string()
    } else {
        let folder = s.folder(account, &p.folder).map_or_else(|| p.folder.clone(), i18n::folder);
        // A .pst is read a few folders at a time, none of them "the" folder: no name then.
        match (p.permille(), folder.is_empty()) {
            (Some(m), false) => tf!("Importing {folder}… {percent}%", folder = folder, percent = progress::percent(m)),
            (Some(m), true) => tf!("Importing {percent}%", percent = progress::percent(m)),
            (None, false) => tf!("Importing {folder}…", folder = folder),
            (None, true) => tl!("Importing…").to_string(),
        }
    };
    ui.horizontal(|ui| {
        ui.add_space(14.0);
        ui.vertical(|ui| {
            if st.live {
                progress::bar(ui, t, p.permille(), vec2((ui.available_width() - 8.0).max(20.0), 2.0));
            }
            let width = (ui.available_width() - 8.0).max(20.0);
            let g = widgets::one_line(ui, &text, egui::TextStyle::Small.resolve(ui.style()), t.dim, width);
            let (r, resp) = ui.allocate_exact_size(vec2(width, g.size().y), Sense::click());
            let m = bidi::Mirror::new(r);
            ui.painter().galley(egui::pos2(m.start(r.min.x, g.size().x), r.min.y), g, t.dim);
            widgets::named(ui, &resp, &text, true, None);
            if resp.on_hover_text(tl!("Show how the import is going")).clicked() {
                acts.push(Action::ImportDetails(account.to_string()));
            }
            // Stopped: going on is one click away.
            if !st.live && !st.removing && can_go_on(s, account, st) && ui.link(RichText::new(tl!("Continue")).small()).clicked() {
                acts.push(Action::Run("account.resumeImport", json!({"account": account})));
            }
        });
    });
    true
}

/// The folders an import hasn't brought in yet, as its plan names them, under its account's
/// folders in the folder pane: faint, in the order they come in, each where it will be. One
/// shows as any other folder once it's in (until then nothing can be done with it).
pub(crate) fn outline(s: &Session, t: &Tokens, ui: &mut Ui, account: &str, have: &[mailcraft_engine::model::Folder]) {
    let Some(st) = s.import_state(account).filter(|st| !st.removing) else { return };
    for f in st.planned.iter().filter(|f| !have.iter().any(|h| h.path == f.path)) {
        let depth = f.path.matches('/').count().saturating_add(1);
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::hover());
        let m = bidi::Mirror::new(rect);
        let x = rect.min.x + 10.0 + depth as f32 * 14.0;
        icons::paint(ui.painter(), m.rect(egui::Rect::from_min_size(egui::pos2(x, rect.center().y - 7.0), vec2(14.0, 14.0))), Icon::Folder, t.faint);
        let width = (rect.max.x - x - 30.0).max(20.0);
        let g = widgets::one_line(ui, &f.name, egui::TextStyle::Body.resolve(ui.style()), t.faint, width);
        let at = egui::pos2(m.start(x + 22.0, g.size().x), rect.center().y - g.size().y / 2.0);
        ui.painter().galley(at, g, t.faint);
        widgets::named(ui, &resp, &format!("{}, {}", f.name, tl!("Not imported yet")), false, None);
        resp.on_hover_text(tl!("Not imported yet"));
    }
}

/// An account's menu, for its import: stop it, go on with it, keep it as it is, or see what an
/// import came to.
pub(crate) fn menu(ui: &mut Ui, s: &Session, account: &str, acts: &mut Vec<Action>) {
    match s.import_state(account) {
        Some(st) if st.removing => {}
        Some(st) if st.live => {
            if st.stopping.is_none() && ui.button(tl!("Stop importing…")).clicked() {
                acts.push(Action::Dialog(Dialog::StopImport { account: account.to_string(), file: st.file.clone() }));
                ui.close();
            }
        }
        Some(st) => {
            // Not when where its data file is isn't known (lost at start): kept as it is, or
            // removed (the menu's own Remove account…).
            if can_go_on(s, account, st) && ui.button(tl!("Continue importing")).clicked() {
                acts.push(Action::Run("account.resumeImport", json!({"account": account})));
                ui.close();
            }
            if ui.button(tl!("Keep what's imported")).clicked() {
                acts.push(Action::Run("account.keepImported", json!({"account": account})));
                ui.close();
            }
        }
        None => {
            if let Some(report) = s.data_source(account).and_then(|d| d.report.as_ref())
                && ui.button(tl!("Import report…")).clicked()
            {
                acts.push(Action::Dialog(Dialog::ImportDone { account: account.to_string(), report: Box::new(report.clone()) }));
                ui.close();
            }
        }
    }
}

/// An account's status while its data file is imported, or its import waits on the user.
pub(crate) fn account_status(s: &Session, t: &Tokens, account: &str) -> Option<(egui::Color32, String)> {
    let st = s.import_state(account)?;
    Some(if st.removing {
        (t.faint, tl!("Removing…").to_string())
    } else if st.live {
        match st.progress.permille().filter(|_| st.progress.phase != ImportPhase::Planning) {
            Some(m) => (t.accent, tf!("Importing {percent}%", percent = progress::percent(m))),
            None => (t.accent, tl!("Importing…").to_string()),
        }
    } else if paused(st) {
        (t.warn, tl!("Import paused").to_string())
    } else {
        (t.faint, tl!("Import stopped").to_string())
    })
}

impl MailApp {
    /// Closing while a data file is imported: say that it goes on the next time MailCraft opens.
    pub(crate) fn ask_import_on_close(&mut self) {
        let (file, permille) =
            shown(&self.session).filter(|(_, st)| st.live).map(|(_, st)| (st.file.clone(), st.progress.permille())).unwrap_or_default();
        self.ui.dialog = Some(Dialog::ImportOnClose { file, permille });
        self.ui.dialog_for = None;
        self.ui.close_asked = true;
    }

    /// An import that has finished: what it came to, once nothing else is being asked.
    pub(crate) fn show_finished_import(&mut self) {
        if self.ui.dialog.is_some() {
            return;
        }
        while let Some(account) = self.session.take_finished_import() {
            if let Some(report) = self.session.data_source(&account).and_then(|d| d.report.clone()) {
                self.ui.dialog = Some(Dialog::ImportDone { account, report: Box::new(report) });
                return;
            }
        }
    }
}

/// The folder `Go to Inbox` opens for an account, when it has one.
pub(crate) fn inbox_of(s: &Session, account: &str) -> Option<String> {
    s.account(account).and_then(|a| a.folder(FolderRole::Inbox)).map(|f| f.path.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_time_left_is_rounded_as_people_say_it() {
        assert_eq!(time_left(0), "Less than a minute left");
        assert_eq!(time_left(59), "Less than a minute left");
        assert_eq!(time_left(60), "About 1 min left");
        assert_eq!(time_left(61), "About 2 min left", "rounded up");
        assert_eq!(time_left(89 * 60 + 59), "About 90 min left");
        assert_eq!(time_left(90 * 60), "About 1 h 30 min left");
        assert_eq!(time_left(2 * 3600 + 7 * 60), "About 2 h 5 min left", "to five minutes");
        assert_eq!(time_left(2 * 3600 + 58 * 60), "About 3 h 0 min left");
        assert!(time_left(u64::MAX).starts_with("About "), "never overflows");
        assert_eq!([duration(0), duration(59_999), duration(60_000), duration(3_725_000)], ["0 s", "59 s", "1 min", "1 h 2 min"]);
    }
}
