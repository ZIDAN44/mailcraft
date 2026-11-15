//! The window (`MailApp`), in parts: each continues `impl MailApp` with what it draws or does.

mod actions;
mod alt_code;
mod bodies;
pub(crate) mod boot;
mod chrome;
mod data_files;
mod dropped;
pub(crate) mod frame;
mod keys;
pub(crate) mod source;
mod updates;
mod windows;

pub(crate) use chrome::caption;
use chrome::{bars, ribbon, search_box};
pub(crate) use data_files::{compacting, importing};

pub use alt_code::{alt_code_pages, alt_codes};
pub(crate) use bodies::Bodies;
pub use boot::BootCard;
pub use dropped::{ClipboardRead, clipboard_keys};
pub use frame::FramePart;
pub(crate) use ribbon::junk_menu;
pub(crate) use search_box::{SearchDrop, SearchTimes, scope_label};
pub use windows::{Monitors, WindowPlace, after_open, set_monitors};
