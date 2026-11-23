//! Issue #245: an organisation's profile in the session. Its defaults go in once and stay as the
//! person leaves them, its locks hold through anything, a command that would change a locked
//! setting is refused, and its rules for accounts hold whichever way an account comes. Without a
//! profile, nothing changes.

use mailcraft_model::{Job, Security};

use super::*;
use crate::policy::Organisation;

fn org(v: Value) -> Organisation {
    Organisation::from_sources(vec![("test".into(), v, true)])
}

#[test]
fn defaults_go_in_once_and_locks_hold() {
    let mut s = sample_session("org-settings");
    let profile = json!({
        "settings": {"compact": true, "plainTopBar": {"value": true, "locked": true},
                     "noSuchSetting": 1, "lastOpen": {"value": null, "locked": true}, "undoSendSecs": "soon"}
    });
    s.set_organisation(org(profile.clone()));
    assert!(s.prefs.compact && s.prefs.plain_top_bar);
    let notes = s.org.notes.join("\n");
    for bit in ["noSuchSetting", "lastOpen", "undoSendSecs"] {
        assert!(notes.contains(bit), "{bit}: {notes}");
    }
    // A default is the person's to change, and stays changed when MailCraft starts again.
    s.run("view.compact", &json!({"on": false})).unwrap();
    s.set_organisation(org(profile.clone()));
    assert!(!s.prefs.compact);
    // A new default from the organisation is put in once more.
    let mut changed = profile.clone();
    changed["settings"]["compact"] = json!({"value": false});
    s.set_organisation(org(changed));
    s.run("view.compact", &json!({"on": true})).unwrap();
    assert!(s.prefs.compact);
    // A locked setting's command is refused, said; changed any other way, it's put back.
    let refused = s.run("view.plainTopBar", &json!({"on": false})).unwrap_err().to_string();
    assert!(refused.contains("Set by your organisation"), "{refused}");
    assert_eq!(s.command_locked("view.plainTopBar"), Some("Set by your organisation"));
    assert_eq!(s.command_locked("view.compact"), None);
    s.prefs.plain_top_bar = false;
    s.run("view.compact", &json!({})).unwrap();
    assert!(s.prefs.plain_top_bar, "held after the next command");
}

#[test]
fn no_profile_changes_nothing() {
    let mut s = sample_session("org-none");
    let before = s.prefs.clone();
    s.set_organisation(Organisation::default());
    assert_eq!(s.prefs, before);
    assert!(s.org.held.is_empty() && s.command_locked("view.plainTopBar").is_none());
    s.run("account.add", &json!({"email": "riley@anywhere.example", "password": "pw", "imapSecurity": "none", "smtpSecurity": "none"})).unwrap();
    assert!(matches!(s.take_jobs().as_slice(), [Job::Start { .. }]), "jobs leave as they are");
    s.run("account.discover", &json!({"email": "zoe@else.example"})).unwrap();
    let jobs = s.take_jobs();
    let [Job::Discover { rules, .. }] = jobs.as_slice() else { panic!("{jobs:?}") };
    assert_eq!(*rules, mailcraft_model::DiscoverRules::default());
}

