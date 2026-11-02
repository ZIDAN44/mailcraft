//! UI tests: an account's data file made smaller. The File page's row (what could be given
//! back, Compact now, what it came to), the dialog in each part, the status bar, the idle policy
//! told how long MailCraft was left alone and an idle compaction stopped by the user coming back,
//! and compacting as MailCraft closes; every state drawn in light, dark and right to left.

use std::sync::{Arc, Mutex};

use mailcraft_engine::Compacting;
use mailcraft_engine::model::Job;
use mailcraft_engine::model::job::{CompactMode, CompactPhase, CompactReport, DataFileHealth, Work};

use super::*;
use crate::Action;
use crate::backstage::{Backstage, Page};

/// A backend that keeps the jobs it's given (the tests look at them).
#[derive(Clone, Default)]
struct Jobs(Arc<Mutex<Vec<Job>>>);

impl Jobs {
    fn take(&self) -> Vec<Job> {
        self.0.lock().map(|mut j| std::mem::take(&mut *j)).unwrap_or_default()
    }
}

impl crate::Backend for Jobs {
    fn submit(&mut self, job: Job) {
        if let Ok(mut j) = self.0.lock() {
            j.push(job);
        }
    }
    fn poll(&mut self) -> Vec<MailEvent> {
        Vec::new()
    }
}

/// A window tall enough for the whole File page.
const TALL: egui::Vec2 = egui::vec2(1280.0, 2400.0);

/// The window over a store of data files, with one account kept in one.
fn with_data_file(name: &str) -> (MailApp, Jobs, String, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("mailcraft-ui-compact-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open_packed(dir.join("app"), dir.join("MailCraft Files")).unwrap();
    let mut s = Session::open(Some(store), Clock::new(NOW, 0));
    let id = kept_in_a_data_file(&mut s, "zoe@example.org");
    let jobs = Jobs::default();
    (MailApp::new(s, Box::new(jobs.clone()), Box::new(crate::NullServices(NOW))), jobs, id, dir)
}

/// An account added whose mail is kept in a data file of its own; its id.
fn kept_in_a_data_file(s: &mut Session, email: &str) -> String {
    let id = s.run("account.add", &json!({"email": email, "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    let inbox = mailcraft_engine::model::Folder {
        account: id.clone(),
        path: "INBOX".into(),
        role: mailcraft_engine::model::FolderRole::Inbox,
        selectable: true,
        ..Default::default()
    };
    s.store.as_ref().unwrap().save_folders(&id, std::slice::from_ref(&inbox)).unwrap();
    s.apply(MailEvent::Folders { account: id.clone(), folders: vec![inbox] });
    assert!(s.store.as_ref().unwrap().data_file(&id).is_some());
    s.apply(MailEvent::Status { account: id.clone(), status: SyncStatus::Online { at: NOW } });
    s.take_jobs();
    id
}

fn finish(a: MailApp, dir: std::path::PathBuf) {
    drop(a);
    let _ = std::fs::remove_dir_all(dir);
}

/// A frame at `time` (seconds of the input's clock), with `events`.
fn frame_at(ctx: &egui::Context, a: &mut MailApp, time: f64, events: Vec<Event>) {
    let input = RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))),
        time: Some(time),
        events,
        ..Default::default()
    };
    let mut out = ctx.run_ui(input, |ui| a.ui(ui));
    out.textures_delta.clear();
}

/// The text a frame draws (after one more: a dialog just opened is measured first), in a window
/// tall enough for the whole File page.
fn drawn(ctx: &egui::Context, a: &mut MailApp) -> Vec<String> {
    let input = || RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, TALL)), ..Default::default() };
    let mut out = ctx.run_ui(input(), |ui| a.ui(ui));
    out.textures_delta.clear();
    let mut out = ctx.run_ui(input(), |ui| a.ui(ui));
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

/// 2 GiB of a 4 GiB file to give back: worth compacting, quickly.
fn bloated() -> DataFileHealth {
    DataFileHealth { file_bytes: 4 << 30, live_bytes: 2 << 30, reclaimable: 2 << 30, recompressed: true, ..DataFileHealth::default() }
}

