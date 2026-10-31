//! Issue #29: who a message says it's from. A known person's name on an address that isn't
//! theirs, the sender's address in the message list, and the External tag.

use mailcraft_model::lookalike::SenderWarning;

use super::*;

/// Put a message in the sample Inbox (with `headers` added to its header) and open it.
fn deliver(s: &mut Session, n: u32, from: (&str, &str), subject: &str, headers: &str) -> mailcraft_model::MessageKey {
    let d = mailcraft_mime::Draft {
        to: vec![mailcraft_model::EmailAddress::new("Riley Moreno", "riley@studio.example")],
        subject: subject.into(),
        body: "Can you buy gift cards for the client today? I'm in a meeting.".into(),
        ..Default::default()
    };
    let raw = mailcraft_mime::build(&d, &mailcraft_model::EmailAddress::new(from.0, from.1), NOW, &format!("<sender{n}@x>")).unwrap();
    let raw = [headers.as_bytes(), &raw].concat();
    let store = s.store.clone().unwrap();
    let mut inbox = store.load_folder("sample", "INBOX").unwrap();
    let key = mailcraft_model::MessageKey { account: "sample".into(), folder: "INBOX".into(), validity: inbox.validity, uid: 9300 + n };
    inbox.messages.insert(0, mailcraft_mime::summary_from_full(&raw, key.clone(), mailcraft_model::Flags::default(), 0, NOW));
    store.save_folder("sample", "INBOX", &inbox).unwrap();
    store.write_body(&key, &raw).unwrap();
    s.apply(Event::FolderChanged { account: "sample".into(), folder: "INBOX".into() });
    s.run("folder.open", &json!({"account": "sample", "folder": "INBOX"})).unwrap();
    s.run("list.open", &json!({"key": key.to_id()})).unwrap();
    key
}

fn warning(s: &Session) -> Option<SenderWarning> {
    s.reading.as_ref().and_then(|r| r.warning.clone())
}

/// The list's row for `subject`, as `list.messages` gives it.
fn row(s: &mut Session, subject: &str) -> Value {
    let r = s.run("list.messages", &json!({"limit": 500})).unwrap();
    r["messages"].as_array().unwrap().iter().find(|m| m["subject"] == subject).cloned().unwrap_or_else(|| panic!("no {subject}"))
}

#[test]
fn a_known_persons_name_on_a_strange_address_is_pointed_out() {
    let mut s = sample_session("known-name");
    // Zoe Park is a contact (zoe@studio.example): her name on a free-mail address is caught, in
    // any order, case or accents.
    deliver(&mut s, 1, ("Zoe Park", "zoe.park.director@gmail.com"), "Urgent", "");
    assert_eq!(
        warning(&s),
        Some(SenderWarning::NameOfKnownPerson {
            name: "Zoe Park".into(),
            known: "zoe@studio.example".into(),
            real: "zoe.park.director@gmail.com".into()
        })
    );
    for (n, name) in [(2, "Park, Zoë"), (3, "ZOE PARK"), (4, "Zoe A. Park")] {
        deliver(&mut s, n, (name, "ceo.office@outlook.com"), "Urgent", "");
        assert!(matches!(warning(&s), Some(SenderWarning::NameOfKnownPerson { .. })), "{name}");
    }
    // The user's own name, from somewhere else.
    deliver(&mut s, 5, ("Riley Moreno", "riley.moreno@gmail.com"), "Me", "");
    assert!(matches!(warning(&s), Some(SenderWarning::NameOfKnownPerson { known, .. }) if known == "riley@studio.example"));
    // The list shows the warning too.
    assert_eq!(row(&mut s, "Urgent")["senderWarning"], json!(true));
    // Someone who really moved: saved as a contact with the new address, the warning goes, in
    // the open message too.
    deliver(&mut s, 7, ("Zoe Park", "zoe.park@proton.me"), "New address", "");
    assert!(warning(&s).is_some());
    s.run("contact.fromMessage", &json!({})).unwrap();
    s.tick();
    assert_eq!(warning(&s), None);
    assert_eq!(row(&mut s, "New address")["senderWarning"], json!(false));
    // A mailing list's header doesn't excuse a free-mail address.
    deliver(&mut s, 6, ("Zoe Park", "zoe1984@gmail.com"), "Listed", "List-Id: <team.lists.example>\r\n");
    assert!(warning(&s).is_some());
    done(s);
}

