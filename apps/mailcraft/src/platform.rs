//! What each system has of its own that MailCraft uses: the desktop's dark or light preference
//! (Linux and the BSDs), documents and quitting the Mac way (macOS), notifications under
//! MailCraft's name (Windows and macOS), the system keychain for passwords, and starting as the
//! user signs in.

pub(super) mod appearance;
#[cfg(target_os = "macos")]
pub(super) mod apple_events;
#[cfg(any(target_os = "macos", test))]
pub(super) mod notifications;
pub(super) mod secrets;
pub(super) mod sign_in;
#[cfg(windows)]
pub(super) mod toast;

/// The sound a notification makes: none, the system's new mail sound (issue #308), or its
/// reminder sound (each reminder sounds, as Outlook's do: issue #239). Here, where the systems'
/// notifications are, so the window's code and theirs both name it without reaching each other.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Chime {
    Silent,
    NewMail,
    Reminder,
}
