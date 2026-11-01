//! The window (`MailApp`), in parts: each continues `impl MailApp` with what it draws or does.

mod actions;
mod alt_code;
mod bars;
mod bodies;
pub(crate) mod boot;
mod dropped;
mod frame;
mod keys;
mod ribbon;
mod search_box;
pub(crate) mod source;
mod updates;
mod windows;

pub use alt_code::{alt_code_pages, alt_codes};
pub(crate) use bodies::Bodies;
pub use boot::BootCard;
pub use dropped::{ClipboardRead, clipboard_keys};
pub(crate) use ribbon::junk_menu;
pub(crate) use search_box::{SearchDrop, scope_label};
pub use windows::{WindowPlace, after_open};
