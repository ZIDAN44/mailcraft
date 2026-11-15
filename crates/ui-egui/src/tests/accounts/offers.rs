//! The Open Outlook data file dialog for every kind of file and every state it can be found in:
//! what it says, the buttons it draws, and what a click on each does.

use egui::{Event, PointerButton, Pos2};
use mailcraft_engine::model::job::{DataFileFacts, DataFileKind, ImportReport, StopHow};

use super::*;
use crate::Action;

/// The buttons the dialog can draw.
const BUTTONS: [&str; 10] = [
    "Cancel",
    "Import another copy",
    "Read the changes in",
    "Continue importing",
    "Go to it",
    "Take it over and read the changes in",
    "Take it over",
    "Open as it was",
    "Open",
    "Import",
];

fn changes(c: Option<mailcraft_engine::Changes>) -> &'static str {
    c.map_or("-", mailcraft_engine::Changes::code)
}

fn act_text(a: &Action) -> String {
    match a {
        Action::ImportDataFile { path, again, changes: c, take_over } => {
            format!("import {path} again={again} changes={} takeOver={take_over}", changes(*c))
        }
        Action::Run(id, v) => format!("run {id} {v}"),
        _ => "another action".into(),
    }
}

/// The dialog for `path` drawn alone (twice: a dialog just opened is measured first), then
/// clicked at `at`: the text drawn before the click, what the click did, and whether the dialog
/// closed.
fn look(s: &Session, path: &str, at: Option<Pos2>) -> (Vec<(String, egui::Rect)>, Vec<String>, bool) {
    let ctx = egui::Context::default();
    let t = crate::theme::Tokens::light();
    let mut pics = crate::reading::Pictures::default();
    let mut d = Dialog::OpenDataFile { path: path.into() };
    let mut open = true;
    let mut run = |events: Vec<Event>, acts: &mut Vec<Action>| {
        let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1280.0, 1600.0))), events, ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| {
            open &= crate::dialogs::show(ui.ctx(), &mut d, s, &mut pics, &t, acts);
        });
        out.textures_delta.clear();
        texts_in(&out.shapes).into_iter().map(|d| (d.text, d.rect)).collect::<Vec<_>>()
    };
    let mut acts = Vec::new();
    run(vec![], &mut acts);
    let texts = run(vec![], &mut acts);
    let mut clicked = Vec::new();
    if let Some(at) = at {
        let press = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        run(vec![Event::PointerMoved(at)], &mut clicked);
        run(vec![press(true)], &mut clicked);
        run(vec![press(false)], &mut clicked);
    }
    let what = acts.iter().chain(&clicked).map(act_text).collect();
    (texts, what, !open)
}

/// What the dialog for `path` shows, and each of its buttons with what a click on it does.
fn offers(s: &Session, path: &str) -> Vec<String> {
    let (texts, before, _) = look(s, path, None);
    assert!(before.is_empty(), "{path}: nothing done before a click: {before:?}");
    let mut out: Vec<String> = texts.iter().filter(|(x, _)| !BUTTONS.contains(&x.as_str())).map(|(x, _)| x.clone()).collect();
    for (label, r) in texts.iter().filter(|(x, _)| BUTTONS.contains(&x.as_str())) {
        let (_, did, closed) = look(s, path, Some(r.center()));
        out.push(format!("[{label}] {} {}", if closed { "closes" } else { "stays" }, did.join("; ")));
    }
    out
}

fn pst(path: &str) -> DataFileFacts {
    DataFileFacts {
        path: path.into(),
        kind: DataFileKind::Pst,
        size: 5 << 20,
        modified: 1_000,
        messages: 40,
        folders: 3,
        estimate: 3 << 20,
        need: 4 << 20,
        free: Some(8 << 30),
        destination: "D:/Mail".into(),
        ..Default::default()
    }
}

fn copy(path: &str, changed: bool, rereadable: bool) -> DataFileFacts {
    DataFileFacts { copy: "D:/Mail/Copy.mailcraft".into(), copy_changed: changed, copy_rereadable: rereadable, ..pst(path) }
}

