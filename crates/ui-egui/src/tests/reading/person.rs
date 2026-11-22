//! UI tests: everything with one person (issue #178), from a name above a message and from a
//! message in the list.

use crate::Dialog;
use crate::dialogs::PersonTab;

use super::*;

const SIZE: egui::Vec2 = egui::vec2(1280.0, 800.0);

/// What the frame shows, as a screen reader is told it: the named nodes on screen.
fn shown(ctx: &egui::Context, a: &mut MailApp) -> Vec<Named> {
    // A dialog just opened (or the window resized) is measured in a frame, drawn in the next.
    sized_frame(ctx, a, SIZE, vec![]);
    named(ctx, a, SIZE).into_iter().filter(|n| n.rect.is_some() && n.name().is_some()).collect()
}

/// The texts the frame draws ([`shown`]'s two frames).
fn texts(ctx: &egui::Context, a: &mut MailApp) -> Vec<String> {
    for _ in 0..2 {
        sized_frame(ctx, a, SIZE, vec![]);
    }
    drawn_strings(ctx, a, SIZE)
}

/// Where the first node named `name` (or starting with it, `prefix`) is.
fn at(nodes: &[Named], name: &str, prefix: bool) -> egui::Pos2 {
    nodes
        .iter()
        .find(|n| n.name().is_some_and(|x| if prefix { x.starts_with(name) } else { x == name }))
        .and_then(Named::centre)
        .unwrap_or_else(|| panic!("no {name}: {:?}", nodes.iter().filter_map(Named::name).collect::<Vec<_>>()))
}

fn person(a: &MailApp) -> &crate::dialogs::PersonForm {
    match &a.ui.dialog {
        Some(Dialog::Person(f)) => f,
        other => panic!("not the person dialog: {other:?}"),
    }
}

fn open(a: &mut MailApp, subject: &str) -> MessageKey {
    let k = a.session.list.iter().find(|m| m.subject == subject).map(|m| m.key.clone()).unwrap();
    a.run("list.open", json!({"key": k.to_id()}));
    k
}

/// A click on Zoe's name above her message, then on "Everything with them…" in her card: her
/// mail, files, meetings and the people beside her, each part a click away; a file opens with
/// its message in a window of its own; the search narrows every part; someone beside her shows
/// everything with them; her meeting opens.
#[test]
fn everything_with_someone_from_their_card() {
    let (mut a, _) = app("person-card");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let storyboard = open(&mut a, "Storyboard review on Thursday");
    for _ in 0..2 {
        frame(&ctx, &mut a, vec![]);
    }
    let nodes = shown(&ctx, &mut a);
    click(&ctx, &mut a, at(&nodes, "Zoe Park", false));
    click_named(&ctx, &mut a, "Everything with them…");
    assert_eq!(person(&a).all.name, "Zoe Park");
    // A few of each, first; each conversation's folder.
    assert_eq!(person(&a).tab, PersonTab::Overview);
    let nodes = shown(&ctx, &mut a);
    for part in ["Overview", "Mail (3)", "Files (1)", "Meetings (1)", "Links (0)", "People (1)"] {
        at(&nodes, part, false);
    }
    at(&nodes, "Kick-off notes, Archive · ", true);
    at(&nodes, "colour-key.png, 268 B · ", true);
    // Her mail by conversation: the Storyboard one with Sam's answer in the Inbox, the kick-off
    // in the Archive.
    click_named(&ctx, &mut a, "Mail (3)");
    assert_eq!(person(&a).tab, PersonTab::Mail);
    let nodes = shown(&ctx, &mut a);
    at(&nodes, "Storyboard review on Thursday, 2 messages", false);
    at(&nodes, "Sam Okafor, Inbox · ", true);
    at(&nodes, "Kick-off notes", true);

    // Her file: opened, its message in a window of its own; this dialog stays.
    click_named(&ctx, &mut a, "Files (1)");
    assert_eq!(person(&a).tab, PersonTab::Files);
    let nodes = shown(&ctx, &mut a);
    click(&ctx, &mut a, at(&nodes, "colour-key.png, 268 B · ", true));
    assert!(a.session.windows.iter().any(|w| w.key == storyboard), "the file's message in a window of its own");
    assert!(matches!(a.ui.dialog, Some(Dialog::Person(_))));
    // (The test draws that window over the main one: closed, so it's out of the way.)
    a.session.close_window(&storyboard);

    // Searched within: each part narrowed.
    ctx.memory_mut(|m| m.request_focus(crate::dialogs::person_search_id()));
    frame(&ctx, &mut a, vec![Event::Text("kick-off".into())]);
    assert_eq!(person(&a).query, "kick-off");
    let nodes = shown(&ctx, &mut a);
    for part in ["Mail (1)", "Files (0)", "Meetings (0)", "People (0)"] {
        at(&nodes, part, false);
    }
    click_named(&ctx, &mut a, "Files (0)");
    assert!(has(&texts(&ctx, &mut a), "Nothing here matches the search."));

    // Sam, beside her: everything with him.
    ctx.memory_mut(|m| m.request_focus(crate::dialogs::person_search_id()));
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![key(Key::A, Modifiers::COMMAND), key(Key::Backspace, Modifiers::NONE)]);
    assert_eq!(person(&a).query, "");
    click_named(&ctx, &mut a, "People (1)");
    let nodes = shown(&ctx, &mut a);
    click(&ctx, &mut a, at(&nodes, "Sam Okafor <sam@studio.example>", true));
    assert_eq!(person(&a).all.email, "sam@studio.example");

    // His meeting opens (his view measured first, then drawn).
    shown(&ctx, &mut a);
    click_named(&ctx, &mut a, "Meetings (1)");
    let nodes = shown(&ctx, &mut a);
    click(&ctx, &mut a, at(&nodes, "Storyboard review, ", true));
    assert!(matches!(a.ui.dialog, Some(Dialog::Event(_))), "{:?}", a.ui.dialog.as_ref().map(std::mem::discriminant));
}