#[test]
fn the_organisations_servers_and_domains() {
    let mut s = sample_session("org-accounts");
    s.set_organisation(org(json!({
        "accounts": {
            "onlyOrganisationDomains": true,
            "servers": {"contoso.example": {"imap": "mail.contoso.example:993:tls", "smtp": "mail.contoso.example:587:starttls",
                                            "caldav": "https://dav.contoso.example/cal/"}},
            "lookups": {"thunderbird": false},
            "autoconfig": "https://config.contoso.example/mail.xml"
        }
    })));
    // Found at once, from the profile: nothing asked of the network.
    let found = s.run("account.discover", &json!({"email": "Riley@Contoso.example"})).unwrap();
    assert_eq!((found["found"].as_bool(), found["source"].as_str()), (Some(true), Some("your organisation")));
    assert_eq!(found["smtp"]["port"], 587);
    assert!(s.take_jobs().is_empty());
    // An address only its servers name: an address and a password.
    let id = s.run("account.add", &json!({"email": "riley@contoso.example", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    let c = &s.account(&id).unwrap().config;
    assert_eq!((c.imap.host.as_str(), c.smtp.port, c.smtp.security), ("mail.contoso.example", 587, Security::StartTls));
    assert_eq!(c.dav.as_ref().map(|d| d.caldav.as_str()), Some("https://dav.contoso.example/cal/"));
    // Servers given by hand win.
    let id2 =
        s.run("account.add", &json!({"email": "ops@contoso.example", "password": "pw", "imapHost": "other.contoso.example"})).unwrap()["account"]
            .as_str()
            .unwrap()
            .to_string();
    assert_eq!(s.account(&id2).unwrap().config.imap.host, "other.contoso.example");
    // Another domain's account isn't allowed; its look-up goes the ways allowed.
    let refused = s.run("account.add", &json!({"email": "me@gmail.example", "password": "pw"})).unwrap_err().to_string();
    assert!(refused.contains("contoso.example"), "{refused}");
    // Nor can one of its accounts be moved to another domain.
    let moved = s.run("account.update", &json!({"account": id, "email": "riley@gmail.example"})).unwrap_err().to_string();
    assert!(moved.contains("contoso.example"), "{moved}");
    assert_eq!(s.account(&id).unwrap().config.email, "riley@contoso.example");
    s.take_jobs();
    s.run("account.discover", &json!({"email": "zoe@else.example"})).unwrap();
    let jobs = s.take_jobs();
    let [Job::Discover { rules, .. }] = jobs.as_slice() else { panic!("{jobs:?}") };
    assert!(!rules.thunderbird && rules.domain && rules.autoconfig.as_deref() == Some("https://config.contoso.example/mail.xml"));
}

#[test]
fn encrypted_connections_only_and_no_certificate_exceptions() {
    let mut s = sample_session("org-security");
    // Added before the organisation's profile: its server takes passwords in the clear.
    let id = s
        .run("account.add", &json!({"email": "riley@lab.example", "password": "pw", "imapSecurity": "none", "smtpSecurity": "starttls"}))
        .unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    s.take_jobs();
    s.run("account.trustCertificate", &json!({"account": id, "fingerprint": "ab".repeat(32)})).unwrap();
    s.take_jobs();
    s.set_organisation(org(json!({"requireEncryption": true, "certificateExceptions": false})));
    // Now: held back, its status saying why; no change to it that keeps it unencrypted.
    let config = s.account(&id).unwrap().config.clone();
    s.push_job(Job::Start { account: config.clone(), password: None });
    assert!(s.take_jobs().is_empty());
    let status = format!("{:?}", s.account(&id).unwrap().status);
    assert!(status.contains("encrypted"), "{status}");
    assert!(s.run("account.update", &json!({"account": id, "imapPort": 144})).is_err());
    assert!(s.run("account.add", &json!({"email": "zoe@lab.example", "password": "pw", "imapSecurity": "none"})).is_err());
    assert!(
        s.run("account.add", &json!({"email": "ews@lab.example", "password": "pw", "ews": "http://exchange.lab.example/EWS/Exchange.asmx"})).is_err()
    );
    // Encrypted again, it starts, without the certificate trusted by hand.
    s.run("account.update", &json!({"account": id, "imapSecurity": "tls", "imapPort": 993})).unwrap();
    let jobs = s.take_jobs();
    let [Job::Start { account, .. }] = jobs.as_slice() else { panic!("{jobs:?}") };
    assert!(account.trusted_certificates.is_empty() && !s.account(&id).unwrap().config.trusted_certificates.is_empty());
    let refused = s.run("account.trustCertificate", &json!({"account": id, "fingerprint": "cd".repeat(32)})).unwrap_err().to_string();
    assert!(refused.contains("certificate"), "{refused}");
    // A calendar server in the clear is refused, the account left as it was.
    s.take_jobs();
    if let Some(a) = s.accounts.iter_mut().find(|a| a.config.id == id) {
        a.status = mailcraft_model::SyncStatus::default();
    }
    let dav = json!({"account": id, "caldav": "http://dav.lab.example/", "username": "riley", "password": "pw"});
    assert!(s.run("account.dav", &dav).is_err());
    assert!(s.take_jobs().is_empty());
    assert_eq!(s.account(&id).unwrap().status, mailcraft_model::SyncStatus::default());
    assert!(s.run("calendar.subscribe", &json!({"url": "http://lab.example/holidays.ics"})).is_err());
}

#[test]
fn pictures_locked_off_load_for_no_sender() {
    let mut s = sample_session("org-pictures");
    let key = "sample:1:4:INBOX";
    s.prefs.trusted_senders.insert("ana@soundlab.example".into());
    s.prefs.safe_senders.insert("ana@soundlab.example".into());
    s.run("list.open", &json!({"key": key})).unwrap();
    assert!(s.pictures_allowed(), "a trusted sender's load by themselves");
    s.set_organisation(org(json!({"settings": {"autoPictures": {"value": false, "locked": true}}})));
    s.run("list.open", &json!({"key": key})).unwrap();
    assert!(!s.pictures_allowed(), "no sender's, trusted or safe");
    // Asked for this message, they load; never for the sender from then on.
    s.run("message.showPictures", &json!({"always": true})).unwrap();
    assert!(s.pictures_allowed());
    s.prefs.trusted_senders.clear();
    s.run("message.showPictures", &json!({"always": true})).unwrap();
    assert!(s.prefs.trusted_senders.is_empty());
    assert!(s.run("view.trustSender", &json!({"address": "zoe@studio.example"})).is_err());
}

#[test]
fn the_organisations_signature_and_disclaimer() {
    let add = json!({"email": "riley@studio.example", "password": "pw", "name": "Riley Moreno", "imapHost": "mail.studio.example", "smtpHost": "mail.studio.example"});
    let (mut s, id) = server_account("org-signature", add, &[("INBOX", FolderRole::Inbox), ("Sent", FolderRole::Sent)]);
    s.take_jobs();
    s.set_organisation(org(json!({"profiles": [{"domains": ["studio.example"],
        "signature": {"name": "Studio", "text": "{name}\n{title}\nPhone: {phone}", "html": "<p><b>{name}</b> · {title}</p>", "locked": true},
        "disclaimer": {"text": "This message is confidential."}}]})));
    let c = s.account(&id).unwrap().config.clone();
    // Filled from what's known: the account's name; lines whose details are missing go.
    assert_eq!(c.signature_named("Studio").map(|g| g.text.as_str()), Some("Riley Moreno"));
    assert_eq!((c.signature_new.as_deref(), c.signature_reply.as_deref()), (Some("Studio"), Some("Studio")));
    // The person's details fill the rest.
    s.run("account.details", &json!({"account": id, "title": "Editor", "phone": "+44 20 7946 0000", "office": 3})).unwrap_err();
    s.run("account.details", &json!({"account": id, "title": "Editor", "phone": "+44 20 7946 0000"})).unwrap();
    let c = s.account(&id).unwrap().config.clone();
    let sig = c.signature_named("Studio").unwrap();
    assert_eq!(sig.text, "Riley Moreno\nEditor\nPhone: +44 20 7946 0000");
    assert!(sig.code.contains("<b>Riley Moreno</b>") && sig.code.contains("Editor"), "{}", sig.code);
    // Locked: as the organisation wrote it.
    for (cmd, params) in [
        ("account.setSignature", json!({"account": id, "name": "Studio", "signature": "mine"})),
        ("account.removeSignature", json!({"account": id, "name": "Studio"})),
        ("account.signatureDefaults", json!({"account": id, "new": null})),
    ] {
        let why = s.run(cmd, &params).unwrap_err().to_string();
        assert!(why.contains("Set by your organisation"), "{cmd}: {why}");
    }
    s.run("account.setSignature", &json!({"account": id, "name": "Mine", "signature": "R."})).unwrap();
    // A new message has it, and what's sent has the disclaimer under it.
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"to": "ana@soundlab.example", "subject": "Mix notes"})).unwrap();
    s.run("compose.send", &json!({})).unwrap();
    let jobs = s.take_jobs();
    let Some(Job::Send { raw, .. }) = jobs.iter().find(|j| matches!(j, Job::Send { .. })) else { panic!("{jobs:?}") };
    let sent = String::from_utf8_lossy(raw);
    assert!(sent.contains("Editor") && sent.contains("This message is confidential."), "{sent}");
}