/// 10 GiB of a 40 GiB file: longer than closing waits.
fn big() -> DataFileHealth {
    DataFileHealth { file_bytes: 40 << 30, live_bytes: 30 << 30, reclaimable: 10 << 30, recompressed: true, ..DataFileHealth::default() }
}

fn measured(a: &mut MailApp, id: &str, h: DataFileHealth) {
    a.session.apply(MailEvent::DataFileHealth { account: id.to_string(), health: Some(Box::new(h)), error: None });
}

fn going(a: &mut MailApp, id: &str, phase: CompactPhase, work: Work, exclusive_s: Option<u64>) {
    a.session.apply(MailEvent::Compacting { account: id.to_string(), phase, work, exclusive_s });
}

fn running(mode: CompactMode, phase: CompactPhase, work: Work, exclusive_s: Option<u64>) -> Compacting {
    Compacting { mode, phase, work, exclusive_s }
}

/// The frames of a compaction in each part, and of compacting on close, for the tour (light and
/// dark, every language) and the right-to-left test. Leaves nothing running behind.
pub(crate) fn tour(a: &mut MailApp, ctx: &egui::Context) {
    let id = a.session.accounts[0].config.id.clone();
    let half = Work { done: 40, total: 100, bytes_done: 40 << 20, bytes_total: 100 << 20 };
    for (phase, work, exclusive_s) in [
        (CompactPhase::Measuring, Work::default(), None),
        (CompactPhase::Recompress, half, Some(12)),
        (CompactPhase::Copy, half, None),
        (CompactPhase::Exclusive, Work::default(), Some(12)),
        (CompactPhase::Exclusive, Work::default(), None),
    ] {
        a.session.compaction.running.insert(id.clone(), running(CompactMode::Manual, phase, work, exclusive_s));
        for closing in [false, true] {
            a.ui.dialog = Some(Dialog::Compacting { account: id.clone(), closing, since: Some(0.0) });
            frame(ctx, a, vec![]);
        }
        a.ui.dialog = None;
        a.ui.compact.closing = false;
        frame(ctx, a, vec![]);
    }
    a.session.compaction.running.clear();
    for h in [big(), DataFileHealth { recompressed: false, recompress_gain: 9 << 30, raw_fraction: 0.7, ..big() }] {
        a.session.compaction.health.insert(id.clone(), h);
        a.ui.dialog = Some(Dialog::CompactOnClose { account: id.clone(), seconds: 125 });
        frame(ctx, a, vec![]);
    }
    a.session.compaction.health.clear();
    a.ui.dialog = None;
    a.ui.compact = Default::default();
    frame(ctx, a, vec![]);
    file_page_tour(ctx, a.ui.dark, &a.ui.language);
}

/// The File page's row for an account with a data file, in each state: what could be
/// reclaimed, being compacted, and each kind of report (the tour's own app has no data file).
fn file_page_tour(ctx: &egui::Context, dark: Option<bool>, language: &str) {
    static RUN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = RUN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let (mut a, _jobs, id, dir) = with_data_file(&format!("tour-{n}"));
    a.ui.dark = dark;
    a.ui.language = language.to_string();
    let mut b = Backstage::new(Page::Info);
    b.account = Some(id.clone());
    a.ui.backstage = Some(b);
    measured(&mut a, &id, bloated());
    frame(ctx, &mut a, vec![]);
    a.session.compaction.running.insert(id.clone(), running(CompactMode::Manual, CompactPhase::Recompress, Work::default(), None));
    frame(ctx, &mut a, vec![]);
    a.session.compaction.running.clear();
    let done =
        CompactReport { mode: CompactMode::Manual, before: 4 << 30, after: 2 << 30, compacted: true, elapsed_ms: 9_000, ..CompactReport::default() };
    let reports: [fn(&mut CompactReport); 6] = [
        |_| {},
        |r| r.after = r.before,
        |r| r.reimport = true,
        |r| r.too_long_s = Some(125),
        |r| r.cancelled = true,
        |r| r.error = Some("disk full".into()),
    ];
    for f in reports {
        let mut r = done.clone();
        f(&mut r);
        a.session.compaction.last.insert(id.clone(), r);
        frame(ctx, &mut a, vec![]);
    }
    finish(a, dir);
}

