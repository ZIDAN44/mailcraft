use super::*;
use crate::RibbonTab;
use crate::tests::{app, frame, key};

fn alt(pressed: bool) -> Event {
    Event::Key { key: Key::AltLeft, physical_key: None, pressed, repeat: false, modifiers: if pressed { Modifiers::ALT } else { Modifiers::NONE } }
}

/// `events` given to the key tips alone, on Windows: what they ask, and what they leave.
fn run(tips: &mut KeyTips, events: Vec<Event>) -> (Vec<Act>, Vec<Event>) {
    let ctx = Context::default();
    ctx.set_os(OperatingSystem::Windows);
    let (mut acts, mut left) = (Vec::new(), Vec::new());
    let mut out = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| {
        acts = tips.process(ui.ctx(), true);
        left = ui.input(|i| i.events.clone());
    });
    out.textures_delta.clear();
    (acts, left)
}

/// Alt (or F10) shows them; a tab's letters open it; a button's press it; Escape goes back a
/// letter, then hides them, as in Outlook.
#[test]
fn letters_walk_from_a_tab_to_its_buttons() {
    let mut tips = KeyTips::default();
    run(&mut tips, vec![alt(true), alt(false)]);
    assert_eq!(tips.prefix(), Some(""));
    let (acts, left) = run(&mut tips, vec![key(Key::H, Modifiers::NONE), Event::Text("h".into())]);
    assert_eq!(acts, vec![Act::Tab(RibbonTab::Home)]);
    assert!(left.is_empty(), "the letter is the key tips', not typed: {left:?}");
    assert_eq!(run(&mut tips, vec![key(Key::R, Modifiers::NONE)]).0, vec![]);
    assert_eq!(tips.prefix(), Some("HR"));
    assert_eq!(run(&mut tips, vec![key(Key::P, Modifiers::NONE)]).0, vec![Act::Press("HRP".into())]);
    assert_eq!(tips.prefix(), None);
    // Send / Receive is J then S; a letter that reaches nothing changes nothing.
    run(&mut tips, vec![key(Key::F10, Modifiers::NONE)]);
    assert_eq!(run(&mut tips, vec![key(Key::Z, Modifiers::NONE)]).0, vec![]);
    assert_eq!(tips.prefix(), Some(""));
    run(&mut tips, vec![key(Key::J, Modifiers::NONE)]);
    assert_eq!(run(&mut tips, vec![key(Key::S, Modifiers::NONE)]).0, vec![Act::Tab(RibbonTab::SendReceive)]);
    run(&mut tips, vec![key(Key::Escape, Modifiers::NONE)]);
    assert_eq!(tips.prefix(), Some("J"));
    run(&mut tips, vec![key(Key::Escape, Modifiers::NONE)]);
    run(&mut tips, vec![key(Key::Escape, Modifiers::NONE)]);
    assert_eq!(tips.prefix(), None);
    // F, Q: the File page and the search box.
    run(&mut tips, vec![alt(true), alt(false)]);
    assert_eq!(run(&mut tips, vec![key(Key::F, Modifiers::NONE)]).0, vec![Act::File]);
    run(&mut tips, vec![alt(true), alt(false)]);
    assert_eq!(run(&mut tips, vec![key(Key::Q, Modifiers::NONE)]).0, vec![Act::Search]);
    // Held Alt types them too (Alt+H), and a shortcut with Ctrl hides them.
    let (acts, _) = run(&mut tips, vec![alt(true), key(Key::H, Modifiers::ALT)]);
    assert_eq!(acts, vec![Act::Tab(RibbonTab::Home)]);
    run(&mut tips, vec![alt(false), key(Key::R, Modifiers::CTRL)]);
    assert_eq!(tips.prefix(), None);
}