#[test]
fn an_unlocked_signature_is_the_persons_once_put_in() {
    let add =
        json!({"email": "zoe@studio.example", "password": "pw", "name": "Zoe", "imapHost": "mail.studio.example", "smtpHost": "mail.studio.example"});
    let (mut s, id) = server_account("org-signature-free", add, &[("INBOX", FolderRole::Inbox)]);
    let profile = json!({"signature": {"name": "Studio", "text": "{name}, Studio"}});
    s.set_organisation(org(profile.clone()));
    s.run("account.signatureDefaults", &json!({"account": id, "new": null})).unwrap();
    s.run("account.removeSignature", &json!({"account": id, "name": "Studio"})).unwrap();
    // MailCraft starting again: as the person left it.
    s.set_organisation(org(profile));
    let c = s.account(&id).unwrap().config.clone();
    assert!(c.signature_named("Studio").is_none() && c.signature_new.is_none());
    // The organisation's new one is put in once more.
    s.set_organisation(org(json!({"signature": {"name": "Studio", "text": "{name}, Studio Ltd"}})));
    assert_eq!(s.account(&id).unwrap().config.signature_named("Studio").map(|g| g.text.as_str()), Some("Zoe, Studio Ltd"));
}

/// How many lines of a message's text are the disclaimer.
fn disclaimers(raw: &[u8], disclaimer: &str) -> usize {
    mailcraft_mime::parse(raw).map(|p| p.text.lines().filter(|l| l.trim() == disclaimer).count()).unwrap_or(0)
}