#[test]
fn people_known_by_their_address_or_organisation_and_generic_names_pass() {
    let mut s = sample_session("known-name-passes");
    for (n, from, headers) in [
        // Her own address, and another at her organisation.
        (1, ("Zoe Park", "zoe@studio.example"), ""),
        (2, ("Zoe Park", "zoe.park@studio.example"), ""),
        // A first name alone, a role, someone unknown.
        (3, ("Zoe", "zoe@gmail.com"), ""),
        (4, ("Studio Support", "help@gmail.com"), ""),
        (5, ("Zoe Parker", "zp@gmail.com"), ""),
        // A mailing list sending her message from its own address.
        (6, ("Zoe Park", "animators@lists.example"), "List-Id: Animators <animators.lists.example>\r\n"),
        // A known address under another name (the print shop's).
        (7, ("Zoe Park", "orders@printshop.example"), ""),
    ] {
        deliver(&mut s, n, from, "Hello", headers);
        assert_eq!(warning(&s), None, "{from:?}");
    }
    done(s);
}

#[test]
fn people_found_in_the_directory_count_as_known() {
    let mut s = sample_session("known-directory");
    let id = s.run("account.add", &json!({"email": "riley@contoso.example", "password": "pw", "ews": "mail.contoso.example"})).unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    s.take_jobs();
    // Looked up as the user searched for her name (Select Names, a recipient being typed).
    s.look_up_directory("dana", true);
    s.take_jobs();
    s.apply(Event::Directory {
        account: id,
        query: "dana".into(),
        people: vec![("Dana Whitfield".into(), "dana.whitfield@contoso.example".into())],
        error: None,
        consent: false,
    });
    deliver(&mut s, 1, ("Whitfield, Dana", "dana.whitfield.ceo@gmail.com"), "Wire", "");
    assert!(matches!(warning(&s), Some(SenderWarning::NameOfKnownPerson { known, .. }) if known == "dana.whitfield@contoso.example"));
    done(s);
}

#[test]
fn the_list_shows_strangers_addresses() {
    let mut s = sample_session("sender-addresses");
    deliver(&mut s, 1, ("Pat Doe", "pat@unknown.example"), "From a stranger", "");
    deliver(&mut s, 2, ("Zoe Park", "zoe@studio.example"), "From a contact", "");
    deliver(&mut s, 3, ("", "nameless@unknown.example"), "No name", "");
    let shown = |s: &mut Session, subject: &str| row(s, subject)["fromAddress"].as_bool().unwrap();
    // At first: for senders the user doesn't know, only where there's a name to show it beside.
    assert!(shown(&mut s, "From a stranger"));
    assert!(!shown(&mut s, "From a contact"));
    assert!(!shown(&mut s, "No name"));
    s.run("view.senderAddresses", &json!({"show": "always"})).unwrap();
    assert!(shown(&mut s, "From a contact"));
    s.run("view.senderAddresses", &json!({"show": "NEVER"})).unwrap();
    assert!(!shown(&mut s, "From a stranger"));
    assert!(s.run("view.senderAddresses", &json!({"show": "sometimes"})).is_err());
    assert!(s.run("view.senderAddresses", &json!({})).is_err());
    assert_eq!(s.prefs.sender_addresses, crate::SenderAddresses::Never);
    // Someone the user writes to stops being a stranger.
    s.run("view.senderAddresses", &json!({"show": "strangers"})).unwrap();
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"to": "Pat Doe <pat@unknown.example>", "subject": "Hi Pat"})).unwrap();
    s.run("compose.send", &json!({"checked": true})).unwrap();
    s.run("folder.open", &json!({"account": "sample", "folder": "INBOX"})).unwrap();
    s.clock.now += 3600;
    s.apply(Event::FolderChanged { account: "sample".into(), folder: "Sent".into() });
    s.refresh_list();
    assert!(!shown(&mut s, "From a stranger"));
    done(s);
}

