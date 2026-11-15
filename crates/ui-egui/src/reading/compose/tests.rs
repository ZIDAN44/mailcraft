//! The message form's tests: what it reads from the session on every frame.

use super::*;

/// The median time of `f`, run many times.
fn median(mut f: impl FnMut()) -> std::time::Duration {
    for _ in 0..50 {
        f();
    }
    let mut times: Vec<std::time::Duration> = (0..400)
        .map(|_| {
            let start = std::time::Instant::now();
            f();
            start.elapsed()
        })
        .collect();
    times.sort();
    times.get(times.len() / 2).copied().unwrap_or_default()
}

/// What a frame of the message form reads from the session besides the message: From's
/// choices, and the pictures from the web the reader was shown (400, as many as are kept).
#[test]
#[ignore = "a measurement: run in release"]
fn measure_what_the_form_reads_on_every_frame() {
    let (mut a, _) = crate::tests::app("measure-form");
    a.run("mail.new", json!({}));
    for n in 0..400 {
        let url = format!("https://pictures.newsletter.example/2026/11/issue-{n}/header-image.png");
        a.session.pictures.insert(url, mailcraft_engine::Picture::Ready(std::sync::Arc::new(vec![0u8; 64])));
    }
    let ctx = egui::Context::default();
    let choices = median(|| {
        std::hint::black_box(addresses::from_choices(&a.session));
    });
    let web = median(|| rich_edit::web_loaded(&ctx, &a.session.pictures));
    println!("MEASURE From's choices: median {choices:?}; the web pictures, 400 loaded: median {web:?}");
}
