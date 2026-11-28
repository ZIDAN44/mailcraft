//! Every dialog, one at a time, filled in as a user would see it, for screenshots of the whole
//! interface before and after a change (the `mailcraft-ui` skill's `shots.ps1`). Opened with
//! `--cmd 'screens.dialog={"i":7}'` in a build with the `screens` feature
//! (`cargo build -p mailcraft --features screens`); never in a release, which turns on no feature.
//! The interface's own tests build it too, so it keeps up with the dialogs: one added is on the
//! list or the test below fails.

use crate::*;

/// Dialog `i` of the list (`None` past its end, or for one that needs what the window hasn't:
/// a message open, the sample's Projects folder or its events). The first 54 keep the numbers
/// the audit's shots had (`d00` to `d53`).
pub fn dialog(a: &mut MailApp, i: usize) -> Option<Dialog> {
    all(a).into_iter().nth(i).flatten()
}

/// How many there are.
pub fn count(a: &mut MailApp) -> usize {
    all(a).len()
}

fn all(a: &mut MailApp) -> Vec<Option<Dialog>> {
    // The sample's, as the window has them: a message open (for Snooze, Follow up and a
    // picture), one of its events, and Gmail's sign-in offered (Add account's button for it).
    if a.session.reading.is_none()
        && let Some(first) = a.session.list.first().map(|m| m.key.to_id())
    {
        a.run("list.open", serde_json::json!({"key": first}));
    }
    a.session.oauth_available = vec!["google".into()];
    let key = a.session.reading.as_ref().map(|r| r.key.clone());
    let event = a.session.pim.get("sample").and_then(|p| p.events.first()).map(|e| e.event.clone());
    let projects = a.session.folder("sample", "Projects").cloned();
    let account = a.session.accounts.first().map(|x| x.config.clone());
    vec![
        Some(Dialog::add_account()),
        Some(Dialog::move_to()),
        Some(Dialog::ConfirmDelete { count: 2, one: None }),
        Some(Dialog::ConfirmDeleteContact { uid: "zoe".into(), name: "Zoe Park".into() }),
        Some(Dialog::EmptySubject),
        Some(Dialog::SendCheck(vec![
            mailcraft_engine::SendCheck::Lookalike { address: "orders@printsh0p.example".into(), like: "printshop.example".into() },
            mailcraft_engine::SendCheck::Exposed { count: 12 },
        ])),
        Some(Dialog::CheckNames(Box::new(crate::dialogs::CheckNamesForm {
            asks: vec![crate::dialogs::NameChoice {
                field: "to".into(),
                typed: "ana".into(),
                people: vec![("Ana Lima".into(), "ana@soundlab.example".into()), (String::new(), "ana.r@studio.example".into())],
            }],
            picked: Some(1),
        }))),
        (|| Some(Dialog::edit_account(&account?)))(),
        Some(Dialog::vacation(
            "sample",
            &mailcraft_engine::model::job::Vacation {
                enabled: true,
                subject: "Away".into(),
                text: "Back soon".into(),
                from: Some("2026-10-15".into()),
                until: None,
                ..Default::default()
            },
            false,
        )),
        Some(Dialog::vacation(
            "sample",
            &mailcraft_engine::model::job::Vacation {
                enabled: true,
                text: "Back soon".into(),
                external: Some("Out of office".into()),
                outside: "known".into(),
                ..Default::default()
            },
            true,
        )),
        Some(Dialog::ConfirmEmpty { account: "sample".into(), folder: "Trash".into(), name: "Deleted Items".into() }),
        Some(Dialog::ConfirmEmpty { account: "sample".into(), folder: "Junk".into(), name: "Junk Email".into() }),
        Some(Dialog::FolderProperties {
            name: "Inbox".into(),
            account: "riley@studio.example".into(),
            location: "INBOX".into(),
            total: 7,
            unread: 3,
            size: 20_480,
            archive: None,
        }),
        // A folder's own AutoArchive setting (issue #180), deleting its old mail.
        Some(Dialog::FolderProperties {
            name: "Projects".into(),
            account: "riley@studio.example".into(),
            location: "Projects".into(),
            total: 2,
            unread: 0,
            size: 4_096,
            archive: crate::dialogs::FolderArchiveForm::new(&a.session, "sample", "Projects", "")
                .map(|f| Box::new(crate::dialogs::FolderArchiveForm { mode: crate::dialogs::FolderMode::Own, ..f })),
        }),
        Some(Dialog::OpenSharedCalendar { email: "ana@contoso.example".into(), error: Some("no access".into()) }),
        Some(Dialog::SubscribeCalendar { url: "webcal://holidays.example/de.ics".into(), name: String::new(), error: Some("HTTP 404".into()) }),
        Some(Dialog::AddSharedMailbox { account: "sample".into(), address: "sales@contoso.example".into(), error: Some("already open".into()) }),
        Some(Dialog::Address(crate::dialogs::AddressForm {
            account: "sample".into(),
            address: "shop@studio.example".into(),
            name: "The Shop".into(),
            reply_to: "help@studio.example".into(),
            error: Some("that's the account's own address".into()),
            ..Default::default()
        })),
        Some(Dialog::Dav(crate::dialogs::DavForm {
            account: "sample".into(),
            caldav: "https://dav.example.com/calendars/riley/default".into(),
            own: true,
            username: "riley@studio.example".into(),
            kept: Some("riley@studio.example".into()),
            error: Some("no calendars were found".into()),
            ..Default::default()
        })),
        Some(Dialog::RemoveAccount { account: "sample".into(), email: "x".into() }),
        Some(Dialog::MailboxClosed {
            account: "sample".into(),
            email: "zoe@hotmail.com".into(),
            error: format!("{}. Turn on IMAP on Outlook.com.", mailcraft_engine::model::MICROSOFT_IMAP_CLOSED),
        }),
        Some(Dialog::TrustCertificate {
            account: "sample".into(),
            shown: Box::new(mailcraft_engine::model::UntrustedCertificate {
                host: "127.0.0.1".into(),
                fingerprint: "0a1b2c3d".repeat(8),
                ..Default::default()
            }),
        }),
        Some(Dialog::TrustCertificate {
            account: "sample".into(),
            shown: Box::new(mailcraft_engine::model::UntrustedCertificate {
                host: "mail.corp.test".into(),
                fingerprint: "0a1b2c3d".repeat(8),
                remote: true,
                subject: "mail.corp.test".into(),
                issuer: "Corp Internal CA".into(),
                names: vec!["mail.corp.test".into(), "imap.corp.test".into()],
                not_before: 1_700_000_000,
                not_after: 1_900_000_000,
            }),
        }),
        Some(Dialog::add_account_with("zoe@hotmail.com", true)),
        Some(Dialog::Signature(Box::new(crate::dialogs::SignatureForm::new(&a.session, "sample", false)))),
        Some(Dialog::Signature(Box::new(crate::dialogs::SignatureForm::new(&a.session, "sample", true)))),
        Some(Dialog::Password { account: "sample".into(), email: "x".into(), user: "riley".into(), password: String::new() }),
        Some(Dialog::Certificate { account: "sample".into(), path: "riley.p12".into(), data: None, password: String::new() }),
        Some(Dialog::Certificate { account: "sample".into(), path: String::new(), data: None, password: String::new() }),
        Some(Dialog::Backup(Default::default())),
        Some(Dialog::Archive(Box::new(crate::dialogs::ArchiveForm::new(&a.session, "sample")))),
        Some(Dialog::Archive(Box::new(crate::dialogs::ArchiveForm {
            folder: Some("Projects".into()),
            path: r"C:\Users\riley\Documents\archive.pst".into(),
            error: Some("Old mail is being archived already: wait until that's done.".into()),
            ..crate::dialogs::ArchiveForm::new(&a.session, "sample")
        }))),
        Some(Dialog::AutoArchive(Box::new(crate::dialogs::AutoArchiveForm::new(
            &a.session,
            r"C:\Users\riley\Documents\MailCraft Files\archive.pst",
        )))),
        Some(Dialog::AutoArchiveDue),
        Some(Dialog::Restore(crate::dialogs::RestoreForm { name: "MailCraft backup 2026-11-13.mailcraft-backup".into(), ..Default::default() })),
        Some(Dialog::PgpKey { account: "sample".into(), path: "ana-secret.asc".into(), passphrase: String::new() }),
        Some(Dialog::SmartCard { account: "sample".into(), module: "/usr/lib/opensc-pkcs11.so".into(), pin: "1234".into(), system: false }),
        Some(Dialog::SmartCard { account: "sample".into(), module: String::new(), pin: String::new(), system: true }),
        Some(Dialog::new_folder("sample", Some("Projects"))),
        Some(Dialog::new_folder("sample", None)),
        (|| Some(Dialog::rename_folder(projects.as_ref()?)))(),
        (|| Some(Dialog::move_folder(projects.as_ref()?)))(),
        (|| Some(Dialog::delete_folder(projects.as_ref()?)))(),
        Some(Dialog::NewCategory { name: "Clients".into(), color: 3, keyword: String::new(), editing: None, error: Some("taken".into()) }),
        Some(Dialog::NewCategory { name: "Work".into(), color: 1, keyword: "bad word".into(), editing: Some("Work".into()), error: None }),
        Some(Dialog::name_keyword("r=c3=bcckfrage")),
        Some(Dialog::Categories),
        Some(Dialog::Rules),
        Some(Dialog::rule_from("zoe@studio.example", None)),
        Some(Dialog::add_account_with("zoe@gmail.com", true)),
        Some(Dialog::person(&mut a.session, "zoe@studio.example")),
        Some(Dialog::person(&mut a.session, "nobody@nowhere.example")),
        Some(Dialog::Preview {
            title: "notes.txt".into(),
            header: vec![("From".into(), "Zoe".into())],
            text: "line one\nline two".into(),
            warning: Some(mailcraft_engine::model::lookalike::SenderWarning::Lookalike {
                domain: "printsh0p.example".into(),
                like: "printshop.example".into(),
            }),
            open: None,
        }),
        Some(Dialog::Preview {
            title: "favour.eml".into(),
            header: vec![("From".into(), "Zoe Park".into())],
            text: "Are you at your desk?".into(),
            warning: Some(mailcraft_engine::model::lookalike::SenderWarning::NameOfKnownPerson {
                name: "Zoe Park".into(),
                known: "zoe@studio.example".into(),
                real: "zoe.park.director@gmail.com".into(),
            }),
            open: None,
        }),
        // Shown with nothing open behind them (their own views' shots have the event, task and
        // contact forms filled in the window).
        Some(Dialog::SaveChanges),
        Some(Dialog::CtrlEnter { remember: false }),
        Some(Dialog::SaveOnClose(2)),
        Some(Dialog::LostOnClose { unsent: 1, changes: 2, sending: 1 }),
        Some(Dialog::UpdateOnClose(crate::UpdateState::Ready { version: "0.44.0".into(), asks: Default::default() })),
        Some(Dialog::ImportOnClose { file: r"C:\Users\riley\Documents\Outlook Files\riley.pst".into(), permille: Some(420) }),
        Some(Dialog::Compacting { account: "sample".into(), closing: false, since: None }),
        Some(Dialog::CompactOnClose { account: "sample".into(), seconds: 40 }),
        Some(Dialog::OpenDataFile { path: r"C:\Users\riley\Documents\Outlook Files\riley.pst".into() }),
        Some(Dialog::StopImport { account: "sample".into(), file: "riley.pst".into() }),
        Some(Dialog::ImportDone {
            account: "sample".into(),
            report: Box::new(mailcraft_engine::model::job::ImportReport {
                file: "riley.pst".into(),
                data_file: "riley.mailcraft".into(),
                ..Default::default()
            }),
        }),
        Some(Dialog::AddRoom {
            name: "Studio Room".into(),
            url: "https://meet.example/studio".into(),
            code: String::new(),
            details: String::new(),
            error: None,
        }),
        Some(Dialog::WordCount(crate::rich_edit::Counts { words: 248, chars: 1_412, chars_no_spaces: 1_170, paragraphs: 6, lines: 21 }, false)),
        Some(Dialog::new_event(&a.session, None)),
        Some(Dialog::new_task(&a.session, None)),
        Some(Dialog::Contact(Box::default())),
        crate::pim_dialogs::FollowUpForm::for_selection(&a.session, true).map(|f| Dialog::FollowUp(Box::new(f))),
        crate::pim_dialogs::SnoozeForm::for_selection(&a.session).map(|f| Dialog::Snooze(Box::new(f))),
        event.as_ref().map(|e| Dialog::Propose(Box::new(crate::pim_dialogs::ProposeForm::new(&a.session, e)))),
        Some(Dialog::WaitingInvites(Box::default())),
        event.as_ref().map(|e| Dialog::OpenRecurring { uid: e.uid.clone(), occurrence: e.start, summary: e.summary.clone() }),
        event
            .as_ref()
            .map(|e| Dialog::PassMeeting(crate::pim_dialogs::PassForm::new(crate::pim_dialogs::Passing::Forward, &e.uid, &e.summary, None))),
        event.as_ref().map(|e| Dialog::CancelMeeting(crate::pim_dialogs::CancelForm::new(&e.uid, &e.summary, None))),
        Some(Dialog::names_for(crate::dialogs::NamesFor::Calendars, "")),
        Some(Dialog::NewTaskList { name: "Errands".into(), error: None }),
        Some(Dialog::SearchFolder(Box::new(crate::dialogs::SearchFolderForm::new()))),
        Some(Dialog::QuickStep(Box::new(crate::dialogs::QuickStepForm::new(&a.session)))),
        key.map(|key| Dialog::PicturePreview { key, index: 0, name: "colour-key.png".into() }),
        Some(Dialog::copy_to()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every kind of dialog is on the list (the audit's tour missed 28 of 66, and one added later
    /// would be missed too), and each on it opens on the sample.
    #[test]
    fn every_dialog_is_on_the_list() {
        let (mut a, _) = crate::tests::app("screens-list");
        let n = count(&mut a);
        let mut shown = std::collections::BTreeSet::new();
        for i in 0..n {
            let d = dialog(&mut a, i).unwrap_or_else(|| panic!("dialog {i} isn't built on the sample"));
            let name: String = format!("{d:?}").chars().take_while(char::is_ascii_alphanumeric).collect();
            shown.insert(name);
        }
        let source = include_str!("dialogs.rs");
        let body = source.split("pub enum Dialog {").nth(1).and_then(|b| b.split("\n}").next()).unwrap_or_default();
        let kinds: Vec<String> = body
            .lines()
            .filter_map(|l| l.strip_prefix("    "))
            .filter(|l| l.starts_with(|c: char| c.is_ascii_uppercase()))
            .map(|l| l.chars().take_while(char::is_ascii_alphanumeric).collect())
            .collect();
        assert!(kinds.len() > 60, "the dialogs read: {kinds:?}");
        let missing: Vec<&String> = kinds.iter().filter(|k| !shown.contains(*k)).collect();
        assert!(missing.is_empty(), "dialogs not on the list: {missing:?}");
    }
}
