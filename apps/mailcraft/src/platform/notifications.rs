//! macOS notifications from MailCraft (issue #193).
//!
//! A notification on a Mac goes out as some application's, by its bundle id. Told none,
//! notify-rust (mac-notification-sys) asks AppleScript for the id of an application named
//! `use_default`: none is, so macOS opened a "Where is use_default?" window for the user to find
//! it, one for each notification sent before the first was answered (they're sent on threads of
//! their own, and the crate looks outside its once-only guard), and after Cancel the
//! notifications went out as the Finder's. So MailCraft names the application once, as it
//! starts, before anything is sent: itself, running from its `.app`, which macOS knows by
//! [`APP_ID`]; the Finder, as a bare program (`cargo run`), which has no bundle id to give.

use std::path::Path;

/// MailCraft's bundle id (`CFBundleIdentifier` in `packaging/macos/Info.plist.in`).
pub const APP_ID: &str = mailcraft_ui_egui::APP_ID;

/// The application a bare program's notifications go out as: the one mac-notification-sys falls
/// back to.
const NO_BUNDLE: &str = "com.apple.Finder";

/// The bundle id notifications go out under, for the program at `exe`: MailCraft's own inside an
/// app bundle (`….app/Contents/MacOS/`), else the Finder's.
pub fn sender_for(exe: &Path) -> &'static str {
    let in_bundle = exe.parent().is_some_and(|macos| {
        macos.file_name().is_some_and(|n| n == "MacOS")
            && macos.parent().is_some_and(|contents| {
                contents.file_name().is_some_and(|n| n == "Contents")
                    && contents.parent().and_then(Path::extension).is_some_and(|e| e.eq_ignore_ascii_case("app"))
            })
    });
    if in_bundle { APP_ID } else { NO_BUNDLE }
}

/// Name the application notifications go out as, once, before the first is sent (they'd look
/// for `use_default` otherwise). A name macOS doesn't know leaves its own bundle's: never the
/// AppleScript either way, as the crate does that only while nothing was named.
#[cfg(target_os = "macos")]
pub fn claim() {
    let sender = std::env::current_exe().map_or(NO_BUNDLE, |exe| sender_for(&exe));
    match notify_rust::set_application(sender) {
        Ok(()) => log::info!("notifications go out as {sender}"),
        Err(e) => log::info!("notifications: {sender} not taken ({e}): the app's own bundle id is used"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notifications_go_out_as_mailcraft_from_its_app_bundle() {
        assert_eq!(sender_for(Path::new("/Applications/MailCraft.app/Contents/MacOS/mailcraft")), APP_ID);
        assert_eq!(sender_for(Path::new("/Volumes/MailCraft 0.41/MailCraft.APP/Contents/MacOS/mailcraft")), APP_ID);
        // A bare program has no bundle id of its own.
        for bare in ["/Users/kim/mailcraft/target/release/mailcraft", "/MacOS/mailcraft", "/x/Contents/MacOS/mailcraft", "mailcraft", ""] {
            assert_eq!(sender_for(Path::new(bare)), NO_BUNDLE, "{bare}");
        }
    }
}
