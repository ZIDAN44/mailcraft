//! UI tests: the reactions gathered under a message.

use super::*;

/// The reaction chips the frame shows, as a screen reader is told them.
fn chips(ctx: &egui::Context, a: &mut MailApp) -> Vec<String> {
    let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))), ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| a.ui(ui));
    out.textures_delta.clear();
    let tree = out.platform_output.accesskit_update.expect("AccessKit is on");
    tree.nodes.iter().filter_map(|(_, n)| n.label()).filter(|l| l.starts_with("Reacted with")).map(str::to_string).collect()
}

/// The reactions under a message are kept from frame to frame, and gathered again when one
/// comes: the user's own, filed in Sent Items, and someone else's, in the list.
#[test]
fn reactions_under_a_message_follow_those_that_come() {
    let (mut a, _) = app("reactions-row");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let plan = b"From: Sam Lee <sam@studio.test>\r\nTo: riley@studio.example\r\nSubject: Launch plan\r\nMessage-ID: <plan-1@studio.test>\r\n\r\nHere's the plan.\r\n";
    let key = arrive(&mut a, 9960, plan);
    a.run("folder.open", json!({"account": "sample", "folder": "Sent"}));
    a.run("folder.open", json!({"account": "sample", "folder": "INBOX"}));
    a.run("list.open", json!({"key": key.to_id()}));
    for _ in 0..2 {
        assert_eq!(chips(&ctx, &mut a), Vec::<String>::new());
    }
    a.run("mail.react", json!({"emoji": "👍"}));
    chips(&ctx, &mut a);
    let mine = chips(&ctx, &mut a);
    assert!(mine.len() == 1 && mine[0].starts_with("Reacted with 👍: "), "{mine:?}");
    let kim = b"From: Kim <kim@studio.test>\r\nTo: riley@studio.example\r\nSubject: Re: Launch plan\r\nMessage-ID: <react-2@studio.test>\r\nIn-Reply-To: <plan-1@studio.test>\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Disposition: reaction\r\n\r\n\xE2\x9D\xA4\xEF\xB8\x8F\r\n";
    arrive(&mut a, 9961, kim);
    a.run("folder.open", json!({"account": "sample", "folder": "INBOX"}));
    a.run("list.open", json!({"key": key.to_id()}));
    chips(&ctx, &mut a);
    let both = chips(&ctx, &mut a);
    assert!(both.len() == 2 && both.iter().any(|c| c == "Reacted with ❤️: Kim"), "{both:?}");
}
