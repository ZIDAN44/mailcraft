//! The window (`MailApp`), in parts: each continues `impl MailApp` with what it draws or does.

mod actions;
mod bars;
mod frame;
mod keys;
mod ribbon;
mod windows;

pub(crate) use ribbon::junk_menu;
pub use windows::{WindowPlace, after_open};