#[test]
fn the_file_page_offers_compacting_and_says_what_it_came_to() {
    let (mut a, jobs, id, dir) = with_data_file("file-page");
    let ctx = egui::Context::default();
    let mut b = Backstage::new(Page::Info);
    b.account = Some(id.clone());
    a.ui.backstage = Some(b);
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    let asked = jobs.take().into_iter().filter(|j| matches!(j, Job::DataFileHealth { account } if *account == id)).count();
    assert_eq!(asked, 1, "measured once a visit");

    // Room to give back: said, and offered.
    measured(&mut a, &id, bloated());
    let texts = drawn(&ctx, &mut a);
    assert!(has(&texts, "2.0 GB can be reclaimed") && has(&texts, "Compact now"), "{texts:?}");

    // Compact now: asked by hand, its dialog over the page, each part in its words.
    a.act(Action::Run("dataFile.compact", json!({ "account": id })));
    a.act(Action::Dialog(Dialog::Compacting { account: id.clone(), closing: false, since: None }));
    assert!(jobs.take().is_empty() && a.session.compaction.running.contains_key(&id));
    frame(&ctx, &mut a, vec![]);
    assert!(jobs.take().iter().any(|j| matches!(j, Job::CompactDataFile { mode: CompactMode::Manual, .. })));
    going(&mut a, &id, CompactPhase::Recompress, Work { done: 1, total: 4, ..Work::default() }, None);
    let texts = drawn(&ctx, &mut a);
    assert!(has(&texts, "Compressing mail again") && has(&texts, "Stop compacting") && has(&texts, "Being compacted…"), "{texts:?}");
    going(&mut a, &id, CompactPhase::Exclusive, Work::default(), Some(9));
    let texts = drawn(&ctx, &mut a);
    assert!(has(&texts, "Finishing: about") && has(&texts, "can't be stopped") && !has(&texts, "Stop compacting"), "{texts:?}");

    // Done: the dialog goes, and the row says what it came to.
    let report =
        CompactReport { mode: CompactMode::Manual, before: 4 << 30, after: 2 << 30, compacted: true, elapsed_ms: 9_000, ..CompactReport::default() };
    a.session.apply(MailEvent::Compacted { account: id.clone(), report: Box::new(report) });
    let texts = drawn(&ctx, &mut a);
    assert!(!matches!(a.ui.dialog, Some(Dialog::Compacting { .. })));
    assert!(has(&texts, "Compacted: 4.0 GB → 2.0 GB (2.0 GB reclaimed) in 9 s"), "{texts:?}");
    // Nothing left to give back: not offered.
    measured(&mut a, &id, DataFileHealth { file_bytes: 2 << 30, live_bytes: 2 << 30, recompressed: true, ..DataFileHealth::default() });
    let texts = drawn(&ctx, &mut a);
    assert!(!has(&texts, "can be reclaimed"), "{texts:?}");
    finish(a, dir);
}

#[test]
fn what_a_compaction_came_to_is_said_in_a_line() {
    use crate::backstage::report_line as line;
    let r = |f: fn(&mut CompactReport)| {
        let mut r = CompactReport { mode: CompactMode::Idle, before: 9, after: 9, ..CompactReport::default() };
        f(&mut r);
        line(&r)
    };
    assert_eq!(r(|_| {}), None, "an idle one that did nothing says nothing");
    assert_eq!(r(|r| r.mode = CompactMode::Manual).as_deref(), Some("Nothing to give back."));
    assert!(r(|r| r.reimport = true).is_some_and(|l| l.contains("importing the file again")));
    assert!(r(|r| r.too_long_s = Some(125)).is_some_and(|l| l.contains("about 2 min")));
    assert!(r(|r| r.cancelled = true).is_some_and(|l| l.contains("stopped")));
    assert!(r(|r| r.error = Some("disk full".into())).is_some_and(|l| l.contains("disk full")));
    assert!(r(|r| (r.copied, r.after) = (true, 4)).is_some_and(|l| l.starts_with("Compacted: ")));
}

