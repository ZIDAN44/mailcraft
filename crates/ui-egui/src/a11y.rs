//! What a screen reader is sent: a guard on every AccessKit update egui produces.
//!
//! AccessKit's contract is that an update's focus is one of its nodes, and the consumer behind
//! the Windows, macOS and Linux adapters treats a breach as fatal: it panics on the UI thread,
//! which closes the window (on Windows: "Focused ID … is not in the node list"). egui can send an
//! update that drops the focused widget's node, when the widget holding the keyboard isn't drawn
//! that frame. The guard points such an update's focus at the root, so the reader starts from the
//! top of the window instead of MailCraft closing (WordCraft #2726 found it).

/// Whether a screen reader (or another assistive technology) is reading the window this frame:
/// egui builds the tree it reads only once one has asked for it.
pub(crate) fn screen_reader(ctx: &egui::Context) -> bool {
    ctx.has_accesskit_node(egui::accesskit_root_id())
}

/// What was said last ([`announce`]): its number, its words, whether it's urgent, and when.
#[derive(Clone, Default)]
struct Said {
    n: u64,
    words: String,
    urgent: bool,
    at: f64,
}

/// Where [`Said`] is kept, between frames.
const SAID: &str = "a11y said";

/// How long what's said stays in the tree, for a reader to find again.
const SAID_SECS: f64 = 5.0;

/// Say `words` to a screen reader without moving the keyboard (issue #175): what a command did
/// that nothing where the keyboard is shows (a notice, a setting switched with a key, bold
/// switched while typing). `urgent` interrupts what it's saying (a failure). Said by
/// [`said`] in the frame, as a live region is.
pub(crate) fn announce(ctx: &egui::Context, words: impl Into<String>, urgent: bool) {
    let id = egui::Id::unique(SAID);
    let at = ctx.input(|i| i.time);
    ctx.data_mut(|d| {
        let n = d.get_temp::<Said>(id).map_or(0, |s| s.n.wrapping_add(1));
        d.insert_temp(id, Said { n, words: words.into(), urgent, at });
    });
}

/// What was said last, in this frame's tree while it's fresh: a live region a screen reader says
/// as it comes. A node of its own each time, so the same words said again are said again.
pub(crate) fn said(ctx: &egui::Context) {
    let id = egui::Id::unique(SAID);
    let Some(s) = ctx.data(|d| d.get_temp::<Said>(id)) else { return };
    if ctx.input(|i| i.time) - s.at > SAID_SECS {
        return;
    }
    ctx.accesskit_node_builder(id.with(s.n), |node| {
        node.set_role(egui::accesskit::Role::Status);
        node.set_label(s.words);
        node.set_live(if s.urgent { egui::accesskit::Live::Assertive } else { egui::accesskit::Live::Polite });
    });
}

/// The guard, as an egui plugin: it sees each frame's output before the adapter does.
pub(crate) struct FocusGuard;

impl egui::plugin::Plugin for FocusGuard {
    fn debug_name(&self) -> &'static str {
        "a11y focus guard"
    }

    fn output_hook(&mut self, _ctx: &egui::Context, output: &mut egui::FullOutput) {
        if let Some(update) = &mut output.platform_output.accesskit_update {
            guard_update(update);
        }
    }
}

/// Point the focus at the root when this update's nodes don't include it.
pub(crate) fn guard_update(update: &mut egui::accesskit::TreeUpdate) {
    if update.nodes.iter().any(|(id, _)| *id == update.focus) {
        return;
    }
    if let Some(root) = update.tree.as_ref().map(|t| t.root) {
        update.focus = root;
    }
}

#[cfg(test)]
mod tests {
    use egui::accesskit::{self, Node, NodeId, Role, TreeUpdate};

    use super::*;

    fn update(focus: NodeId, kept: NodeId) -> TreeUpdate {
        let root = egui::accesskit_root_id().accesskit_id();
        let mut window = Node::new(Role::Window);
        window.push_child(kept);
        TreeUpdate {
            nodes: vec![(root, window), (kept, Node::new(Role::Button))],
            tree: Some(accesskit::TreeInfo::new(root)),
            tree_id: accesskit::TreeId::ROOT,
            focus,
        }
    }

    /// The consumer the adapters run dies on an update whose focus isn't among its nodes; the
    /// guard moves the focus to the root, and the same update is taken.
    #[test]
    fn a_focus_the_update_does_not_carry_falls_back_to_the_root() {
        let (lost, kept) = (egui::Id::unique("dropped").accesskit_id(), egui::Id::unique("kept").accesskit_id());
        let raw = std::panic::catch_unwind(|| accesskit_consumer::Tree::new(update(lost, kept), true));
        assert!(raw.is_err(), "an update without its focused node breaks the consumer");
        let mut fixed = update(lost, kept);
        guard_update(&mut fixed);
        assert_eq!(fixed.focus, egui::accesskit_root_id().accesskit_id());
        assert!(std::panic::catch_unwind(|| accesskit_consumer::Tree::new(fixed, true)).is_ok(), "the guarded update is taken");
        // A focus the update carries stays where it is.
        let mut fine = update(kept, kept);
        guard_update(&mut fine);
        assert_eq!(fine.focus, kept);
    }
}
