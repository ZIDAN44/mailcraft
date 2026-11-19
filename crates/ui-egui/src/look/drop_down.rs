//! A drop-down's keys, as a list box's on Windows (issue #175). egui's drop-down has none: Up
//! and Down moved the keyboard to the field above or below, so a screen reader's user, who has
//! only the keyboard, could change one only by finding the keys that open its list.

use egui::{EventFilter, Id, Key, Modifiers, Response, Ui};

/// Up and Down choose the choice before or after, Home and End the first and the last, while
/// the list is closed; the keyboard stays on the drop-down, which a screen reader reads with its
/// new value. Alt+Down opens the list (as Space and Enter do) with the keyboard on the choice
/// made, and its own arrows go through its choices; Enter or Space picks one and closes the list
/// (egui closes it on a click only), and then, or once Esc closes it, the keyboard is back on the
/// drop-down (it was in the list, gone).
/// `resp` is egui's ComboBox's; `choices` are what its list offers, in its order.
pub(crate) fn keys<T: PartialEq + Clone>(ui: &Ui, resp: &Response, v: &mut T, choices: &[T]) {
    let ctx = ui.ctx();
    let id = resp.id;
    // As egui's ComboBox names its list.
    let list = id.with("popup");
    let focused = ctx.memory(|m| m.focused());
    // Where the keyboard was in the open list, kept until the list closes.
    let in_list = id.with("keyboard in the list");
    if egui::Popup::is_id_open(ctx, list) {
        let item = focused.filter(|f| *f != id);
        // A choice picked with the keyboard (the list's contents, drawn before this, took the
        // press as a click on it).
        if item.is_some() && ctx.input(|i| i.key_pressed(Key::Enter) || i.key_pressed(Key::Space)) {
            egui::Popup::close_id(ctx, list);
            resp.request_focus();
            return;
        }
        // Opened with the keyboard on the drop-down: it goes in at the choice made, as a list
        // box's does (egui's goes in at the first, once Down is pressed). Not in the frame it
        // opens in, which lays the list out unseen, with nothing in it to click yet.
        if focused == Some(id)
            && let Some(at) = choices.iter().position(|c| c == v)
            && let items = list_items(ctx, list)
            // The list offers what `choices` does (one that offers more is gone into as egui does).
            && items.len() == choices.len()
            && let Some(choice) = items.get(at)
        {
            ctx.memory_mut(|m| m.request_focus(*choice));
        }
        ctx.data_mut(|d| d.insert_temp(in_list, item));
        // The open list's arrows are egui's again: Down goes into it.
        ctx.memory_mut(|m| m.set_focus_lock_filter(id, EventFilter::default()));
        return;
    }
    if let Some(item) = ctx.data_mut(|d| d.remove_temp::<Option<Id>>(in_list)).flatten()
        && focused.is_none_or(|f| f == item)
    {
        resp.request_focus();
    }
    if !resp.has_focus() {
        return;
    }
    // Up and Down are the drop-down's, not egui's way to the next field (from the next frame:
    // egui takes them before anything is drawn).
    ctx.memory_mut(|m| m.set_focus_lock_filter(id, EventFilter { vertical_arrows: true, ..Default::default() }));
    if ctx.input_mut(|i| i.consume_key(Modifiers::ALT, Key::ArrowDown)) {
        egui::Popup::open_id(ctx, list);
        return;
    }
    let Some(last) = choices.len().checked_sub(1) else { return };
    let at = choices.iter().position(|c| c == v);
    let to = ctx.input_mut(|i| {
        if i.consume_key(Modifiers::NONE, Key::ArrowDown) {
            Some(at.map_or(0, |a| a.saturating_add(1).min(last)))
        } else if i.consume_key(Modifiers::NONE, Key::ArrowUp) {
            Some(at.map_or(0, |a| a.saturating_sub(1)))
        } else if i.consume_key(Modifiers::NONE, Key::Home) {
            Some(0)
        } else if i.consume_key(Modifiers::NONE, Key::End) {
            Some(last)
        } else {
            None
        }
    });
    if let Some(c) = to.and_then(|n| choices.get(n)) {
        *v = c.clone();
    }
}

/// The choices of the open list `list`, in their order: what can be clicked in it this frame and
/// take the keyboard (not the list's ground, nor a heading such as an account's name over its
/// folders).
fn list_items(ctx: &egui::Context, list: Id) -> Vec<Id> {
    let layer = egui::LayerId::new(egui::Order::Foreground, list);
    ctx.viewport(|v| {
        v.this_pass.widgets.get_layer(layer).filter(|w| w.enabled && w.sense.senses_click() && w.sense.is_focusable()).map(|w| w.id).collect()
    })
}