#[test]
fn the_external_tag_marks_mail_from_outside_the_organisation() {
    let mut s = sample_session("external");
    deliver(&mut s, 1, ("Pat Doe", "pat@unknown.example"), "Outside", "");
    deliver(&mut s, 2, ("Sam Okafor", "sam@studio.example"), "Inside", "");
    let external = |s: &mut Session, subject: &str| row(s, subject)["external"].as_bool().unwrap();
    // Off at first.
    assert!(!external(&mut s, "Outside"));
    let r = s.run("view.externalTag", &json!({"on": true})).unwrap();
    assert_eq!(r["organisation"], json!(["studio.example"]));
    assert!(external(&mut s, "Outside"));
    assert!(!external(&mut s, "Inside"));
    let outside = row(&mut s, "Outside")["key"].clone();
    s.run("list.open", &json!({"key": outside})).unwrap();
    assert_eq!(s.run("message.read", &json!({})).unwrap()["external"], json!(true));
    // The user's own list of domains.
    let r = s.run("view.externalTag", &json!({"domains": " @Unknown.Example., studio.example studio.example"})).unwrap();
    assert_eq!(r["domains"], json!(["unknown.example", "studio.example"]));
    assert!(!external(&mut s, "Outside"));
    // The user's own mail isn't external, even when their account's domain isn't listed.
    s.run("view.externalTag", &json!({"domains": "unknown.example"})).unwrap();
    assert!(!s.is_external("riley@studio.example"));
    assert!(s.is_external("sam@studio.example"));
    for bad in [json!({"domains": "not a domain!"}), json!({"domains": 5}), json!({"domains": [1]})] {
        assert!(s.run("view.externalTag", &bad).is_err(), "{bad}");
    }
    let many: Vec<String> = (0..101).map(|i| format!("d{i}.example")).collect();
    assert!(s.run("view.externalTag", &json!({"domains": many})).is_err());
    // An empty list goes back to the accounts' domains.
    let r = s.run("view.externalTag", &json!({"domains": []})).unwrap();
    assert_eq!(r["organisation"], json!(["studio.example"]));
    done(s);
}

#[test]
fn a_free_mail_user_has_no_organisation() {
    let mut s = Session::open(None, mailcraft_model::dates::Clock::new(NOW, 0));
    s.run("account.add", &json!({"email": "me@gmail.com", "password": "pw", "imapHost": "imap.gmail.com", "smtpHost": "smtp.gmail.com"})).unwrap();
    let r = s.run("view.externalTag", &json!({"on": true})).unwrap();
    assert_eq!(r["organisation"], json!([]));
    assert!(!s.is_external("anyone@gmail.com"));
    assert!(!s.is_external("anyone@else.example"));
}

#[test]
fn a_list_message_whose_entry_doesnt_say_so_is_known_by_its_own_header() {
    // An Exchange listing carries no `List-Id`, and a message opened in a window or from another
    // folder needn't be in the list: its own header says it came through a list.
    let mut s = sample_session("listed-by-header");
    let key = deliver(&mut s, 1, ("Zoe Park", "animators@lists.example"), "Listed", "List-Id: Animators <animators.lists.example>\r\n");
    let store = s.store.clone().unwrap();
    let mut inbox = store.load_folder("sample", "INBOX").unwrap();
    inbox.messages.iter_mut().filter(|m| m.key == key).for_each(|m| m.bulk = false);
    store.save_folder("sample", "INBOX", &inbox).unwrap();
    s.apply(Event::FolderChanged { account: "sample".into(), folder: "INBOX".into() });
    s.run("list.open", &json!({"key": key.to_id()})).unwrap();
    assert!(!s.summary(&key).unwrap().bulk);
    assert_eq!(warning(&s), None);
    // Gathered again (a contact saved), it still says so.
    s.run("contact.create", &json!({"name": "Pat Doe", "emails": "pat@unknown.example"})).unwrap();
    s.tick();
    assert_eq!(warning(&s), None);
    done(s);
}

#[test]
fn a_calendar_syncing_doesnt_count_the_address_book_again() {
    // Every calendar or contacts sync drops the address book; counting it again reads every
    // cached folder, which the window's every frame mustn't do each time one comes in.
    let mut s = sample_session("senders-cheap");
    deliver(&mut s, 1, ("Zoe Park", "zoe.park.director@gmail.com"), "Urgent", "");
    s.tick();
    let counted = s.contacts_built;
    for _ in 0..5 {
        s.save_pim("sample");
        s.tick();
    }
    assert_eq!(s.contacts_built, counted);
    // The people written to are still known, and the warning stands.
    assert!(warning(&s).is_some());
    s.run("view.senderAddresses", &json!({"show": "strangers"})).unwrap();
    assert_eq!(row(&mut s, "Urgent")["senderWarning"], json!(true));
    // Old, it's counted again as the list is refreshed.
    s.clock.now += 3600;
    s.refresh_list();
    assert_ne!(s.contacts_built, counted);
    done(s);
}
