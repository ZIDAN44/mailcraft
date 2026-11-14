//! The message's text as its text box edits it, knowing what it was before the box changed it.

use egui::text::CharIndex;

use super::*;

/// The text box's text: the message's, and, once the box changes it, what it was before (to
/// see whether the box had changed it, the text was copied every frame).
pub(super) struct Tracked<'a> {
    text: &'a mut String,
    before: Option<String>,
}

impl<'a> Tracked<'a> {
    pub(super) fn new(text: &'a mut String) -> Tracked<'a> {
        Tracked { text, before: None }
    }

    /// What the text was, if the box changed it.
    pub(super) fn before(self) -> Option<String> {
        let now = self.text;
        self.before.filter(|before| before != now)
    }

    fn changing(&mut self) {
        if self.before.is_none() {
            self.before = Some(self.text.clone());
        }
    }
}

/// As a `String` is, each change first keeping what the text was.
impl egui::TextBuffer for Tracked<'_> {
    fn is_mutable(&self) -> bool {
        true
    }

    fn as_str(&self) -> &str {
        self.text.as_str()
    }

    fn insert_text(&mut self, text: &str, char_index: CharIndex) -> usize {
        self.changing();
        <String as egui::TextBuffer>::insert_text(self.text, text, char_index)
    }

    fn delete_char_range(&mut self, char_range: Range<CharIndex>) {
        self.changing();
        <String as egui::TextBuffer>::delete_char_range(self.text, char_range);
    }

    fn clear(&mut self) {
        self.changing();
        <String as egui::TextBuffer>::clear(self.text);
    }

    fn replace_with(&mut self, text: &str) {
        self.changing();
        <String as egui::TextBuffer>::replace_with(self.text, text);
    }

    fn take(&mut self) -> String {
        self.changing();
        <String as egui::TextBuffer>::take(self.text)
    }

    fn type_id(&self) -> std::any::TypeId {
        std::any::TypeId::of::<Tracked<'static>>()
    }
}
