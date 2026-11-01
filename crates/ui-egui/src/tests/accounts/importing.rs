//! UI tests: a data file's import. Its bar and time left in the status bar from the first
//! progress, its details, its line in the folder pane, the file looked at before it's imported,
//! closing while it runs, and what it came to; every state and question drawn in light, dark and
//! right to left.

use mailcraft_engine::model::Job;
use mailcraft_engine::model::job::{Blocked, DataFileFacts, ImportPhase, ImportProgress, ImportReport, SkipReason, StopHow, Work};

use super::*;
use crate::Action;

const PATH: &str = "C:/Mail/Archive.pst";

/// A data file imported into a new account (nothing reads it: the tests say how it goes).
fn import(a: &mut MailApp) -> String {
    let id = a.session.run("account.importPst", &json!({"path": PATH})).unwrap()["account"].as_str().unwrap().to_string();
    a.session.take_jobs();
    id
}

fn progress(phase: ImportPhase, done: u64, total: u64, run_ms: u64) -> ImportProgress {
    ImportProgress {
        phase,
        work: Work { done, total, bytes_done: done * 1000, bytes_total: total * 1000 },
        folders_done: 1,
        folders_total: 4,
        folder: "INBOX".into(),
        imported: done,
        skipped: 2,
        run_ms,
        elapsed_ms: run_ms,
        ..Default::default()
    }
}

fn going(a: &mut MailApp, id: &str, p: ImportProgress) {
    a.session.apply(MailEvent::ImportProgress { account: id.to_string(), progress: p });
}

/// A report with a loss of every kind.
fn report(messages: u64) -> ImportReport {
    let mut r = ImportReport {
        file: PATH.into(),
        messages,
        folders: 4,
        contacts: 3,
        events: 2,
        notes: 1,
        elapsed_ms: 4_000_000,
        runs: 1,
        ..Default::default()
    };
    for (i, reason) in all_reasons().into_iter().enumerate() {
        r.skip(reason, u64::try_from(i).unwrap() + 1);
    }
    r
}

fn all_reasons() -> Vec<SkipReason> {
    use SkipReason::*;
    vec![
        MessageUnreadable,
        MessageNotRebuilt,
        NotWritten,
        ListsUnwritten,
        AttachmentUnreadable,
        AttachmentTooBig,
        AttachmentLink,
        AttachedMessageUnreadable,
        AttachedMessageFilesLeftOut,
        EmbeddedTooDeep,
        RecipientWithoutAddress,
        BodyNotConverted,
        CharsetUnsupported,
        FolderUnreadable,
        FolderTooDeep,
        FolderLeftOut,
        ItemInFolderLeftOut,
        ItemUnreadable,
        ItemNotUsable,
        StoppedEarly,
        Other,
    ]
}

fn ended(id: &str, r: ImportReport) -> MailEvent {
    let messages = usize::try_from(r.messages).unwrap();
    MailEvent::Imported { account: id.to_string(), messages, skipped: 0, done: true, error: None, report: Some(Box::new(r)) }
}

/// The text a frame draws (after one more: a dialog or a popup just opened is measured first).
fn drawn(ctx: &egui::Context, a: &mut MailApp) -> Vec<String> {
    frame(ctx, a, vec![]);
    let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))), ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| a.ui(ui));
    out.textures_delta.clear();
    out.shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::Shape::Text(t) => Some(t.galley.job.text.clone()),
            _ => None,
        })
        .collect()
}

fn has(texts: &[String], what: &str) -> bool {
    texts.iter().any(|t| t.contains(what))
}

/// The window asked to close, as its close button does.
fn close(ctx: &egui::Context, a: &mut MailApp) {
    let input = RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))),
        viewports: std::iter::once((egui::ViewportId::ROOT, egui::ViewportInfo { events: vec![egui::ViewportEvent::Close], ..Default::default() }))
            .collect(),
        ..Default::default()
    };
    let mut out = ctx.run_ui(input, |ui| a.ui(ui));
    out.textures_delta.clear();
}

