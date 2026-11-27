use super::*;

/// The owner's report: on the File page the Export card's account picker and button sat on top of
/// its description. A row's controls never cover its text, at any width: beside it when there's
/// room, under it when not, and inside the card either way.
#[test]
fn a_setting_rows_controls_never_cover_its_text() {
    let t = Tokens::light();
    for width in [420.0, 520.0, 760.0] {
        let ctx = egui::Context::default();
        let mut got = (egui::Rect::NOTHING, egui::Rect::NOTHING);
        // A few frames: the row measures its controls in one and lays out by them in the next.
        for _ in 0..4 {
            let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(width, 600.0))), ..Default::default() };
            let mut out = ctx.run_ui(input, |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    got = setting_row_at(
                        ui,
                        &t,
                        true,
                        name(&t, "Account"),
                        Some(note(&t, "Its contacts, calendar, notes and tasks go inside it too.")),
                        |ui| {
                            let _ = ui.button("Export…");
                            egui::ComboBox::from_id_salt("account")
                                .width(300.0)
                                .selected_text("riley@studio.example   ·   Sample mailbox")
                                .show_ui(ui, |_| {});
                        },
                    );
                });
            });
            // No renderer here: drop the texture uploads deliberately.
            out.textures_delta.clear();
        }
        let (text, controls) = got;
        assert!(!text.intersects(controls), "{width}: text {text:?}, controls {controls:?}");
        assert!(controls.min.x >= -0.5 && controls.max.x <= width + 0.5, "{width}: controls {controls:?} outside the window");
        assert!(text.max.x <= width + 0.5, "{width}: text {text:?}");
        // Beside the text when the window is wide, under it when narrow.
        let beside = (controls.center().y - text.center().y).abs() < 20.0;
        assert_eq!(beside, width >= 700.0, "{width}: text {text:?}, controls {controls:?}");
    }
}

/// Options is in parts, each under it in the rail: one shows its own cards (one page of thirteen
/// ran seven screens and more); a search finds a setting by its name or what it does, in any.
#[test]
fn options_is_in_parts_with_a_search() {
    use crate::tests::*;
    let (mut a, _) = app("options-parts");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    a.act(crate::Action::Backstage(Page::Options));
    let size = egui::vec2(1280.0, 2400.0);
    for _ in 0..2 {
        let _ = drawn_text(&ctx, &mut a, size);
    }
    let rail: Vec<String> = named(&ctx, &mut a, size).into_iter().filter_map(|n| n.name().map(str::to_string)).collect();
    for part in ["General", "Mail", "Calendar", "Data files", "Privacy and safety", "AI agents"] {
        assert!(rail.iter().any(|n| n == part), "{part} in the rail: {rail:?}");
    }
    let general = drawn_strings(&ctx, &mut a, size);
    assert!(has(&general, "Theme") && !has(&general, "Check spelling as you type"), "General alone: {general:?}");
    click_named(&ctx, &mut a, "Mail");
    let mail = drawn_strings(&ctx, &mut a, size);
    assert!(has(&mail, "Writing") && has(&mail, "Check spelling as you type") && !has(&mail, "Theme"), "{mail:?}");
    if let Some(b) = a.ui.backstage.as_mut() {
        b.find = "spelling".into();
    }
    for _ in 0..2 {
        let _ = drawn_text(&ctx, &mut a, size);
    }
    let found = drawn_strings(&ctx, &mut a, size);
    assert!(has(&found, "Check spelling as you type") && !has(&found, "Reading pane") && !has(&found, "Theme"), "{found:?}");
}