#[test]
fn left_alone_the_policy_is_told_and_coming_back_stops_an_idle_compaction() {
    let (mut a, jobs, id, dir) = with_data_file("idle");
    let ctx = egui::Context::default();
    // (The first frame only loads the fonts.)
    frame_at(&ctx, &mut a, 0.5, vec![]);
    frame_at(&ctx, &mut a, 1.0, vec![]);
    jobs.take();
    // Ten minutes without input: measured for the idle policy (told every 30 s, not every frame).
    frame_at(&ctx, &mut a, 400.0, vec![]);
    frame_at(&ctx, &mut a, 700.0, vec![]);
    let measuring = |jobs: &Jobs| jobs.take().into_iter().filter(|j| matches!(j, Job::DataFileHealth { account } if *account == id)).count();
    assert_eq!(measuring(&jobs), 1);
    frame_at(&ctx, &mut a, 701.0, vec![]);
    frame_at(&ctx, &mut a, 731.0, vec![]);
    assert_eq!(measuring(&jobs), 0, "not again the same day");
    // It needs it: compacted at idle; the user comes back and it stops, asked once.
    measured(&mut a, &id, bloated());
    assert!(matches!(a.session.take_jobs().as_slice(), [Job::CompactDataFile { mode: CompactMode::Idle, .. }]));
    going(&mut a, &id, CompactPhase::Recompress, Work::default(), None);
    let moved = || vec![Event::PointerMoved(egui::pos2(300.0, 300.0))];
    frame_at(&ctx, &mut a, 702.0, moved());
    frame_at(&ctx, &mut a, 703.0, moved());
    let stops = jobs.take().into_iter().filter(|j| matches!(j, Job::CancelCompact { account } if *account == id)).count();
    assert_eq!(stops, 1);
    assert!(a.ui.dialog.is_none(), "it happened in the background: nothing over the window");

    // The part that has the file to itself can't be stopped: it's shown over the window, and
    // stays there however it's put away, until it ends.
    a.session.compaction.running.clear();
    a.session.compaction.running.insert(id.clone(), running(CompactMode::Idle, CompactPhase::Exclusive, Work::default(), Some(10)));
    frame_at(&ctx, &mut a, 704.0, moved());
    assert!(jobs.take().iter().all(|j| !matches!(j, Job::CancelCompact { .. })));
    assert!(matches!(&a.ui.dialog, Some(Dialog::Compacting { account, closing: false, since: Some(t) }) if *account == id && *t == 704.0));
    a.ui.dialog = None;
    frame_at(&ctx, &mut a, 709.0, vec![key(Key::Escape, Modifiers::NONE)]);
    assert!(matches!(a.ui.dialog, Some(Dialog::Compacting { .. })), "back");
    // Its bar goes by the time it was expected to take, short of the end.
    let c = a.session.compaction.running[&id].clone();
    let (words, permille) = crate::app::compacting::phase_words(&c, Some(704.0), 709.0);
    assert_eq!((words.as_str(), permille), ("Finishing: about 5 s", Some(500)));
    assert_eq!(crate::app::compacting::phase_words(&c, Some(704.0), 800.0).1, Some(950), "stops short of the end");
    a.session.apply(MailEvent::Compacted { account: id.clone(), report: Box::new(CompactReport::default()) });
    frame_at(&ctx, &mut a, 710.0, vec![]);
    assert!(a.ui.dialog.is_none());
    finish(a, dir);
}