/// Alt held for a character's code (Alt+0201) types it; letting go doesn't show the key tips.
#[test]
fn an_alt_code_shows_no_key_tips() {
    let ctx = Context::default();
    ctx.set_os(OperatingSystem::Windows);
    let step = |tips: &mut KeyTips, events: Vec<Event>, code: bool| {
        let mut out = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| {
            if code {
                ui.ctx().data_mut(|d| d.insert_temp(alt_code_typed(), true));
            }
            tips.process(ui.ctx(), true);
        });
        out.textures_delta.clear();
    };
    let mut tips = KeyTips::default();
    step(&mut tips, vec![alt(true)], false);
    step(&mut tips, vec![], true);
    step(&mut tips, vec![alt(false)], false);
    assert_eq!(tips.prefix(), None);
    step(&mut tips, vec![alt(true)], false);
    step(&mut tips, vec![alt(false)], false);
    assert_eq!(tips.prefix(), Some(""), "a plain tap still shows them");
}

/// Every button's letters lead to it alone: none is the start of another's on its tab.
#[test]
fn each_sequence_names_one_button() {
    for (tab, _, label) in table::BUTTONS {
        let (head, own) = table::button_letters(*tab, label).unwrap_or_default();
        let own = format!("{head}{own}");
        assert_eq!(table::lookup(&own), Some(Entry::Button(own.clone())), "{label}");
        assert!(!table::has_deeper(&own), "{label}: {own} starts another");
    }
}

/// Looking a sequence up in two parts finds what the whole sequences written out find, for every
/// sequence of up to three of the letters and digits the table uses, and a few it doesn't.
#[test]
fn sequences_in_two_parts_read_as_whole_ones() {
    let whole: Vec<(String, Entry)> = table::TOPS
        .iter()
        .map(|(l, top)| (l.to_string(), Entry::Top(*top)))
        .chain(table::BUTTONS.iter().filter_map(|(tab, l, _)| {
            let s = format!("{}{l}", table::tab_letters(*tab)?);
            Some((s.clone(), Entry::Button(s)))
        }))
        .collect();
    let letters: Vec<char> = "FHJSVQNDORMWGULABCPY12Z".chars().collect();
    let mut tried = 0;
    for len in 0..=3u32 {
        for mut n in 0..letters.len().pow(len) {
            let mut s = String::new();
            for _ in 0..len {
                s.push(letters[n % letters.len()]);
                n /= letters.len();
            }
            let found = whole.iter().find(|(w, _)| *w == s).map(|(_, e)| e.clone());
            let deeper = whole.iter().any(|(w, _)| w.len() > s.len() && w.starts_with(&s));
            assert_eq!((table::lookup(&s), table::has_deeper(&s)), (found, deeper), "{s:?}");
            tried += 1;
        }
    }
    assert_eq!(tried, 1 + 23 + 23 * 23 + 23 * 23 * 23);
}

/// The ribbon's buttons show the letters that are left to type: none before a tab's are typed,
/// each Home button's after H, and only those that go on from R after H, R.
#[test]
fn buttons_show_the_letters_left_to_type() {
    let (mut a, _) = app("keytips-badges");
    let ctx = Context::default();
    ctx.set_os(OperatingSystem::Windows);
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![key(Key::ArrowDown, Modifiers::NONE)]);
    let size = egui::vec2(1280.0, 800.0);
    let badges = |texts: &[String]| -> Vec<String> {
        ["N", "D", "O", "R1", "MV", "RP", "RA", "FW", "W", "G", "U", "L"]
            .iter()
            .filter(|b| texts.iter().any(|t| t == *b))
            .map(|b| b.to_string())
            .collect()
    };
    let texts = crate::tests::drawn_strings(&ctx, &mut a, size);
    assert!(badges(&texts).is_empty(), "no key tips yet: {texts:?}");
    frame(&ctx, &mut a, vec![alt(true), alt(false)]);
    assert!(badges(&crate::tests::drawn_strings(&ctx, &mut a, size)).is_empty(), "the tabs' letters only");
    frame(&ctx, &mut a, vec![key(Key::H, Modifiers::NONE)]);
    let shown = badges(&crate::tests::drawn_strings(&ctx, &mut a, size));
    assert!(["N", "D", "RP", "RA", "FW", "MV"].iter().all(|b| shown.iter().any(|s| s == b)), "{shown:?}");
    frame(&ctx, &mut a, vec![key(Key::R, Modifiers::NONE)]);
    let shown = badges(&crate::tests::drawn_strings(&ctx, &mut a, size));
    assert!(shown.iter().any(|s| s == "RP") && shown.iter().any(|s| s == "RA"), "{shown:?}");
    assert!(!shown.iter().any(|s| s == "N" || s == "D" || s == "FW"), "{shown:?}");
}