fn held(path: &str, changed: bool, rereadable: bool) -> DataFileFacts {
    DataFileFacts { copy_held_by: "E:/Other MailCraft".into(), ..copy(path, changed, rereadable) }
}

fn no_room(f: DataFileFacts) -> DataFileFacts {
    DataFileFacts { free: Some(1 << 20), ..f }
}

/// Every kind of file and state the dialog can find: the same words, buttons and clicks as it
/// had before its rows of buttons were drawn from one place.
#[test]
fn every_data_file_offers_its_buttons() {
    let (mut a, dir) = app("data-file-offers");
    let mut shown: Vec<(String, Vec<String>)> = Vec::new();
    // Being looked at.
    a.session.run("account.inspectDataFile", &json!({"path": "D:/Mail/Counting.pst"})).unwrap();
    shown.push(("counting".into(), offers(&a.session, "D:/Mail/Counting.pst")));
    // Looked at, in every state found.
    let cases = [
        (
            "broken",
            DataFileFacts {
                path: "D:/Mail/Broken.pst".into(),
                kind: DataFileKind::Pst,
                error: Some("not an Outlook data file".into()),
                ..Default::default()
            },
        ),
        ("fits", pst("D:/Mail/Fits.pst")),
        ("elsewhere", DataFileFacts { elsewhere: true, destination: "C:/MailCraft Files".into(), ..pst("D:/Mail/Elsewhere.pst") }),
        ("no room", no_room(pst("D:/Mail/Big.pst"))),
        ("no room elsewhere", no_room(DataFileFacts { elsewhere: true, ..pst("D:/Mail/BigElsewhere.pst") })),
        ("mailcraft", DataFileFacts { kind: DataFileKind::MailCraft, ..pst("D:/Mail/Old.mailcraft") }),
        ("mailcraft no room", no_room(DataFileFacts { kind: DataFileKind::MailCraft, ..pst("D:/Mail/BigOld.mailcraft") })),
        ("copy", DataFileFacts { estimate: 0, need: 0, ..copy("D:/Mail/Copy.pst", false, true) }),
        ("copy changed", copy("D:/Mail/Changed.pst", true, true)),
        ("copy changed, no note", copy("D:/Mail/NoNote.pst", true, false)),
        ("copy changed, no room", no_room(copy("D:/Mail/ChangedBig.pst", true, true))),
        ("held", held("D:/Mail/Held.pst", false, true)),
        ("held changed", held("D:/Mail/HeldChanged.pst", true, true)),
        ("held changed, no note", held("D:/Mail/HeldNoNote.pst", true, false)),
        ("held changed, no room", no_room(held("D:/Mail/HeldBig.pst", true, true))),
    ];
    for (name, facts) in cases {
        let path = facts.path.clone();
        a.session.apply(MailEvent::DataFileInspected { path: path.clone(), facts: Box::new(facts) });
        shown.push((name.into(), offers(&a.session, &path)));
    }
    // Open as an account already: importing, stopped, done; then the file changed since, with
    // and without a note of what came in, and no room for another copy.
    let path = "D:/Mail/Open.pst";
    a.session.apply(MailEvent::DataFileInspected { path: path.into(), facts: Box::new(pst(path)) });
    let id = a.session.run("account.importPst", &json!({"path": path})).unwrap()["account"].as_str().unwrap().to_string();
    a.session.take_jobs();
    let inbox = mailcraft_engine::model::Folder {
        account: id.clone(),
        path: "INBOX".into(),
        role: mailcraft_engine::model::FolderRole::Inbox,
        selectable: true,
        ..Default::default()
    };
    a.session.apply(MailEvent::Folders { account: id.clone(), folders: vec![inbox] });
    shown.push(("open, importing".into(), offers(&a.session, path)));
    a.session.run("account.stopImport", &json!({"account": id})).unwrap();
    let stopped = ImportReport { messages: 5, stopped: Some(StopHow::Cancel), ..Default::default() };
    a.session.apply(MailEvent::Imported { account: id.clone(), messages: 5, skipped: 0, done: true, error: None, report: Some(Box::new(stopped)) });
    shown.push(("open, stopped".into(), offers(&a.session, path)));
    a.session.run("account.resumeImport", &json!({"account": id})).unwrap();
    let done = ImportReport { file: path.into(), messages: 40, ..Default::default() };
    a.session.apply(MailEvent::Imported { account: id.clone(), messages: 40, skipped: 0, done: true, error: None, report: Some(Box::new(done)) });
    a.session.take_jobs();
    shown.push(("open, done".into(), offers(&a.session, path)));
    a.session
        .apply(MailEvent::DataFileInspected { path: path.into(), facts: Box::new(DataFileFacts { size: 6 << 20, modified: 2_000, ..pst(path) }) });
    shown.push(("open, changed, no note".into(), offers(&a.session, path)));
    let seen = dir.join(&id);
    std::fs::create_dir_all(&seen).unwrap();
    std::fs::write(seen.join(mailcraft_engine::store::SEEN_DOC), "{}").unwrap();
    assert!(a.session.can_read_changes(&id), "the note is found");
    shown.push(("open, changed".into(), offers(&a.session, path)));
    a.session.apply(MailEvent::DataFileInspected {
        path: path.into(),
        facts: Box::new(no_room(DataFileFacts { size: 6 << 20, modified: 2_000, ..pst(path) })),
    });
    shown.push(("open, changed, no room".into(), offers(&a.session, path)));
    a.session.run("account.importPst", &json!({"path": path, "changes": "read"})).unwrap();
    a.session.take_jobs();
    shown.push(("open, changed, reading it".into(), offers(&a.session, path)));
    assert_eq!(shown.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), EXPECTED.iter().map(|(n, _)| *n).collect::<Vec<_>>());
    for ((name, got), (_, want)) in shown.iter().zip(EXPECTED) {
        assert_eq!(got, want, "{name}");
    }
}