#[test]
fn closing_compacts_when_chosen_and_short_and_offers_a_long_one() {
    let (mut a, jobs, id, dir) = with_data_file("close");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    measured(&mut a, &id, bloated());
    // Off unless chosen: MailCraft just closes.
    close(&ctx, &mut a);
    assert!(a.ui.dialog.is_none() && !a.ui.close_asked);

    // Chosen: compacted as it closes, the close held until it's done.
    a.run("dataFile.compactOnClose", json!({ "on": true }));
    jobs.take();
    close(&ctx, &mut a);
    assert!(matches!(&a.ui.dialog, Some(Dialog::Compacting { closing: true, .. })) && a.ui.compact.closing);
    assert!(jobs.take().iter().any(|j| matches!(j, Job::CompactDataFile { mode: CompactMode::Close, .. })));
    let texts = drawn(&ctx, &mut a);
    assert!(has(&texts, "before closing…") && has(&texts, "Close now"), "{texts:?}");
    assert!(!a.ui.closing_anyway, "not closed yet");
    a.session.apply(MailEvent::Compacted { account: id.clone(), report: Box::new(CompactReport { mode: CompactMode::Close, ..Default::default() }) });
    frame(&ctx, &mut a, vec![]);
    assert!(a.ui.closing_anyway, "done: it closes");

    // Too long for closing: offered by hand, with how long it takes; closing without it closes.
    let (mut a, jobs, id, dir2) = {
        finish(a, dir);
        with_data_file("close-long")
    };
    frame(&ctx, &mut a, vec![]);
    a.run("dataFile.compactOnClose", json!({ "on": true }));
    measured(&mut a, &id, big());
    jobs.take();
    close(&ctx, &mut a);
    assert!(matches!(&a.ui.dialog, Some(Dialog::CompactOnClose { seconds, .. }) if *seconds > 30));
    assert!(jobs.take().iter().all(|j| !matches!(j, Job::CompactDataFile { .. })), "nothing begun without asking");
    let texts = drawn(&ctx, &mut a);
    assert!(has(&texts, "about 3 min") && has(&texts, "Compact, then close"), "{texts:?}");
    a.act(Action::CompactOnClose(Some(id.clone())));
    assert!(matches!(&a.ui.dialog, Some(Dialog::Compacting { closing: true, .. })));
    frame(&ctx, &mut a, vec![]);
    assert!(jobs.take().iter().any(|j| matches!(j, Job::CompactDataFile { mode: CompactMode::Manual, .. })));
    // Close now: closed though it runs (stopped as MailCraft goes; what's done is kept).
    a.act(Action::CompactOnClose(None));
    assert!(a.ui.closing_anyway && a.ui.close_now);
    finish(a, dir2);
}

/// Click the button a screen reader names `label`, as the user does (the frame after shows what
/// it did).
fn click(ctx: &egui::Context, a: &mut MailApp, label: &str) {
    let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))), ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| a.ui(ui));
    out.textures_delta.clear();
    let nodes = out.platform_output.accesskit_update.expect("AccessKit is on").nodes;
    let at = nodes
        .iter()
        .find(|(_, n)| n.label() == Some(label) && n.bounds().is_some() && !n.is_disabled())
        .and_then(|(_, n)| n.bounds())
        .map(|b| egui::pos2(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32))
        .unwrap_or_else(|| panic!("no {label} to click"));
    let button = |pressed| Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    frame(ctx, a, vec![Event::PointerMoved(at)]);
    frame(ctx, a, vec![button(true)]);
    frame(ctx, a, vec![button(false)]);
}

