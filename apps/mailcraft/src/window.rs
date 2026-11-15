//! What MailCraft's window is opened with: the graphics adapter it's drawn with, the windowing
//! system it opens on (Linux and the BSDs), and dragging files out of it under X11.

pub(super) mod display;
pub(super) mod graphics;
#[cfg(all(unix, not(target_os = "macos")))]
pub(super) mod xdnd;