/// In the app: Alt, H, R, P on a message opens its reply, as in Outlook; the File page and the
/// search box have theirs.
#[test]
fn alt_h_r_p_replies() {
    let (mut a, _) = app("keytips");
    let ctx = Context::default();
    ctx.set_os(OperatingSystem::Windows);
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![key(Key::ArrowDown, Modifiers::NONE)]);
    assert!(!a.session.selection.is_empty());
    a.ui.tab = RibbonTab::View;
    frame(&ctx, &mut a, vec![alt(true), alt(false)]);
    frame(&ctx, &mut a, vec![key(Key::H, Modifiers::NONE)]);
    assert_eq!(a.ui.tab, RibbonTab::Home);
    frame(&ctx, &mut a, vec![key(Key::R, Modifiers::NONE)]);
    assert!(a.session.compose.is_none());
    frame(&ctx, &mut a, vec![key(Key::P, Modifiers::NONE)]);
    frame(&ctx, &mut a, vec![]);
    assert!(a.session.compose.is_some(), "the reply opened");
    assert_eq!(a.ui.keytips.prefix(), None);
}

/// Found in review: with no message selected, Alt, H, D pressed a popped-out message window's own
/// Delete (a button of the same name), and the message went with its window. Only the ribbon's
/// buttons take key tips.
#[test]
fn only_the_ribbons_buttons_take_key_tips() {
    let (mut a, _) = app("keytips-window");
    let ctx = Context::default();
    ctx.set_os(OperatingSystem::Windows);
    frame(&ctx, &mut a, vec![]);
    let k = a.session.list[0].key.clone();
    a.run("message.popOut", serde_json::json!({"key": k.to_id()}));
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    assert!(a.session.windows.len() == 1 && a.session.selection.is_empty());
    // The ribbon's Delete can't be used, the window's can.
    assert!(a.session.registry().get("mail.delete").is_some_and(|c| (c.enabled)(&a.session).is_some()));
    frame(&ctx, &mut a, vec![alt(true), alt(false)]);
    for letter in [Key::H, Key::D] {
        frame(&ctx, &mut a, vec![key(letter, Modifiers::NONE)]);
    }
    frame(&ctx, &mut a, vec![]);
    assert_eq!(a.session.windows.len(), 1, "its window stays");
    assert!(a.session.summary(&k).is_some_and(|m| !m.flags.deleted), "the message stays");
}

/// What the Home tab's buttons pay for key tips on a frame where none show (Alt not pressed), as
/// the ribbon draws them.
#[test]
#[ignore = "a measurement: cargo test --release -p mailcraft-ui-egui --lib measure_idle_apply -- --ignored --nocapture"]
fn measure_idle_apply() {
    let ctx = Context::default();
    let t = Tokens::light();
    let labels: Vec<&str> = table::BUTTONS.iter().filter(|(tab, _, _)| *tab == RibbonTab::Home).map(|(_, _, l)| *l).collect();
    const ROUNDS: u32 = 20_000;
    let mut each = std::time::Duration::ZERO;
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
        set_frame(ui.ctx(), Frame { prefix: None, tab: Some(RibbonTab::Home), press: None, ribbon: false });
        let mut resp = ui.allocate_response(egui::vec2(10.0, 10.0), egui::Sense::click());
        on_ribbon(ui, |ui| {
            let start = std::time::Instant::now();
            for _ in 0..ROUNDS {
                for label in &labels {
                    apply(ui, std::hint::black_box(label), &mut resp, true, &t);
                }
            }
            each = start.elapsed() / (ROUNDS * labels.len() as u32);
        });
        assert!(!resp.clicked());
    });
    out.textures_delta.clear();
    println!("MEASURE keytips::apply, idle, {} Home buttons: {each:?} a button", labels.len());
}