/// "Compact, then close" clicked in the offer compacts the data file and closes once it's done.
/// The dialog is put away before its answer runs, and the answer once read the account from it:
/// on a 38 GB file MailCraft closed at once with nothing compacted.
#[test]
fn compact_then_close_clicked_compacts_before_closing() {
    let (mut a, jobs, id, dir) = with_data_file("close-click");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    frame(&ctx, &mut a, vec![]);
    a.run("dataFile.compactOnClose", json!({ "on": true }));
    measured(&mut a, &id, big());
    jobs.take();
    close(&ctx, &mut a);
    assert!(matches!(&a.ui.dialog, Some(Dialog::CompactOnClose { .. })), "{:?}", a.ui.dialog);
    click(&ctx, &mut a, "Compact, then close");
    assert!(
        jobs.take().iter().any(|j| matches!(j, Job::CompactDataFile { mode: CompactMode::Manual, account, .. } if *account == id)),
        "compacted by hand"
    );
    assert!(matches!(&a.ui.dialog, Some(Dialog::Compacting { account, closing: true, .. }) if *account == id), "{:?}", a.ui.dialog);
    assert!(a.ui.compact.closing && !a.ui.closing_anyway && !a.ui.close_now, "not closed yet");
    frame(&ctx, &mut a, vec![]);
    assert!(!a.ui.closing_anyway && !a.ui.close_now, "still waiting");
    a.session
        .apply(MailEvent::Compacted { account: id.clone(), report: Box::new(CompactReport { mode: CompactMode::Manual, ..Default::default() }) });
    frame(&ctx, &mut a, vec![]);
    assert!(a.ui.closing_anyway, "done: it closes");

    // "Close without compacting", clicked, closes with nothing begun.
    let (mut a, jobs, id, dir2) = {
        finish(a, dir);
        with_data_file("close-click-not")
    };
    frame(&ctx, &mut a, vec![]);
    a.run("dataFile.compactOnClose", json!({ "on": true }));
    measured(&mut a, &id, big());
    jobs.take();
    close(&ctx, &mut a);
    click(&ctx, &mut a, "Close without compacting");
    assert!(jobs.take().iter().all(|j| !matches!(j, Job::CompactDataFile { .. })));
    assert!(a.ui.closing_anyway && a.ui.close_now);
    finish(a, dir2);
}

#[test]
fn putting_the_closing_dialog_away_keeps_mailcraft_open() {
    let (mut a, _jobs, id, dir) = with_data_file("close-kept");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.run("dataFile.compactOnClose", json!({ "on": true }));
    measured(&mut a, &id, bloated());
    close(&ctx, &mut a);
    assert!(a.ui.compact.closing);
    a.ui.dialog = None;
    frame(&ctx, &mut a, vec![]);
    assert!(!a.ui.compact.closing, "the close is off");
    a.session.apply(MailEvent::Compacted { account: id.clone(), report: Box::new(CompactReport { mode: CompactMode::Close, ..Default::default() }) });
    frame(&ctx, &mut a, vec![]);
    assert!(!a.ui.closing_anyway && !a.ui.close_now, "done, and MailCraft stays open");
    finish(a, dir);
}

#[test]
fn the_status_bar_shows_a_compaction_before_the_index() {
    let (mut a, _jobs, id, dir) = with_data_file("status");
    let ctx = egui::Context::default();
    a.session
        .compaction
        .running
        .insert(id.clone(), running(CompactMode::Idle, CompactPhase::Recompress, Work { done: 1, total: 4, ..Work::default() }, None));
    let texts = drawn(&ctx, &mut a);
    assert!(texts.iter().any(|t| t.starts_with("Compacting ") && t.ends_with("… 25%")), "{texts:?}");
    let (text, _, _) = crate::app::compacting::status(&a.session, &a.ui.compact, 0.0).unwrap();
    assert!(text.starts_with("Compacting "));
    a.session.compaction.running.clear();
    assert!(crate::app::compacting::status(&a.session, &a.ui.compact, 0.0).is_none());
    finish(a, dir);
}

/// Closing waits for every data file it compacts: the dialog goes on to the next as one ends,
/// and MailCraft closes once the last is done (it once stayed open, the close dropped silently).
#[test]
fn closing_waits_for_every_data_file_it_compacts() {
    let (mut a, jobs, id, dir) = with_data_file("close-two");
    let other = kept_in_a_data_file(&mut a.session, "yan@example.org");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.run("dataFile.compactOnClose", json!({ "on": true }));
    measured(&mut a, &id, bloated());
    measured(&mut a, &other, bloated());
    jobs.take();
    close(&ctx, &mut a);
    assert!(a.session.compaction.running.contains_key(&id) && a.session.compaction.running.contains_key(&other));
    let Some(Dialog::Compacting { account: first, closing: true, .. }) = a.ui.dialog.clone() else { panic!("{:?}", a.ui.dialog) };
    let second = if first == id { other.clone() } else { id.clone() };
    let done = || Box::new(CompactReport { mode: CompactMode::Close, ..Default::default() });
    a.session.apply(MailEvent::Compacted { account: first.clone(), report: done() });
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    assert!(matches!(&a.ui.dialog, Some(Dialog::Compacting { account, closing: true, .. }) if *account == second), "{:?}", a.ui.dialog);
    assert!(a.ui.compact.closing && !a.ui.closing_anyway, "still waiting for the other");
    a.session.apply(MailEvent::Compacted { account: second.clone(), report: done() });
    frame(&ctx, &mut a, vec![]);
    assert!(a.ui.closing_anyway, "both done: it closes");
    finish(a, dir);
}

