//! The window (`MailApp`), in parts: each continues `impl MailApp` with what it draws or does.

mod actions;
mod alt_code;
mod bars;
mod dropped;
mod frame;
mod keys;
mod ribbon;
mod search_box;
mod updates;
mod windows;

pub use alt_code::{alt_code_pages, alt_codes};
pub use dropped::{ClipboardRead, clipboard_keys};
pub(crate) use ribbon::junk_menu;
pub(crate) use search_box::{SearchDrop, scope_label};
pub use windows::{WindowPlace, after_open};
