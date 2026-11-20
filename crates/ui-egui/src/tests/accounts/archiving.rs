//! UI tests: File › Info › Clean up old items (issue #179): the dialog it opens, what it asks the
//! engine for, and a second asked for while one runs. AutoArchive's settings, a folder's own and
//! its question (issue #180).

use mailcraft_engine::model::{Job, dates};

use super::*;
use crate::Action;
use crate::backstage::{Backstage, Page};
use crate::dialogs::ArchiveForm;

#[test]
fn old_mail_is_archived_from_the_file_page() {
    let _tall = crate::tests::windows::layout::Dressed::new(egui::vec2(1280.0, 2400.0), "");
    let store = scratch_store(Store::open(scratch("archive-page")).unwrap());
    sample::seed_if_empty(&store, NOW).unwrap();
    let jobs = Jobs::default();
    let mut a = MailApp::new(Session::open(Some(store), Clock::new(NOW, 0)), Box::new(jobs.clone()), Box::new(crate::NullServices(NOW)));
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut b = Backstage::new(Page::Info);
    b.account = Some("sample".into());
    a.ui.backstage = Some(b);
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    click_named(&ctx, &mut a, "Clean up old items…");
    frame(&ctx, &mut a, vec![]);
    let Some(Dialog::Archive(f)) = a.ui.dialog.clone() else { panic!("no archive dialog: {:?}", a.ui.dialog) };
    // The account's whole mailbox, older than six months, the file still to pick.
    let today = dates::today(&a.session.clock);
    let day = crate::pim_dialogs::pattern_day(&f.before).unwrap();
    assert!(f.account == "sample" && f.folder.is_none() && f.path.is_empty(), "{f:?}");
    assert!((today - 186..=today - 179).contains(&day), "six months back: {}", f.before);

    // A folder and a file picked: the day as it starts here, the folder, the file.
    let f = ArchiveForm { folder: Some("Projects".into()), path: " C:/Mail/archive.pst ".into(), ..*f };
    let p = f.params(&a.session).unwrap();
    assert_eq!(p["before"], json!(a.session.clock.midnight(day)));
    a.act(Action::Submit("dataFile.archive", p));
    assert!(a.ui.dialog.is_none());
    frame(&ctx, &mut a, vec![]);
    let asked = jobs.take();
    assert!(
        asked.iter().any(|j| matches!(j, Job::ArchivePst { account, path, folders, .. }
            if account == "sample" && path == "C:/Mail/archive.pst" && folders.as_slice() == ["Projects"])),
        "{asked:?}"
    );

    // Another while it runs: said in the dialog, which stays.
    a.ui.dialog = Some(Dialog::Archive(Box::new(f.clone())));
    a.act(Action::Submit("dataFile.archive", f.params(&a.session).unwrap()));
    assert!(
        matches!(&a.ui.dialog, Some(Dialog::Archive(f)) if f.error.as_deref().is_some_and(|e| e.contains("archived already"))),
        "{:?}",
        a.ui.dialog
    );
    assert!(jobs.take().is_empty());
}

/// AutoArchive (issue #180): its settings from File › Options › Data files, its file suggested
/// beside the data files; a folder's own in its Properties; and its question, which opens by
/// itself once it's due, and puts it off when it's not now.
#[test]
fn autoarchive_is_set_and_asks() {
    let _tall = crate::tests::windows::layout::Dressed::new(egui::vec2(1280.0, 2400.0), "");
    let (mut a, dir) = app("auto-archive-page");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    a.ui.data_folder = Some((dir.join("MailCraft Files"), true));
    a.ui.backstage = Some(Backstage::new(Page::Options));
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    click_named(&ctx, &mut a, "AutoArchive settings…");
    frame(&ctx, &mut a, vec![]);
    let Some(Dialog::AutoArchive(f)) = a.ui.dialog.clone() else { panic!("no AutoArchive settings: {:?}", a.ui.dialog) };
    let suggested = dir.join("MailCraft Files").join("archive.pst").display().to_string();
    assert!(!f.on && f.prompt && f.every == 14 && f.rule.path == suggested, "{f:?}");
    let f = crate::dialogs::AutoArchiveForm { on: true, ..*f };
    a.ui.dialog = Some(Dialog::AutoArchive(Box::new(f.clone())));
    a.act(Action::Submit("autoArchive.set", f.params()));
    assert!(a.ui.dialog.is_none());
    let set = &a.session.prefs.auto_archive;
    assert_eq!((set.every_days, set.rule.path.as_str(), set.rule.older_than), (14, suggested.as_str(), 6));

    // A folder's own: never archived.
    let projects = a.session.folder("sample", "Projects").cloned().unwrap();
    let Dialog::FolderProperties { archive: Some(form), .. } = Dialog::folder_properties(&a.session, &projects) else {
        panic!("no AutoArchive in its Properties")
    };
    assert_eq!(form.mode, crate::dialogs::FolderMode::Default);
    let form = crate::dialogs::FolderArchiveForm { mode: crate::dialogs::FolderMode::Off, ..*form };
    a.act(Action::Submit("autoArchive.folder", form.params()));
    assert_eq!(a.session.prefs.auto_archive.folder("sample", "Projects"), Some(None));

    // Due: it asks, once nothing else is open; Not now puts it off.
    a.ui.backstage = None;
    a.session.prefs.auto_archive.last_run = NOW - 15 * 86_400;
    a.session.tick();
    frame(&ctx, &mut a, vec![]);
    assert!(matches!(a.ui.dialog, Some(Dialog::AutoArchiveDue)), "{:?}", a.ui.dialog);
    click_named(&ctx, &mut a, "Cancel");
    frame(&ctx, &mut a, vec![]);
    assert!(a.ui.dialog.is_none());
    assert_eq!(a.session.prefs.auto_archive.last_run, a.session.clock.now, "put off from now");
}
