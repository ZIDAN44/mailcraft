//! The open message's body read and parsed on the reading lane, not in a frame; its pictures
//! taken out of the source read for it.

use super::*;

/// Each piece of work handed over (whatever its lane), run in order.
fn run_held(held: &std::sync::Arc<std::sync::Mutex<Vec<crate::Work>>>) -> usize {
    let works: Vec<crate::Work> = std::mem::take(&mut *held.lock().unwrap());
    let n = works.len();
    for w in works {
        (w.run)();
    }
    n
}

/// A message opened is read and parsed on the reading lane: the pane shows a spinner until the
/// lane is done, nothing is parsed in a frame, and its pictures are taken out of the source the
/// lane read (the cache isn't read again for them).
#[test]
fn a_body_is_read_on_the_reading_lane() {
    let (mut a, dir) = app("body-lane");
    let ctx = egui::Context::default();
    let held = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    a.services = Box::new(Held(held.clone()));
    a.defer_bodies();
    frame(&ctx, &mut a, vec![]);
    let k = with_files(&mut a, 9501, pictures(6));
    let before = a.session.bodies_parsed_here();
    a.run("list.open", json!({"key": k.to_id()}));
    frame(&ctx, &mut a, vec![]);
    assert!(a.session.reading.as_ref().unwrap().parsed.is_none() && a.session.body_waiting(&k), "waits for the lane");
    assert_eq!(run_held(&held), 1, "the body's work");
    frame(&ctx, &mut a, vec![]);
    assert_eq!(a.session.reading.as_ref().unwrap().parsed.as_ref().unwrap().attachments.len(), 6);
    assert_eq!(a.session.bodies_parsed_here(), before, "nothing parsed in a frame");
    // Its pictures: their work went to the lane too, and starts from the source read for it.
    assert_eq!(run_held(&held), 1, "the pictures' work");
    let (frames, _) = pump(&ctx, &mut a, WINDOW, 50, |a| a.ui.pictures.ready() == 6);
    assert!(frames < 50, "all shown ({} ready)", a.ui.pictures.ready());
    assert_eq!(a.ui.pictures.counts().0, 1, "one extraction");
    assert_eq!(a.ui.pictures.reads(), 0, "from the source read to show the message");
    let _ = std::fs::remove_dir_all(dir);
}

/// Moving down the list faster than the lane reads hands it one piece of work, which reads the
/// message landed on (not each one passed); a reply then acts on that message, read at once.
#[test]
fn the_lane_reads_only_the_latest() {
    let (mut a, dir) = app("body-latest");
    let ctx = egui::Context::default();
    let held = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    a.services = Box::new(Held(held.clone()));
    a.defer_bodies();
    frame(&ctx, &mut a, vec![]);
    let first = a.session.list[0].key.clone();
    a.run("list.open", json!({"key": first.to_id()}));
    frame(&ctx, &mut a, vec![]);
    for _ in 0..4 {
        a.run("list.next", json!({}));
        frame(&ctx, &mut a, vec![]);
    }
    let landed = a.session.list[4].key.clone();
    assert_eq!(a.session.reading.as_ref().unwrap().key, landed);
    assert_eq!(held.lock().unwrap().len(), 1, "one piece of work handed over");
    run_held(&held);
    frame(&ctx, &mut a, vec![]);
    let r = a.session.reading.as_ref().unwrap();
    assert_eq!(r.parsed.as_ref().unwrap().subject, a.session.summary(&landed).unwrap().subject);
    // Another, and a reply before the lane is done.
    a.run("list.next", json!({}));
    frame(&ctx, &mut a, vec![]);
    a.session.prefs.new_mail_window = false;
    a.run("mail.reply", json!({}));
    let next = a.session.list[5].key.clone();
    assert!(a.session.compose.as_ref().is_some_and(|c| c.subject.contains(&a.session.summary(&next).unwrap().subject)));
    run_held(&held);
    frame(&ctx, &mut a, vec![]);
    let _ = std::fs::remove_dir_all(dir);
}

/// Work no thread takes is done in the window: the message shows in the frame it was opened in.
#[test]
fn a_body_no_lane_takes_is_read_in_the_window() {
    let (mut a, dir) = app("body-refused");
    let ctx = egui::Context::default();
    a.services = Box::new(Refused);
    a.defer_bodies();
    frame(&ctx, &mut a, vec![]);
    let k = a.session.list[0].key.clone();
    a.run("list.open", json!({"key": k.to_id()}));
    assert!(a.session.body_waiting(&k));
    frame(&ctx, &mut a, vec![]);
    assert!(a.session.reading.as_ref().unwrap().parsed.is_some() && !a.session.body_waiting(&k));
    let _ = std::fs::remove_dir_all(dir);
}

/// On a thread of its own (as in the app), holding Down over big messages parses none of them
/// in a frame, and the one landed on shows.
#[test]
fn holding_down_over_big_messages_parses_none_in_a_frame() {
    let (mut a, dir) = app("body-threads");
    let ctx = egui::Context::default();
    a.services = Box::new(Threads);
    a.defer_bodies();
    frame(&ctx, &mut a, vec![]);
    let big = "A long line of text that goes on. ".repeat(30_000);
    let keys: Vec<MessageKey> =
        (0..20u32).map(|i| with_files(&mut a, 9600 + i, vec![(format!("notes {i}.txt"), "text/plain", big.clone().into_bytes())])).collect();
    let before = a.session.bodies_parsed_here();
    a.run("list.open", json!({"key": a.session.list[0].key.to_id()}));
    for _ in 1..keys.len() {
        frame(&ctx, &mut a, vec![]);
        a.run("list.next", json!({}));
    }
    let landed = a.session.reading.as_ref().unwrap().key.clone();
    let shown = |a: &MailApp| a.session.reading.as_ref().is_some_and(|r| r.parsed.is_some());
    for _ in 0..1000 {
        if shown(&a) {
            break;
        }
        frame(&ctx, &mut a, vec![]);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(shown(&a), "shown");
    assert_eq!(a.session.reading.as_ref().unwrap().key, landed);
    assert_eq!(a.session.bodies_parsed_here(), before, "none parsed in a frame");
    let _ = std::fs::remove_dir_all(dir);
}
