//! What each system has of its own that MailCraft uses: the desktop's dark or light preference
//! (Linux and the BSDs), documents and quitting the Mac way (macOS), notifications under
//! MailCraft's name (Windows and macOS), and the system keychain for passwords.

pub(super) mod appearance;
#[cfg(target_os = "macos")]
pub(super) mod apple_events;
#[cfg(any(target_os = "macos", test))]
pub(super) mod notifications;
pub(super) mod secrets;
#[cfg(windows)]
pub(super) mod toast;