/// The frames of an import in each state and each of its questions, for the tour (light and
/// dark, every language) and the right-to-left test. Leaves no account behind.
pub(crate) fn tour(a: &mut MailApp, ctx: &egui::Context) {
    let id = import(a);
    frame(ctx, a, vec![]);
    for (p, details) in [
        (progress(ImportPhase::Planning, 0, 0, 500), false),
        (progress(ImportPhase::Importing, 10, 100, 25_000), false),
        (progress(ImportPhase::Importing, 30, 100, 50_000), true),
        (progress(ImportPhase::Importing, 40, 0, 60_000), true),
    ] {
        going(a, &id, p);
        a.ui.import_details = details.then(|| id.clone());
        frame(ctx, a, vec![]);
        frame(ctx, a, vec![]);
    }
    for d in [
        Dialog::StopImport { account: id.clone(), file: "Archive.pst".into() },
        Dialog::RemoveAccount { account: id.clone(), email: "Archive.pst".into() },
        Dialog::ImportOnClose { file: "Archive.pst".into(), permille: Some(425) },
        Dialog::ImportOnClose { file: "Archive.pst".into(), permille: None },
        Dialog::OpenDataFile { path: "D:/Old mail.pst".into() },
        Dialog::OpenDataFile { path: PATH.into() },
        Dialog::ImportDone { account: id.clone(), report: Box::new(report(1234)) },
    ] {
        a.ui.dialog = Some(d);
        frame(ctx, a, vec![]);
    }
    // Looked at: room for it, too little, and a file that couldn't be.
    for (path, facts) in [
        (
            "E:/Fits.pst",
            DataFileFacts {
                size: 41 << 30,
                messages: 34_000,
                folders: 120,
                estimate: 38 << 30,
                need: 40 << 30,
                free: Some(400 << 30),
                destination: "C:/Mail files".into(),
                ..Default::default()
            },
        ),
        (
            "E:/Big.pst",
            DataFileFacts {
                size: 41 << 30,
                messages: 34_000,
                folders: 120,
                estimate: 38 << 30,
                need: 40 << 30,
                free: Some(2 << 30),
                destination: "C:/Mail files".into(),
                ..Default::default()
            },
        ),
        ("E:/Broken.pst", DataFileFacts { error: Some("not an Outlook data file".into()), ..Default::default() }),
    ] {
        a.session.apply(MailEvent::DataFileInspected { path: path.into(), facts: Box::new(facts) });
        a.ui.dialog = Some(Dialog::OpenDataFile { path: path.into() });
        frame(ctx, a, vec![]);
    }
    a.ui.dialog = None;
    // Stopped by a full drive, its details open; then done, with what it came to.
    let mut blocked = progress(ImportPhase::Blocked, 40, 100, 61_000);
    blocked.blocked = Some(Blocked::DiskFull { free: 300 << 20 });
    going(a, &id, blocked);
    a.ui.import_details = Some(id.clone());
    frame(ctx, a, vec![]);
    frame(ctx, a, vec![]);
    a.session.run("account.resumeImport", &json!({"account": id})).unwrap();
    a.session.apply(ended(&id, report(100)));
    frame(ctx, a, vec![]);
    a.ui.dialog = None;
    a.ui.import_details = None;
    a.session.run("account.remove", &json!({"account": id})).unwrap();
    a.session.take_jobs();
    frame(ctx, a, vec![]);
}

/// The import from its first progress: a percentage at once (not "Opening…"), the time left once
/// there's a rate to go by, in the status bar and the account's status; its details on a click;
/// a line under the account in the folder pane instead of "Loading folders…".
#[test]
fn an_import_is_shown_from_its_first_second() {
    let (mut a, dir) = app("import-shown");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let id = import(&mut a);
    let texts = drawn(&ctx, &mut a);
    assert!(has(&texts, "Importing Archive.pst…") && has(&texts, "Counting items…"), "{texts:?}");
    assert!(!has(&texts, "Loading folders") && !has(&texts, "Opening"), "{texts:?}");

    going(&mut a, &id, progress(ImportPhase::Importing, 1, 100, 1_000));
    let texts = drawn(&ctx, &mut a);
    assert!(has(&texts, "Importing Archive.pst… 1%"), "a percentage from the first second: {texts:?}");
    assert!(has(&texts, "Importing INBOX… 1%"), "the folder pane's line: {texts:?}");
    assert!(!has(&texts, "left"), "too soon for the time left: {texts:?}");
    assert_eq!(crate::app::importing::account_status(&a.session, &a.ui.tokens, &id).map(|s| s.1).as_deref(), Some("Importing 1%"));

    going(&mut a, &id, progress(ImportPhase::Importing, 10, 100, 25_000));
    going(&mut a, &id, progress(ImportPhase::Importing, 30, 100, 50_000));
    let texts = drawn(&ctx, &mut a);
    assert!(texts.iter().any(|t| t.starts_with("Importing Archive.pst… 30% – About ") && t.ends_with(" min left")), "{texts:?}");

    // A click on the line under the account: its details, beside the status bar.
    a.act(Action::ImportDetails(id.clone()));
    let texts = drawn(&ctx, &mut a);
    for line in ["Messages: 30 of 100", "Data: 29 KB of 98 KB", "Folder: INBOX (2 of 4)", "Not brought in so far: 2", "Stop importing…"] {
        assert!(has(&texts, line), "{line}: {texts:?}");
    }
    let _ = std::fs::remove_dir_all(dir);
}