/// A data file too long for closing is still offered when another is compacted quickly first.
#[test]
fn a_long_one_is_offered_once_the_quick_ones_are_done() {
    let (mut a, jobs, id, dir) = with_data_file("close-quick-long");
    let other = kept_in_a_data_file(&mut a.session, "yan@example.org");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.run("dataFile.compactOnClose", json!({ "on": true }));
    measured(&mut a, &id, bloated());
    measured(&mut a, &other, big());
    jobs.take();
    close(&ctx, &mut a);
    assert!(matches!(&a.ui.dialog, Some(Dialog::Compacting { account, closing: true, .. }) if *account == id), "{:?}", a.ui.dialog);
    a.session.apply(MailEvent::Compacted { account: id.clone(), report: Box::new(CompactReport { mode: CompactMode::Close, ..Default::default() }) });
    frame(&ctx, &mut a, vec![]);
    assert!(matches!(&a.ui.dialog, Some(Dialog::CompactOnClose { account, .. }) if *account == other), "{:?}", a.ui.dialog);
    assert!(!a.ui.closing_anyway, "asked first");
    a.act(Action::CompactOnClose(None));
    assert!(a.ui.closing_anyway && a.ui.close_now);
    finish(a, dir);
}

/// Reading in a message's own window is the user being back: an idle compaction stops.
#[test]
fn input_in_a_window_of_its_own_stops_an_idle_compaction() {
    let (mut a, jobs, id, dir) = with_data_file("idle-elsewhere");
    let ctx = egui::Context::default();
    frame_at(&ctx, &mut a, 0.5, vec![]);
    frame_at(&ctx, &mut a, 1.0, vec![]);
    a.session.compaction.running.insert(id.clone(), running(CompactMode::Idle, CompactPhase::Recompress, Work::default(), None));
    jobs.take();
    frame_at(&ctx, &mut a, 2.0, vec![]);
    assert!(jobs.take().iter().all(|j| !matches!(j, Job::CancelCompact { .. })), "nothing done yet");
    let mut out =
        ctx.run_ui(RawInput { events: vec![Event::PointerMoved(egui::pos2(5.0, 5.0))], ..Default::default() }, |ui| a.ui.compact.note_input(ui));
    out.textures_delta.clear();
    frame_at(&ctx, &mut a, 3.0, vec![]);
    assert_eq!(jobs.take().iter().filter(|j| matches!(j, Job::CancelCompact { account } if *account == id)).count(), 1);
    finish(a, dir);
}

/// Every state of compacting, its File page row included, drawn in Hebrew and Arabic, light and
/// dark (mirrored windows): nothing panics.
#[test]
fn compacting_is_drawn_right_to_left() {
    for lang in ["he", "ar"] {
        let (mut a, _jobs, _id, dir) = with_data_file(&format!("rtl-{lang}"));
        a.ui.language = lang.to_string();
        crate::i18n::set_current(crate::i18n::Lang::from_code(lang).unwrap());
        let ctx = egui::Context::default();
        for dark in [Some(false), Some(true)] {
            a.ui.dark = dark;
            tour(&mut a, &ctx);
        }
        assert_ne!(crate::i18n::t("{size} can be reclaimed"), "{size} can be reclaimed", "{lang}: translated");
        crate::i18n::set_current(crate::i18n::Lang::EN);
        finish(a, dir);
    }
}