/// What the dialog showed for each, before its rows of buttons were drawn from one place: its
/// words, then each button and what a click on it did.
const EXPECTED: &[(&str, &[&str])] = &[
    ("counting", &["Open Outlook data file", "Counting.pst", "Counting items…", "[Import] stays ", "[Cancel] closes "]),
    (
        "broken",
        &[
            "Open Outlook data file",
            "Broken.pst",
            "This file couldn't be looked at: not an Outlook data file",
            "[Import] closes import D:/Mail/Broken.pst again=false changes=- takeOver=false",
            "[Cancel] closes ",
        ],
    ),
    (
        "fits",
        &[
            "Open Outlook data file",
            "Fits.pst",
            "5.0 MB   ·   40 items in 3 folders",
            "MailCraft keeps its own copy beside the file, about 3.0 MB, in D:/Mail. The file itself is never changed.",
            "Free there: 8.0 GB",
            "[Import] closes import D:/Mail/Fits.pst again=false changes=- takeOver=false",
            "[Cancel] closes ",
        ],
    ),
    (
        "elsewhere",
        &[
            "Open Outlook data file",
            "Elsewhere.pst",
            "5.0 MB   ·   40 items in 3 folders",
            "This file's folder can't be written: MailCraft's copy, about 3.0 MB, goes in C:/MailCraft Files.",
            "Free there: 8.0 GB",
            "[Import] closes import D:/Mail/Elsewhere.pst again=false changes=- takeOver=false",
            "[Cancel] closes ",
        ],
    ),
    (
        "no room",
        &[
            "Open Outlook data file",
            "Big.pst",
            "5.0 MB   ·   40 items in 3 folders",
            "MailCraft keeps its own copy beside the file, about 3.0 MB, in D:/Mail. The file itself is never changed.",
            "Free there: 1.0 MB",
            "There isn't room for the copy beside the file. Free up space on that drive.",
            "[Import] stays ",
            "[Cancel] closes ",
        ],
    ),
    (
        "no room elsewhere",
        &[
            "Open Outlook data file",
            "BigElsewhere.pst",
            "5.0 MB   ·   40 items in 3 folders",
            "This file's folder can't be written: MailCraft's copy, about 3.0 MB, goes in D:/Mail.",
            "Free there: 1.0 MB",
            "There isn't room for the copy. Free up space, or move MailCraft's data folder (File › Options).",
            "[Import] stays ",
            "[Cancel] closes ",
        ],
    ),
    (
        "mailcraft",
        &[
            "Open Outlook data file",
            "Old.mailcraft",
            "5.0 MB   ·   40 items in 3 folders",
            "MailCraft makes its own copy, about 3.0 MB, in D:/Mail.",
            "Free there: 8.0 GB",
            "[Import] closes import D:/Mail/Old.mailcraft again=false changes=- takeOver=false",
            "[Cancel] closes ",
        ],
    ),
    (
        "mailcraft no room",
        &[
            "Open Outlook data file",
            "BigOld.mailcraft",
            "5.0 MB   ·   40 items in 3 folders",
            "MailCraft makes its own copy, about 3.0 MB, in D:/Mail.",
            "Free there: 1.0 MB",
            "There isn't room for the copy. Free up space, or move MailCraft's data folder (File › Options).",
            "[Import] stays ",
            "[Cancel] closes ",
        ],
    ),
    (
        "copy",
        &[
            "Open Outlook data file",
            "Copy.pst",
            "5.0 MB   ·   40 items in 3 folders",
            "MailCraft's copy of this file is beside it, with what was changed in MailCraft: D:/Mail/Copy.mailcraft",
            "[Open] closes import D:/Mail/Copy.pst again=false changes=- takeOver=false",
            "[Cancel] closes ",
        ],
    ),
    (
        "copy changed",
        &[
            "Open Outlook data file",
            "Changed.pst",
            "5.0 MB   ·   40 items in 3 folders",
            "MailCraft's copy of this file is beside it, with what was changed in MailCraft: D:/Mail/Copy.mailcraft",
            "The file has changed since MailCraft read it.",
            "Free there: 8.0 GB",
            "[Cancel] closes ",
            "[Open as it was] closes import D:/Mail/Changed.pst again=false changes=keep takeOver=false",
            "[Read the changes in] closes import D:/Mail/Changed.pst again=false changes=read takeOver=false",
        ],
    ),
    (
        "copy changed, no note",
        &[
            "Open Outlook data file",
            "NoNote.pst",
            "5.0 MB   ·   40 items in 3 folders",
            "MailCraft's copy of this file is beside it, with what was changed in MailCraft: D:/Mail/Copy.mailcraft",
            "The file has changed since MailCraft read it.",
            "MailCraft has no note of what came in from this file before, so its changes can't be read in.",
            "Free there: 8.0 GB",
            "[Cancel] closes ",
            "[Open as it was] closes import D:/Mail/NoNote.pst again=false changes=keep takeOver=false",
        ],
    ),
    (
        "copy changed, no room",
        &[
            "Open Outlook data file",
            "ChangedBig.pst",
            "5.0 MB   ·   40 items in 3 folders",
            "MailCraft's copy of this file is beside it, with what was changed in MailCraft: D:/Mail/Copy.mailcraft",
            "The file has changed since MailCraft read it.",
            "Free there: 1.0 MB",
            "There isn't room for the copy. Free up space, or move MailCraft's data folder (File › Options).",
            "[Cancel] closes ",
            "[Open as it was] closes import D:/Mail/ChangedBig.pst again=false changes=keep takeOver=false",
            "[Read the changes in] stays ",
        ],
    ),
    (
        "held",
        &[
            "Open Outlook data file",
            "Held.pst",
            "5.0 MB   ·   40 items in 3 folders",
            "MailCraft's copy of this file is beside it, with what was changed in MailCraft: D:/Mail/Copy.mailcraft",
            "It's an account in another MailCraft data folder: E:/Other MailCraft",
            "Open it there, or take it over here: it's then this data folder's, and its account there should be removed.",
            "Free there: 8.0 GB",
            "[Cancel] closes ",
            "[Import another copy] closes import D:/Mail/Held.pst again=true changes=- takeOver=false",
            "[Take it over] closes import D:/Mail/Held.pst again=false changes=- takeOver=true",
        ],
    ),
    (
        "held changed",
        &[
            "Open Outlook data file",
            "HeldChanged.pst",
            "5.0 MB   ·   40 items in 3 folders",
            "MailCraft's copy of this file is beside it, with what was changed in MailCraft: D:/Mail/Copy.mailcraft",
            "It's an account in another MailCraft data folder: E:/Other MailCraft",
            "Open it there, or take it over here: it's then this data folder's, and its account there should be removed.",
            "The file has changed since MailCraft read it.",
            "Free there: 8.0 GB",
            "[Cancel] closes ",
            "[Import another copy] closes import D:/Mail/HeldChanged.pst again=true changes=- takeOver=false",
            "[Take it over and read the changes in] closes import D:/Mail/HeldChanged.pst again=false changes=read takeOver=true",
            "[Take it over] closes import D:/Mail/HeldChanged.pst again=false changes=keep takeOver=true",
        ],
    ),
    (
        "held changed, no note",
        &[
            "Open Outlook data file",
            "HeldNoNote.pst",
            "5.0 MB   ·   40 items in 3 folders",
            "MailCraft's copy of this file is beside it, with what was changed in MailCraft: D:/Mail/Copy.mailcraft",
            "It's an account in another MailCraft data folder: E:/Other MailCraft",
            "Open it there, or take it over here: it's then this data folder's, and its account there should be removed.",
            "The file has changed since MailCraft read it.",
            "MailCraft has no note of what came in from this file before, so its changes can't be read in.",
            "Free there: 8.0 GB",
            "[Cancel] closes ",
            "[Import another copy] closes import D:/Mail/HeldNoNote.pst again=true changes=- takeOver=false",
            "[Take it over] closes import D:/Mail/HeldNoNote.pst again=false changes=keep takeOver=true",
        ],
    ),
    (
        "held changed, no room",
        &[
            "Open Outlook data file",
            "HeldBig.pst",
            "5.0 MB   ·   40 items in 3 folders",
            "MailCraft's copy of this file is beside it, with what was changed in MailCraft: D:/Mail/Copy.mailcraft",
            "It's an account in another MailCraft data folder: E:/Other MailCraft",
            "Open it there, or take it over here: it's then this data folder's, and its account there should be removed.",
            "The file has changed since MailCraft read it.",
            "Free there: 1.0 MB",
            "There isn't room for the copy. Free up space, or move MailCraft's data folder (File › Options).",
            "[Cancel] closes ",
            "[Import another copy] stays ",
            "[Take it over and read the changes in] stays ",
            "[Take it over] stays ",
        ],
    ),
    (
        "open, importing",
        &[
            "Open Outlook data file",
            "Open.pst",
            "5.0 MB   ·   40 items in 3 folders",
            "MailCraft keeps its own copy beside the file, about 3.0 MB, in D:/Mail. The file itself is never changed.",
            "Free there: 8.0 GB",
            "This data file is already open as Open.",
            "[Cancel] closes ",
            "[Import another copy] closes import D:/Mail/Open.pst again=true changes=- takeOver=false",
            "[Go to it] closes run folder.open {\"account\":\"a2\",\"folder\":\"INBOX\"}",
        ],
    ),
    (
        "open, stopped",
        &[
            "Open Outlook data file",
            "Open.pst",
            "5.0 MB   ·   40 items in 3 folders",
            "MailCraft keeps its own copy beside the file, about 3.0 MB, in D:/Mail. The file itself is never changed.",
            "Free there: 8.0 GB",
            "This data file is already open as Open.",
            "[Cancel] closes ",
            "[Import another copy] closes import D:/Mail/Open.pst again=true changes=- takeOver=false",
            "[Continue importing] closes run account.resumeImport {\"account\":\"a2\"}",
            "[Go to it] closes run folder.open {\"account\":\"a2\",\"folder\":\"INBOX\"}",
        ],
    ),
    (
        "open, done",
        &[
            "Open Outlook data file",
            "Open.pst",
            "5.0 MB   ·   40 items in 3 folders",
            "MailCraft keeps its own copy beside the file, about 3.0 MB, in D:/Mail. The file itself is never changed.",
            "Free there: 8.0 GB",
            "This data file is already open as Open.",
            "[Cancel] closes ",
            "[Import another copy] closes import D:/Mail/Open.pst again=true changes=- takeOver=false",
            "[Go to it] closes run folder.open {\"account\":\"a2\",\"folder\":\"INBOX\"}",
        ],
    ),
    (
        "open, changed, no note",
        &[
            "Open Outlook data file",
            "Open.pst",
            "6.0 MB   ·   40 items in 3 folders",
            "MailCraft keeps its own copy beside the file, about 3.0 MB, in D:/Mail. The file itself is never changed.",
            "Free there: 8.0 GB",
            "This data file is already open as Open.",
            "The file has changed since MailCraft read it.",
            "MailCraft has no note of what came in from this file before, so its changes can't be read in.",
            "[Cancel] closes ",
            "[Import another copy] closes import D:/Mail/Open.pst again=true changes=- takeOver=false",
            "[Go to it] closes run folder.open {\"account\":\"a2\",\"folder\":\"INBOX\"}",
        ],
    ),
    (
        "open, changed",
        &[
            "Open Outlook data file",
            "Open.pst",
            "6.0 MB   ·   40 items in 3 folders",
            "MailCraft keeps its own copy beside the file, about 3.0 MB, in D:/Mail. The file itself is never changed.",
            "Free there: 8.0 GB",
            "This data file is already open as Open.",
            "The file has changed since MailCraft read it.",
            "[Cancel] closes ",
            "[Import another copy] closes import D:/Mail/Open.pst again=true changes=- takeOver=false",
            "[Read the changes in] closes import D:/Mail/Open.pst again=false changes=read takeOver=false",
            "[Go to it] closes run folder.open {\"account\":\"a2\",\"folder\":\"INBOX\"}",
        ],
    ),
    (
        "open, changed, no room",
        &[
            "Open Outlook data file",
            "Open.pst",
            "6.0 MB   ·   40 items in 3 folders",
            "MailCraft keeps its own copy beside the file, about 3.0 MB, in D:/Mail. The file itself is never changed.",
            "Free there: 1.0 MB",
            "There isn't room for the copy beside the file. Free up space on that drive.",
            "This data file is already open as Open.",
            "The file has changed since MailCraft read it.",
            "[Cancel] closes ",
            "[Import another copy] stays ",
            "[Read the changes in] closes import D:/Mail/Open.pst again=false changes=read takeOver=false",
            "[Go to it] closes run folder.open {\"account\":\"a2\",\"folder\":\"INBOX\"}",
        ],
    ),
    (
        "open, changed, reading it",
        &[
            "Open Outlook data file",
            "Open.pst",
            "6.0 MB   ·   40 items in 3 folders",
            "MailCraft keeps its own copy beside the file, about 3.0 MB, in D:/Mail. The file itself is never changed.",
            "Free there: 1.0 MB",
            "There isn't room for the copy beside the file. Free up space on that drive.",
            "This data file is already open as Open.",
            "The file has changed since MailCraft read it.",
            "[Cancel] closes ",
            "[Import another copy] stays ",
            "[Go to it] closes run folder.open {\"account\":\"a2\",\"folder\":\"INBOX\"}",
        ],
    ),
];