/// Closing while a data file is imported says it goes on next time: closing then pauses it (its
/// place kept); with nothing imported, or the import already stopping, nothing is asked.
#[test]
fn closing_while_importing_says_it_goes_on() {
    let (mut a, dir) = app("import-close");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let id = import(&mut a);
    going(&mut a, &id, progress(ImportPhase::Importing, 42, 100, 5_000));
    close(&ctx, &mut a);
    assert!(matches!(&a.ui.dialog, Some(Dialog::ImportOnClose { file, permille: Some(420) }) if file == "Archive.pst"), "{:?}", a.ui.dialog);
    assert!(!a.ui.close_now);
    // Keep importing: put away, nothing stops.
    a.ui.dialog = None;
    frame(&ctx, &mut a, vec![]);
    assert!(a.session.take_jobs().iter().all(|j| !matches!(j, Job::StopImport { .. })));
    // Close MailCraft: the import is paused, and the window closes.
    close(&ctx, &mut a);
    assert!(matches!(a.ui.dialog, Some(Dialog::ImportOnClose { .. })), "asked again");
    a.act(Action::Run("account.stopImport", json!({"all": true, "how": "pause"})));
    a.act(Action::CloseAnyway);
    assert!(a.ui.close_now);
    assert_eq!(a.session.take_jobs(), [Job::StopImport { account: id.clone(), how: StopHow::Pause }]);
    let _ = std::fs::remove_dir_all(dir);

    // Nothing being imported: nothing asked.
    let (mut a, dir) = app("import-close-none");
    frame(&ctx, &mut a, vec![]);
    close(&ctx, &mut a);
    assert!(a.ui.dialog.is_none());
    let _ = std::fs::remove_dir_all(dir);
}

/// An import that finishes shows what it came to, every loss on a line of its own; one stopped
/// doesn't, and its report is there again from the account's menu once done.
#[test]
fn a_finished_import_shows_what_it_came_to() {
    let (mut a, dir) = app("import-done");
    let ctx = egui::Context::default();
    let id = import(&mut a);
    a.session.run("account.stopImport", &json!({"account": id})).unwrap();
    a.session.apply(MailEvent::Folders { account: id.clone(), folders: vec![inbox(&id)] });
    a.session.apply(ended(&id, ImportReport { messages: 5, stopped: Some(StopHow::Cancel), ..Default::default() }));
    frame(&ctx, &mut a, vec![]);
    assert!(a.ui.dialog.is_none(), "stopped: nothing to sum up yet");
    let texts = drawn(&ctx, &mut a);
    assert!(has(&texts, "Import stopped: 0 of 0") || has(&texts, "Import stopped"), "{texts:?}");

    a.session.run("account.resumeImport", &json!({"account": id})).unwrap();
    a.session.apply(ended(&id, report(1234)));
    frame(&ctx, &mut a, vec![]);
    assert!(matches!(&a.ui.dialog, Some(Dialog::ImportDone { account, report }) if *account == id && report.messages == 1234));
    let texts = drawn(&ctx, &mut a);
    assert!(has(&texts, "Imported Archive.pst") && has(&texts, "1234 messages in 4 folders (1 h 6 min)"), "{texts:?}");
    assert!(has(&texts, "Contacts: 3") && has(&texts, "Calendar items: 2") && has(&texts, "Notes: 1"), "{texts:?}");
    for (i, reason) in all_reasons().into_iter().enumerate() {
        let line = crate::dialogs::skip_line(reason, u64::try_from(i).unwrap() + 1);
        assert!(has(&texts, &line), "{reason:?}: {line}");
    }
    let lines: std::collections::BTreeSet<String> = all_reasons().into_iter().map(|r| crate::dialogs::skip_line(r, 7)).collect();
    assert_eq!(lines.len(), all_reasons().len(), "a sentence of its own for each reason");
    // Shown once.
    a.ui.dialog = None;
    frame(&ctx, &mut a, vec![]);
    assert!(a.ui.dialog.is_none());
    let _ = std::fs::remove_dir_all(dir);
}

