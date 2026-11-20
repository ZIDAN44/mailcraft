//! F6 and Shift+F6: from pane to pane, as in Outlook (issue #175). The keyboard goes to the
//! folders, the message list, the reading pane, the To-Do Bar and the ribbon in turn, so a
//! screen reader's user can get to each without going through every control on the way with Tab.

use crate::*;

/// The main window's panes, in the order F6 goes through them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Pane {
    Folders,
    List,
    /// The reading pane, or Calendar's, People's, Tasks' or Notes' own.
    Reading,
    TodoBar,
    /// The top bar and the ribbon under it.
    Bars,
}

const ORDER: [Pane; 5] = [Pane::Folders, Pane::List, Pane::Reading, Pane::TodoBar, Pane::Bars];

impl MailApp {
    /// F6 (`back`: Shift+F6): the keyboard to the next pane drawn last frame that has a control
    /// to take it, after the one it's in. From nowhere (where the list's arrows work), the list
    /// is where it's from.
    pub(super) fn next_pane(&mut self, ctx: &egui::Context, back: bool) {
        let mut panes = self.ui.panes.clone();
        if let Some(r) = self.ui.top_bar_rect {
            panes.push((Pane::Bars, self.ui.ribbon_rect.map_or(r, |ribbon| r.union(ribbon))));
        }
        let focused = ctx.memory(|m| m.focused());
        let at = focused
            .and_then(|f| ctx.viewport(|v| v.prev_pass.widgets.get(f).map(|w| w.rect.center())))
            .and_then(|p| panes.iter().find(|(_, r)| r.contains(p)).map(|(pane, _)| *pane))
            .unwrap_or(Pane::List);
        let start = ORDER.iter().position(|p| *p == at).unwrap_or(0);
        let n = ORDER.len();
        for step in 1..n {
            let i = if back { (start + n - step) % n } else { (start + step) % n };
            let Some(pane) = ORDER.get(i) else { continue };
            let Some(rect) = panes.iter().find(|(p, _)| p == pane).map(|(_, r)| *r) else { continue };
            if let Some(id) = self.pane_stop(ctx, *pane, rect) {
                ctx.memory_mut(|m| m.request_focus(id));
                return;
            }
        }
    }

    /// The control a pane gives the keyboard to: the folder shown, the list's message (its one stop
    /// for Tab), the message's text when a screen reader reads it, else the pane's first control
    /// wholly in it (not the handle between two panes that resizes them).
    fn pane_stop(&self, ctx: &egui::Context, pane: Pane, rect: egui::Rect) -> Option<egui::Id> {
        let drawn = |id: egui::Id| ctx.viewport(|v| v.prev_pass.widgets.contains(id)).then_some(id);
        let own = match pane {
            Pane::Folders => crate::nav::current_row(ctx).and_then(drawn),
            Pane::List => self.ui.list_stop.and_then(drawn),
            Pane::Reading => self.ui.reading_stop.and_then(drawn),
            _ => None,
        };
        own.or_else(|| {
            ctx.viewport(|v| {
                v.prev_pass
                    .widgets
                    .get_layer(egui::LayerId::background())
                    .find(|w| w.enabled && w.sense.is_focusable() && rect.contains_rect(w.rect))
                    .map(|w| w.id)
            })
        })
    }
}