#[test]
fn the_disclaimer_goes_once_under_everything_sent() {
    const DISCLAIMER: &str = "Confidential to Studio Example.";
    let add =
        json!({"email": "ana@studio.example", "password": "pw", "name": "Ana", "imapHost": "mail.studio.example", "smtpHost": "mail.studio.example"});
    let (mut s, id) = server_account("org-disclaimer", add, &[("INBOX", FolderRole::Inbox), ("Sent", FolderRole::Sent)]);
    s.set_organisation(org(json!({"disclaimer": {"text": DISCLAIMER}})));
    s.take_jobs();
    let send = |s: &mut Session| {
        s.take_jobs().into_iter().find_map(|j| match j {
            Job::Send { id, raw, from, recipients, .. } => Some((id, raw, from, recipients)),
            _ => None,
        })
    };
    // Under a message whose quote has it already: theirs quoted, and ours.
    s.run("mail.new", &json!({})).unwrap();
    s.run(
        "compose.update",
        &json!({"to": "ann@soundlab.example", "subject": "Plans", "body": "See you\n\n> Earlier\n> Confidential to Studio Example."}),
    )
    .unwrap();
    s.run("compose.send", &json!({})).unwrap();
    let (send_id, raw, from, recipients) = send(&mut s).unwrap();
    assert_eq!(disclaimers(&raw, DISCLAIMER), 1, "{}", String::from_utf8_lossy(&raw));
    // Held in the Outbox, opened again and sent: once.
    let store = s.store.clone().unwrap();
    let item = mailcraft_store::OutboxItem { id: send_id, from, recipients, queued: NOW, error: Some("offline".into()), ..Default::default() };
    store.outbox_add(&id, &item, &raw).unwrap();
    s.apply(Event::Queued { account: id.clone(), id: send_id, error: "offline".into(), kind: Default::default() });
    s.run("folder.outbox", &json!({})).unwrap();
    let key = s.list[0].key.clone();
    s.run("outbox.edit", &json!({"key": key.to_id()})).unwrap();
    assert_eq!(s.compose.as_ref().unwrap().body, "See you\n\n> Earlier\n> Confidential to Studio Example.", "as it was written");
    s.run("compose.send", &json!({})).unwrap();
    let (_, again, ..) = send(&mut s).unwrap();
    assert_eq!(disclaimers(&again, DISCLAIMER), 1, "{}", String::from_utf8_lossy(&again));
    // A meeting's invitation has it too.
    s.run("event.create", &json!({"account": id, "summary": "Planning", "start": "2026-11-10T10:00", "attendees": "Ann <ann@soundlab.example>"}))
        .unwrap();
    let (_, invite, ..) = send(&mut s).expect("the invitation");
    assert_eq!(disclaimers(&invite, DISCLAIMER), 1, "{}", String::from_utf8_lossy(&invite));
}

#[test]
fn an_unlocked_signature_follows_the_details_until_changed() {
    let add =
        json!({"email": "ana@studio.example", "password": "pw", "name": "Ana", "imapHost": "mail.studio.example", "smtpHost": "mail.studio.example"});
    let (mut s, id) = server_account("org-signature-details", add, &[("INBOX", FolderRole::Inbox)]);
    let profile = json!({"signature": {"name": "Studio", "text": "{name}\n{title}"}});
    s.set_organisation(org(profile.clone()));
    let text = |s: &Session| s.account(&id).and_then(|a| a.config.signature_named("Studio")).map(|g| g.text.clone());
    assert_eq!(text(&s).as_deref(), Some("Ana"));
    // Still as it was filled in: their new details fill it again.
    s.run("account.details", &json!({"account": id, "title": "Producer"})).unwrap();
    assert_eq!(text(&s).as_deref(), Some("Ana\nProducer"));
    // Changed by them: theirs, whatever their details or name become, MailCraft starting again too.
    s.run("account.setSignature", &json!({"account": id, "name": "Studio", "signature": "Ana, Studio sound"})).unwrap();
    s.run("account.details", &json!({"account": id, "title": "Head of Sound"})).unwrap();
    s.run("account.update", &json!({"account": id, "name": "Ana Silva"})).unwrap();
    s.set_organisation(org(profile));
    assert_eq!(text(&s).as_deref(), Some("Ana, Studio sound"));
}