/// A data file chosen is looked at first: counting, then what it holds and where its copy goes,
/// no Import when there's no room for it, and one already open as an account caught.
#[test]
fn a_data_file_is_looked_at_before_it_is_imported() {
    let (mut a, dir) = app("import-look");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let path = "E:/Big.pst";
    a.ui.dialog = Some(Dialog::OpenDataFile { path: path.into() });
    let texts = drawn(&ctx, &mut a);
    assert!(has(&texts, "Counting items…"), "{texts:?}");
    assert!(a.session.inspecting(path), "asked");
    let facts = DataFileFacts {
        messages: 34_000,
        pim_items: 10,
        folders: 120,
        estimate: 38 << 30,
        need: 40 << 30,
        free: Some(2 << 30),
        destination: "C:/Files".into(),
        ..Default::default()
    };
    a.session.apply(MailEvent::DataFileInspected { path: path.into(), facts: Box::new(facts) });
    let texts = drawn(&ctx, &mut a);
    assert!(has(&texts, "34010 items in 120 folders") && has(&texts, "about 38.0 GB, in C:/Files") && has(&texts, "Free there: 2.0 GB"), "{texts:?}");
    assert!(has(&texts, "There isn't room for the copy"), "{texts:?}");

    // Imported: the dialog goes, the account comes, and the status bar takes it from there.
    a.act(Action::ImportDataFile { path: PATH.into(), again: false });
    assert!(a.session.imports().any(|(_, st)| st.path == PATH));
    a.session.take_jobs();
    // The same file again: caught, with another copy only when asked for.
    a.ui.dialog = Some(Dialog::OpenDataFile { path: "c:/mail/ARCHIVE.pst".into() });
    let texts = drawn(&ctx, &mut a);
    assert!(has(&texts, "This data file is already open as Archive."), "{texts:?}");
    let before = a.session.accounts.len();
    a.act(Action::ImportDataFile { path: "c:/mail/ARCHIVE.pst".into(), again: false });
    assert_eq!(a.session.accounts.len(), before, "refused");
    a.act(Action::ImportDataFile { path: "c:/mail/ARCHIVE.pst".into(), again: true });
    assert_eq!(a.session.accounts.len(), before + 1);
    let _ = std::fs::remove_dir_all(dir);
}