/// Someone else found from the box at its top, by a part of their name; and from the People
/// pane, the person shown there.
#[test]
fn someone_else_from_the_box_and_from_people() {
    let (mut a, _) = app("person-lookup");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    a.act(crate::Action::Person("zoe@studio.example".into()));
    assert!(a.ui.focus_main, "the main window forward, where it shows (asked for from a message window)");
    shown(&ctx, &mut a);
    ctx.memory_mut(|m| m.request_focus(crate::dialogs::person_lookup_id()));
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![Event::Text("watan".into())]);
    let found: Vec<&str> = person(&a).people_found.iter().map(|c| c.address.email.as_str()).collect();
    assert_eq!(found, ["jun@studio.example"]);
    let nodes = shown(&ctx, &mut a);
    click(&ctx, &mut a, at(&nodes, "Jun Watanabe <jun@studio.example>", false));
    let f = person(&a);
    assert_eq!((f.all.name.as_str(), f.lookup.as_str()), ("Jun Watanabe", ""), "a new look, the box empty");
    assert_eq!(f.all.threads.first().map(|t| t.subject.as_str()), Some("Character turnarounds"));

    // Nobody: said.
    shown(&ctx, &mut a);
    ctx.memory_mut(|m| m.request_focus(crate::dialogs::person_lookup_id()));
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![Event::Text("zzqx".into())]);
    assert!(has(&texts(&ctx, &mut a), "No one found."));

    // The People pane: the person picked, everything with them.
    a.ui.dialog = None;
    a.run("view.module", json!({"name": "people"}));
    a.ui.person = Some("ana@sample.mailcraft".into());
    shown(&ctx, &mut a);
    click_named(&ctx, &mut a, "Everything with them…");
    assert_eq!(person(&a).all.email, "ana@soundlab.example");
}

/// Find related on a message in the list: everything with its sender.
#[test]
fn everything_with_the_sender_from_find_related() {
    let (mut a, _) = app("person-related");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    for _ in 0..2 {
        frame(&ctx, &mut a, vec![]);
    }
    let nodes = shown(&ctx, &mut a);
    let row = at(&nodes, "Ana Lima, Sound mix", true);
    click_at(&ctx, &mut a, row, egui::PointerButton::Secondary, Modifiers::NONE);
    // Its submenu opens under the pointer.
    let related = at(&shown(&ctx, &mut a), "Find related", false);
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![Event::PointerMoved(related)]);
    }
    click_named(&ctx, &mut a, "Everything with sender…");
    let f = person(&a);
    assert_eq!((f.all.name.as_str(), f.all.message_count()), ("Ana Lima", 1));
    assert_eq!(f.all.meetings.len(), 1, "the playback she organises");
}

/// What a signature gives is offered for their contact, never put in by itself: a number in a
/// new contact's first place, or the first empty place of the contact they have.
#[test]
fn a_signatures_number_is_offered_for_their_contact() {
    let (mut a, _) = app("person-signature");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let raw = b"From: Pat Example <pat@far.example>\r\nTo: riley@studio.example\r\nSubject: The cut\r\nDate: Mon, 16 Nov 2026 10:00:00 +0000\r\nMessage-ID: <cut@far.example>\r\n\r\nHere it is.\r\n\r\n-- \r\nPat Example\r\nTel +44 20 7946 0958\r\nwww.pat.example\r\n";
    with_raw(&mut a, 9960, raw);
    a.act(crate::Action::Person("pat@far.example".into()));
    assert_eq!(person(&a).all.found.phones, ["+44 20 7946 0958"]);
    let texts = texts(&ctx, &mut a);
    for what in ["In their signature", "+44 20 7946 0958", "www.pat.example"] {
        assert!(has(&texts, what), "{what}: {texts:?}");
    }
    assert!(a.session.contact_by_address("pat@far.example").is_none(), "nothing saved by itself");
    click_named(&ctx, &mut a, "Add to their contact…");
    let Some(Dialog::Contact(f)) = &a.ui.dialog else { panic!("no contact form") };
    assert_eq!((f.uid.as_deref(), f.emails[0].as_str(), f.phones[0].as_str()), (None, "pat@far.example", "+44 20 7946 0958"));

    // Zoe has a contact, her mobile in it: a number from her signature goes in the next place.
    let raw = b"From: Zoe Park <zoe@studio.example>\r\nTo: riley@studio.example\r\nSubject: Desk line\r\nDate: Mon, 16 Nov 2026 11:00:00 +0000\r\nMessage-ID: <desk@studio.example>\r\n\r\nNew desk.\r\n\r\nZoe Park\r\nDesk: 020 7946 0018\r\nMobile: +1 555 0142\r\n";
    with_raw(&mut a, 9961, raw);
    a.act(crate::Action::Person("zoe@studio.example".into()));
    assert_eq!(person(&a).all.found.phones, ["020 7946 0018"], "not the mobile her contact has");
    shown(&ctx, &mut a);
    click_named(&ctx, &mut a, "Add to their contact…");
    let Some(Dialog::Contact(f)) = &a.ui.dialog else { panic!("no contact form") };
    assert!(f.uid.is_some(), "her own contact");
    assert_eq!(f.phones[0], "020 7946 0018");
    assert_eq!(f.phones[2], "+1 555 0142", "her mobile where it was");
}
