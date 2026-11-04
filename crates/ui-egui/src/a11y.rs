//! What a screen reader is sent: a guard on every AccessKit update egui produces.
//!
//! AccessKit's contract is that an update's focus is one of its nodes, and the consumer behind
//! the Windows, macOS and Linux adapters treats a breach as fatal: it panics on the UI thread,
//! which closes the window (on Windows: "Focused ID … is not in the node list"). egui can send an
//! update that drops the focused widget's node, when the widget holding the keyboard isn't drawn
//! that frame. The guard points such an update's focus at the root, so the reader starts from the
//! top of the window instead of MailCraft closing (WordCraft #2726 found it).

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