/// Services whose file picker hands over one data file.
struct Picks(&'static str);

impl crate::Services for Picks {
    fn clock(&self) -> (i64, i32) {
        (NOW, 0)
    }
    fn pick_files(&mut self) -> Vec<std::path::PathBuf> {
        Vec::new()
    }
    fn save_path(&mut self, _: &str) -> Option<std::path::PathBuf> {
        None
    }
    fn open_path(&mut self, _: &std::path::Path) -> Result<(), String> {
        Ok(())
    }
    fn notify(&mut self, _: &str, _: &str) {}
    fn pick_data_file(&mut self) -> Option<std::path::PathBuf> {
        Some(self.0.into())
    }
}

/// The data files asked to be looked at since the last frame.
fn inspections(a: &mut MailApp) -> Vec<String> {
    a.session
        .take_jobs()
        .into_iter()
        .filter_map(|j| match j {
            Job::InspectDataFile { path } => Some(path),
            _ => None,
        })
        .collect()
}

fn facts(free: u64) -> DataFileFacts {
    DataFileFacts {
        messages: 34_000,
        folders: 120,
        estimate: 38 << 30,
        need: 40 << 30,
        free: Some(free),
        destination: "C:/Files".into(),
        ..Default::default()
    }
}

/// A file picked again is looked at afresh: room freed since a "no room" answer lets it be
/// imported without restarting MailCraft (the first answer was kept for the session).
#[test]
fn a_data_file_picked_again_is_looked_at_again() {
    let (mut a, dir) = app("import-again");
    a.services = Box::new(Picks("E:/Big.pst"));
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.act(Action::OpenDataFile);
    assert_eq!(inspections(&mut a), ["E:/Big.pst"]);
    a.session.apply(MailEvent::DataFileInspected { path: "E:/Big.pst".into(), facts: Box::new(facts(2 << 30)) });
    let texts = drawn(&ctx, &mut a);
    assert!(has(&texts, "There isn't room for the copy"), "{texts:?}");

    // Room freed, the same file picked: asked again, and the new answer shown.
    a.ui.dialog = None;
    a.act(Action::OpenDataFile);
    assert_eq!(inspections(&mut a), ["E:/Big.pst"], "looked at again");
    assert!(has(&drawn(&ctx, &mut a), "Counting items…"), "the old answer isn't shown meanwhile");
    a.session.apply(MailEvent::DataFileInspected { path: "E:/Big.pst".into(), facts: Box::new(facts(400 << 30)) });
    let texts = drawn(&ctx, &mut a);
    assert!(has(&texts, "Free there: 400.0 GB") && !has(&texts, "There isn't room"), "{texts:?}");
    let _ = std::fs::remove_dir_all(dir);
}

/// A path with spaces at its ends is looked at by the path trimmed, and the dialog finds the
/// answer by it (it asked again on every frame, and stayed on "Counting items…").
#[test]
fn a_data_file_path_with_spaces_at_its_ends_is_answered() {
    let (mut a, dir) = app("import-trim");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.ui.dialog = Some(Dialog::OpenDataFile { path: " E:/Big.pst ".into() });
    drawn(&ctx, &mut a);
    assert!(a.session.inspecting(" E:/Big.pst ") && a.session.inspecting("E:/Big.pst"), "asked");
    a.session.apply(MailEvent::DataFileInspected { path: "E:/Big.pst".into(), facts: Box::new(facts(400 << 30)) });
    let texts = drawn(&ctx, &mut a);
    assert!(has(&texts, "34000 items in 120 folders") && !has(&texts, "Counting items"), "{texts:?}");
    assert!(a.session.data_file_open_as(" E:/Big.pst ").is_none());
    let _ = std::fs::remove_dir_all(dir);
}

/// An import stopped by something (a full drive) is paused, one the user stopped is stopped:
/// said the same in the status bar, the folder pane and the account's status.
#[test]
fn a_paused_import_and_a_stopped_one_are_told_apart() {
    let (mut a, dir) = app("import-paused");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let id = import(&mut a);
    let mut blocked = progress(ImportPhase::Blocked, 40, 100, 61_000);
    blocked.blocked = Some(Blocked::DiskFull { free: 300 << 20 });
    going(&mut a, &id, blocked);
    let status = |a: &MailApp| crate::app::importing::account_status(&a.session, &a.ui.tokens, &id).map(|s| s.1);
    let texts = drawn(&ctx, &mut a);
    assert!(has(&texts, "Import paused: Archive.pst") && has(&texts, "Import paused: 40 of 100"), "{texts:?}");
    assert!(!has(&texts, "Import stopped"), "{texts:?}");
    assert_eq!(status(&a).as_deref(), Some("Import paused"));

    // Gone on with, then stopped by the user.
    a.session.run("account.resumeImport", &json!({"account": id})).unwrap();
    a.session.run("account.stopImport", &json!({"account": id})).unwrap();
    a.session.take_jobs();
    going(&mut a, &id, progress(ImportPhase::Importing, 50, 100, 70_000));
    a.session.apply(MailEvent::Imported {
        account: id.clone(),
        messages: 50,
        skipped: 0,
        done: true,
        error: None,
        report: Some(Box::new(ImportReport { messages: 50, stopped: Some(StopHow::Cancel), ..Default::default() })),
    });
    let st = a.session.import_state(&id).unwrap();
    assert!(!st.live && st.progress.blocked.is_none(), "{:?}", st.progress);
    let texts = drawn(&ctx, &mut a);
    assert!(has(&texts, "Import stopped: Archive.pst") && has(&texts, "Import stopped: "), "{texts:?}");
    assert!(!has(&texts, "Import paused"), "{texts:?}");
    assert_eq!(status(&a).as_deref(), Some("Import stopped"));
    let _ = std::fs::remove_dir_all(dir);
}

/// Every state and question of an import drawn in Arabic and Hebrew, light and dark (mirrored
/// windows): nothing panics, and the import's words are in the language.
#[test]
fn an_import_is_drawn_right_to_left() {
    for lang in ["he", "ar"] {
        let (mut a, dir) = app(&format!("import-rtl-{lang}"));
        a.ui.language = lang.to_string();
        crate::i18n::set_current(crate::i18n::Lang::from_code(lang).unwrap());
        let ctx = egui::Context::default();
        for dark in [Some(false), Some(true)] {
            a.ui.dark = dark;
            tour(&mut a, &ctx);
        }
        let id = import(&mut a);
        going(&mut a, &id, progress(ImportPhase::Importing, 30, 100, 5_000));
        let texts = drawn(&ctx, &mut a);
        let words = crate::i18n::t("Importing {file}… {percent}%");
        assert_ne!(words, "Importing {file}… {percent}%", "{lang}: translated");
        assert!(!has(&texts, "Importing Archive.pst"), "{lang}: {texts:?}");
        crate::i18n::set_current(crate::i18n::Lang::EN);
        let _ = std::fs::remove_dir_all(dir);
    }
}

fn inbox(account: &str) -> mailcraft_engine::model::Folder {
    mailcraft_engine::model::Folder {
        account: account.into(),
        path: "INBOX".into(),
        role: mailcraft_engine::model::FolderRole::Inbox,
        selectable: true,
        ..Default::default()
    }
}
