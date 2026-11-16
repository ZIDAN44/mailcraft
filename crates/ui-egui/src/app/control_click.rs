//! On a Mac a Control-click is the right-click, in every app. winit reports it as the primary
//! button with Control held, so no right-click menu in MailCraft opened on it: on a Mac with no
//! second button and no two-finger click, they couldn't be reached at all (issue #119: removing a
//! subscribed calendar is on its menu). As PhotoCraft reads it.

/// Read a Control-click as the right-click on a Mac (`macos`), in each window's input before egui
/// reads it. The release goes with its press even if Control was let go first, and a click with
/// Command held as well stays a click.
pub fn control_click(ctx: &egui::Context, raw: &mut egui::RawInput, macos: bool) {
    if !macos {
        return;
    }
    // Whether the button down now went down as a right-click, per window.
    let id = egui::Id::unique(("control-click", raw.viewport_id));
    let mut held = ctx.data(|d| d.get_temp::<bool>(id)).unwrap_or(false);
    for e in &mut raw.events {
        if let egui::Event::PointerButton { button: button @ egui::PointerButton::Primary, pressed, modifiers, .. } = e {
            if *pressed {
                held = modifiers.ctrl && !modifiers.mac_cmd && !modifiers.command;
            }
            if held {
                *button = egui::PointerButton::Secondary;
            }
            if !*pressed {
                held = false;
            }
        }
    }
    ctx.data_mut(|d| d.insert_temp(id, held));
}

#[cfg(test)]
mod tests {
    use egui::{Event, Modifiers, PointerButton};

    use super::*;

    fn press(pressed: bool, modifiers: Modifiers) -> Event {
        Event::PointerButton { pos: egui::pos2(50.0, 50.0), button: PointerButton::Primary, pressed, modifiers }
    }

    fn buttons(ctx: &egui::Context, events: Vec<Event>, macos: bool) -> Vec<PointerButton> {
        let mut raw = egui::RawInput { events, ..Default::default() };
        control_click(ctx, &mut raw, macos);
        raw.events
            .iter()
            .filter_map(|e| match e {
                Event::PointerButton { button, .. } => Some(*button),
                _ => None,
            })
            .collect()
    }

    /// A Control-click on a Mac is a right-click, its release too when Control was let go first
    /// and the release comes in a later frame; a plain click after it is a click again.
    #[test]
    fn a_control_click_on_a_mac_is_a_right_click() {
        let ctx = egui::Context::default();
        assert_eq!(buttons(&ctx, vec![press(true, Modifiers::CTRL)], true), [PointerButton::Secondary]);
        assert_eq!(buttons(&ctx, vec![press(false, Modifiers::NONE)], true), [PointerButton::Secondary], "its release");
        let plain = vec![press(true, Modifiers::NONE), press(false, Modifiers::NONE)];
        assert_eq!(buttons(&ctx, plain, true), [PointerButton::Primary; 2]);
    }

    /// Elsewhere Ctrl+click stays a click (it adds a message to the selection), and on a Mac so
    /// does Control with Command.
    #[test]
    fn a_ctrl_click_elsewhere_stays_a_click() {
        let ctx = egui::Context::default();
        let ctrl = vec![press(true, Modifiers::CTRL), press(false, Modifiers::CTRL)];
        assert_eq!(buttons(&ctx, ctrl, false), [PointerButton::Primary; 2]);
        let both = Modifiers { ctrl: true, mac_cmd: true, command: true, ..Default::default() };
        assert_eq!(buttons(&ctx, vec![press(true, both), press(false, both)], true), [PointerButton::Primary; 2]);
    }
}