/// The profile `docs/organisations.md` shows is taken whole: nothing of it left out.
#[test]
fn the_documented_profile_is_taken_whole() {
    // As checked out: with Windows line endings too.
    let doc = include_str!("../../../../docs/organisations.md").replace("\r\n", "\n");
    let json = doc.split("```json\n").nth(1).and_then(|r| r.split("\n```").next()).expect("the docs' example");
    let v: Value = serde_json::from_str(json).expect("the docs' example is JSON");
    let mut s = sample_session("org-docs");
    s.set_organisation(org(v));
    assert!(s.organisation.ignored.is_empty() && s.org.notes.is_empty(), "{:?} {:?}", s.organisation.ignored, s.org.notes);
    assert!(s.org.require_encryption && s.org.no_certificate_exceptions && s.setting_locked("plainTopBar"));
    assert_eq!(s.organisation.profiles.len(), 1);
}

/// The directory (Exchange) fills the details an organisation's signature names (issue #245),
/// asked once the account is online: as it's added, and as MailCraft starts, it isn't yet, and
/// a directory asked then only answers that it's offline (found on a real grommunio).
#[test]
fn the_directory_fills_the_details_once_online() {
    let add = json!({"email": "ana@studio.example", "password": "pw", "name": "Ana", "ews": "https://mail.studio.example/EWS/Exchange.asmx"});
    let (mut s, id) = server_account("org-directory-details", add, &[("INBOX", FolderRole::Inbox)]);
    s.take_jobs();
    s.set_organisation(org(json!({"signature": {"name": "Studio", "text": "{name}\n{title}, {department}"}})));
    let asked = |s: &mut Session| s.take_jobs().into_iter().filter(|j| matches!(j, Job::SearchDirectory { .. })).count();
    assert_eq!(asked(&mut s), 0, "not asked offline");
    let online = |s: &mut Session| s.apply(Event::Status { account: id.clone(), status: mailcraft_model::SyncStatus::Online { at: NOW } });
    online(&mut s);
    assert_eq!(asked(&mut s), 1, "asked once online");
    // A failure is asked again as it comes online next; an answer isn't.
    s.apply(Event::Directory {
        account: id.clone(),
        query: "ana@studio.example".into(),
        people: vec![],
        error: Some("refused".into()),
        consent: false,
    });
    online(&mut s);
    assert_eq!(asked(&mut s), 1, "asked again after a failure");
    let ana = mailcraft_model::DirectoryPerson {
        name: "Ana Silva".into(),
        email: "Ana@studio.example".into(),
        title: "Sound editor".into(),
        department: "Post production".into(),
    };
    s.apply(Event::Directory { account: id.clone(), query: "ana@studio.example".into(), people: vec![ana], error: None, consent: false });
    let text = s.account(&id).and_then(|a| a.config.signature_named("Studio")).map(|g| g.text.clone());
    // None of them given in File › Info: the directory's name, title and department.
    assert_eq!(text.as_deref(), Some("Ana Silva\nSound editor, Post production"));
    online(&mut s);
    assert_eq!(asked(&mut s), 0, "answered: not asked again");
}

/// A locked setting's command still reads it, and sets it to what it's locked to (the CLI and
/// AI agents ask what it is so); only a change is refused (found on real Linux and macOS).
#[test]
fn a_locked_setting_is_read_not_changed() {
    let mut s = sample_session("org-read-locked");
    s.set_organisation(org(json!({"updates": {"value": "off", "locked": true}, "settings": {"undoSendSecs": {"value": 10, "locked": true}}})));
    assert_eq!(s.run("options.undoSend", &json!({})).unwrap()["seconds"], 10);
    assert_eq!(s.run("options.updates", &json!({"value": "off"})).unwrap()["value"], "off");
    let refused = s.run("options.undoSend", &json!({"seconds": 30})).unwrap_err().to_string();
    assert!(refused.contains("Set by your organisation"), "{refused}");
    assert_eq!(s.prefs.undo_send_secs, 10);
    assert!(s.run("options.updates", &json!({"value": "on"})).is_err());
    assert!(s.command_locked("options.undoSend").is_some(), "its control is still held");
}
