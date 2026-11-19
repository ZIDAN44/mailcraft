//! What a screen reader is told without the keyboard moving (issue #175): a notice as it comes,
//! a setting switched with a key, bold switched while writing.

use egui::accesskit::{Live, Role};

use super::*;

const SIZE: egui::Vec2 = egui::vec2(1280.0, 800.0);

/// What the frame's live regions say: their words, how urgently, and their node.
fn spoken(out: &egui::FullOutput) -> Vec<(String, Live, egui::accesskit::NodeId)> {
    let u = out.platform_output.accesskit_update.as_ref().expect("AccessKit is on");
    u.nodes
        .iter()
        .filter(|(_, n)| n.role() == Role::Status)
        .filter_map(|(id, n)| Some((n.label().unwrap_or_default().to_string(), n.live().filter(|l| *l != Live::Off)?, *id)))
        .collect()
}

/// A notice is said as it comes, a failure urgently; the same words again are said again (a node
/// of their own, which the reader takes as new).
#[test]
fn a_notice_is_said_as_it_comes() {
    let (mut a, _) = app("access-said-notice");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    sized_frame(&ctx, &mut a, SIZE, vec![]);
    a.session.notice("Copied to Archive", false);
    let first = spoken(&sized_frame(&ctx, &mut a, SIZE, vec![]));
    assert_eq!(first.iter().map(|(w, l, _)| (w.as_str(), *l)).collect::<Vec<_>>(), vec![("Copied to Archive", Live::Polite)]);
    a.session.notice("Copied to Archive", false);
    let again = spoken(&sized_frame(&ctx, &mut a, SIZE, vec![]));
    assert_eq!(again.len(), 1);
    assert_ne!(again[0].2, first[0].2, "said again, as new");
    a.session.notice("The server refused the message", true);
    let failed = spoken(&sized_frame(&ctx, &mut a, SIZE, vec![]));
    assert_eq!(failed.iter().map(|(w, l, _)| (w.as_str(), *l)).collect::<Vec<_>>(), vec![("The server refused the message", Live::Assertive)]);
}

/// A setting switched with its key says how it's left: Ctrl+F1 hides the ribbon, and shows it.
#[test]
fn a_setting_switched_with_a_key_says_how_it_is_left() {
    let (mut a, _) = app("access-said-toggle");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    sized_frame(&ctx, &mut a, SIZE, vec![]);
    let hidden = a.session.prefs.ribbon_hidden;
    let out = sized_frame(&ctx, &mut a, SIZE, vec![key(Key::F1, Modifiers::COMMAND)]);
    assert_ne!(a.session.prefs.ribbon_hidden, hidden, "Ctrl+F1 switched it");
    let words: Vec<String> = spoken(&out).into_iter().map(|(w, _, _)| w).collect();
    assert_eq!(words, vec![if hidden { "Hide the ribbon: off" } else { "Hide the ribbon: on" }.to_string()]);
    let out = sized_frame(&ctx, &mut a, SIZE, vec![key(Key::F1, Modifiers::COMMAND)]);
    let words: Vec<String> = spoken(&out).into_iter().map(|(w, _, _)| w).collect();
    assert_eq!(words, vec![if hidden { "Hide the ribbon: on" } else { "Hide the ribbon: off" }.to_string()]);
}

/// Ctrl+B, Ctrl+I and Ctrl+U while writing say what they did: on for what's typed next, then
/// off; for a selection, how the whole of it is left.
#[test]
fn the_formatting_keys_say_what_they_did() {
    let (mut a, _) = app("access-said-format");
    a.session.prefs.new_mail_window = false;
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    sized_frame(&ctx, &mut a, SIZE, vec![]);
    a.run("mail.new", json!({}));
    a.run("compose.update", json!({"to": "ann@x.example", "subject": "Plan", "body": "Hello"}));
    sized_frame(&ctx, &mut a, SIZE, vec![]);
    let id = crate::rich_edit::body_id(a.session.compose.as_ref().expect("a message is written"));
    ctx.memory_mut(|m| m.request_focus(id));
    sized_frame(&ctx, &mut a, SIZE, vec![]);
    let said = |a: &mut MailApp, k: Key| -> Vec<String> {
        spoken(&sized_frame(&ctx, a, SIZE, vec![key(k, Modifiers::COMMAND)])).into_iter().map(|(w, _, _)| w).collect()
    };
    assert_eq!(said(&mut a, Key::B), vec!["Bold on"]);
    assert_eq!(said(&mut a, Key::B), vec!["Bold off"]);
    assert_eq!(said(&mut a, Key::I), vec!["Italic on"]);
    // The whole of "Hello" selected, underlined, then not.
    let mut state = egui::TextEdit::load_state(&ctx, id).unwrap_or_default();
    state.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(5))));
    state.store(&ctx, id);
    assert_eq!(said(&mut a, Key::U), vec!["Underline on"]);
    assert!(a.session.compose.as_ref().is_some_and(|c| c.styles.style(2).underline));
    assert_eq!(said(&mut a, Key::U), vec!["Underline off"]);
}
