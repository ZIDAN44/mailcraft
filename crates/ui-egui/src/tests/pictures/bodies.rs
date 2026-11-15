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
    let (mut a, _) = app("body-lane");
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
}

/// Moving down the list faster than the lane reads hands it one piece of work, which reads the
/// message landed on (not each one passed); a reply then acts on that message, read at once.
#[test]
fn the_lane_reads_only_the_latest() {
    let (mut a, _) = app("body-latest");
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
}

/// Work no thread takes is done in the window: the message shows in the frame it was opened in.
#[test]
fn a_body_no_lane_takes_is_read_in_the_window() {
    let (mut a, _) = app("body-refused");
    let ctx = egui::Context::default();
    a.services = Box::new(Refused);
    a.defer_bodies();
    frame(&ctx, &mut a, vec![]);
    let k = a.session.list[0].key.clone();
    a.run("list.open", json!({"key": k.to_id()}));
    assert!(a.session.body_waiting(&k));
    frame(&ctx, &mut a, vec![]);
    assert!(a.session.reading.as_ref().unwrap().parsed.is_some() && !a.session.body_waiting(&k));
}

/// On a thread of its own (as in the app), holding Down over big messages parses none of them
/// in a frame, and the one landed on shows.
#[test]
fn holding_down_over_big_messages_parses_none_in_a_frame() {
    let (mut a, _) = app("body-threads");
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
}

/// The arc's target: holding Down over 200 big messages on threads of their own, every step
/// (the command and its frame) takes under 16 ms. A timing test: run in release
/// (`cargo test --release -p mailcraft-ui-egui holding_down_frames -- --ignored --nocapture`).
#[test]
#[ignore = "a timing test: run in release"]
fn holding_down_frames_stay_under_16_ms() {
    let (mut a, _) = app("body-frames");
    let ctx = egui::Context::default();
    a.services = Box::new(Threads);
    a.defer_bodies();
    frame(&ctx, &mut a, vec![]);
    let big = "A long line of text that goes on. ".repeat(30_000);
    for i in 0..200u32 {
        with_files(&mut a, 9700 + i, vec![(format!("notes {i}.txt"), "text/plain", big.clone().into_bytes())]);
    }
    a.run("list.open", json!({"key": a.session.list[0].key.to_id()}));
    frame(&ctx, &mut a, vec![]);
    // Not marked read on the way: each mark is saved at once (no write-behind), 15 to 490 ms
    // a step in a temporary folder, which this lane can't take off the window's thread.
    a.session.prefs.mark_read_on_open = false;
    let mut worst = std::time::Duration::ZERO;
    for _ in 1..200 {
        let t = std::time::Instant::now();
        a.run("list.next", json!({}));
        frame(&ctx, &mut a, vec![]);
        worst = worst.max(t.elapsed());
        // A key held down repeats about every 30 ms.
        std::thread::sleep(std::time::Duration::from_millis(30));
    }
    println!("worst step {:.1} ms", worst.as_secs_f64() * 1000.0);
    assert!(worst < std::time::Duration::from_millis(16), "worst step {worst:?}");
}

/// MEASURE: a frame of a newsletter of centred lines, each with a tracked link (a redirect with
/// tags), all of it in the pane; and what cleaning its links' addresses takes. In release
/// (`cargo test --release -p mailcraft-ui-egui --lib centred_newsletter_frames -- --ignored --nocapture`).
#[test]
#[ignore = "a measurement: run in release"]
fn centred_newsletter_frames() {
    let (mut a, _) = app("newsletter-frames");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let mut html = String::from("<table width=\"600\" align=\"center\"><tr><td align=\"center\">");
    let mut links = Vec::new();
    for i in 0..400 {
        let link =
            format!("https://click.news.example/r?u=https%3A%2F%2Fshop.example%2Fitem%2F{i}%3Futm_source%3Dnews&utm_medium=email&utm_campaign=fall");
        html.push_str(&format!("<p>Read <a href=\"{link}\">story {i} of the fall</a> and <b>save</b> today.</p>"));
        links.push(link);
    }
    html.push_str("</td></tr></table>");
    let raw = format!("From: Shop <news@shop.example>\r\nSubject: Fall\r\nContent-Type: text/html\r\n\r\n{html}");
    let k = with_raw(&mut a, 9800, raw.as_bytes());
    a.run("list.open", json!({"key": k.to_id()}));
    assert!(a.session.prefs.clean_links);
    let whole = median_frame(&ctx, &mut a);
    const ROUNDS: u32 = 100;
    let t = std::time::Instant::now();
    let cleaned: usize = (0..ROUNDS).map(|_| links.iter().map(|l| mailcraft_engine::mime::clean_link(l).len()).sum::<usize>()).sum();
    let clean = t.elapsed() / ROUNDS;
    println!("MEASURE centred newsletter, {} lines with links: median frame {whole:?}; its links cleaned {clean:?} ({cleaned})", links.len());
}

/// The median of 100 frames of the open message, after 5 to settle (the machine is shared: the
/// slowest are others' work).
fn median_frame(ctx: &egui::Context, a: &mut MailApp) -> std::time::Duration {
    pump(ctx, a, WINDOW, 5, |_| false);
    let mut frames: Vec<std::time::Duration> = (0..100)
        .map(|_| {
            let t = std::time::Instant::now();
            pump(ctx, a, WINDOW, 1, |_| false);
            t.elapsed()
        })
        .collect();
    frames.sort();
    frames[frames.len() / 2]
}

/// MEASURE: a frame of a newsletter laid out in tables three deep, a picture from the web in
/// each of its innermost ones still coming (shown, not yet here), and then come. In release
/// (`cargo test --release -p mailcraft-ui-egui --lib nested_tables_frames -- --ignored --nocapture`).
#[test]
#[ignore = "a measurement: run in release"]
fn nested_tables_frames() {
    let (mut a, _) = app("nested-frames");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let words = "Our fall collection is here with coats boots and scarves for every day of the season ".repeat(8);
    let mut html = String::from("<table width=\"100%\"><tr><td>");
    let mut urls = Vec::new();
    for i in 0..20 {
        let url = format!("https://cdn.shop.example/{i}.png");
        html.push_str(&format!(
            "<table><tr><td><table><tr><td>{words}</td><td><img src=\"{url}\" alt=\"Coat {i}\"></td></tr></table></td><td>{words}</td></tr></table>"
        ));
        urls.push(url);
    }
    html.push_str("</td></tr></table>");
    let raw = format!("From: Shop <news@shop.example>\r\nSubject: Fall\r\nContent-Type: text/html\r\n\r\n{html}");
    let k = with_raw(&mut a, 9801, raw.as_bytes());
    a.run("list.open", json!({"key": k.to_id()}));
    a.run("message.showPictures", json!({}));
    let coming = median_frame(&ctx, &mut a);
    assert!(urls.iter().all(|u| matches!(a.session.pictures.get(u), Some(mailcraft_engine::Picture::Loading))), "still coming");
    for u in &urls {
        a.session.apply(MailEvent::Fetched { url: u.clone(), data: Some(png(40, 30)), error: None });
    }
    let here = median_frame(&ctx, &mut a);
    println!("MEASURE nested tables, 20 pictures from the web: median frame while they're coming {coming:?}; once they're here {here:?}");
}
