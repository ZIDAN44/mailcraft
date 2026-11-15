//! Which windowing system the window opens on, on Linux and the BSDs.
//!
//! MailCraft's attachments drag out to other apps through X11's drag and drop (`xdnd.rs`; the
//! Craft apps' winit takes files dropped on Wayland too, but has no drag out), so on a Wayland
//! desktop the window opens through XWayland when it's there (as the other Craft apps do);
//! `MAILCRAFT_WAYLAND=1` keeps it on Wayland. A stale `DISPLAY`
//! (an ended `ssh -X` session, or XWayland turned off) made such a start fail with "failed to
//! open display" while a Wayland compositor was running (DeckCraft found): a local `DISPLAY` whose
//! X socket isn't there opens on Wayland instead.

// The choice is tested everywhere; only Linux and the BSDs use it.
#![cfg_attr(not(all(unix, not(target_os = "macos"))), allow(dead_code))]

use std::path::Path;

/// The windowing system to ask winit for.
#[derive(Debug, PartialEq, Eq)]
pub enum Choice {
    X11,
    /// winit's own choice (Wayland when `WAYLAND_DISPLAY` is set).
    Default,
}

/// The X server number of a local `DISPLAY` (`:1`, `:1.0`, `unix:1`); `None` for a remote one
/// (`host:10.0`, as `ssh -X` sets) or one that can't be read, which are left to X11 itself.
fn local_display_number(display: &str) -> Option<u32> {
    let rest = display.strip_prefix(':').or_else(|| display.strip_prefix("unix:"))?;
    rest.split('.').next()?.parse().ok()
}

/// The choice for `display` (`DISPLAY`) and `wayland` (`WAYLAND_DISPLAY`); `socket_exists` checks
/// an X socket's path; `x11_libraries` says which of X11's libraries are missing, if any. The
/// packages only recommend those, and winit fails to start (panics, without libxkbcommon-x11)
/// when they're asked for and missing: with a Wayland compositor running, the window opens on
/// Wayland instead (WordCraft found this on Fedora and Ubuntu). Asked only when it would be
/// XWayland.
pub fn choose(
    display: Option<&str>,
    wayland: Option<&str>,
    force_wayland: bool,
    socket_exists: impl Fn(&Path) -> bool,
    x11_libraries: impl FnOnce() -> Result<(), String>,
) -> Choice {
    let wayland = wayland.is_some_and(|w| !w.is_empty());
    let choice = choose_display(display, wayland, force_wayland, socket_exists);
    if choice == Choice::X11
        && wayland
        && let Err(missing) = x11_libraries()
    {
        log::warn!("X11's libraries aren't all installed ({missing}); opening on Wayland");
        return Choice::Default;
    }
    choice
}

/// [`choose`], X11's libraries aside.
fn choose_display(display: Option<&str>, wayland: bool, force_wayland: bool, socket_exists: impl Fn(&Path) -> bool) -> Choice {
    match display.filter(|d| !d.is_empty()) {
        None => Choice::Default,
        Some(_) if force_wayland && wayland => Choice::Default,
        Some(d) => match local_display_number(d) {
            Some(n) if wayland && !socket_exists(&Path::new("/tmp/.X11-unix").join(format!("X{n}"))) => {
                log::warn!("DISPLAY={d} has no X server; opening on Wayland");
                Choice::Default
            }
            _ => Choice::X11,
        },
    }
}

/// [`choose`] for this process.
pub fn choose_from_env() -> Choice {
    let display = std::env::var("DISPLAY").ok();
    let wayland = std::env::var("WAYLAND_DISPLAY").ok();
    let force = std::env::var("MAILCRAFT_WAYLAND").is_ok_and(|v| !v.is_empty() && v != "0");
    choose(display.as_deref(), wayland.as_deref(), force, |p| p.exists(), x11_libraries)
}

/// `Err` naming the libraries winit's X11 backend opens at start that can't be loaded: libX11,
/// libXcursor, libX11-xcb, libXi, and libxkbcommon-x11 for the keyboard.
#[cfg(all(unix, not(target_os = "macos")))]
fn x11_libraries() -> Result<(), String> {
    use x11_dl::{xcursor::Xcursor, xinput2::XInput2, xlib::Xlib, xlib_xcb::Xlib_xcb};
    let mut missing: Vec<String> = [Xlib::open().err(), Xcursor::open().err(), Xlib_xcb::open().err(), XInput2::open().err()]
        .into_iter()
        .flatten()
        .map(|e| e.to_string())
        .collect();
    if xkbcommon_dl::x11::xkbcommon_x11_option().is_none() {
        missing.push("libxkbcommon-x11.so.0 could not be loaded".into());
    }
    if missing.is_empty() { Ok(()) } else { Err(missing.join("; ")) }
}

#[cfg(not(all(unix, not(target_os = "macos"))))]
fn x11_libraries() -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// X11's libraries, all there.
    fn ok() -> Result<(), String> {
        Ok(())
    }

    /// On a Wayland desktop with XWayland the window opens on X11, where files drag and drop.
    #[test]
    fn xwayland_when_there_is_one() {
        let all = |_: &Path| true;
        assert_eq!(choose(Some(":0"), Some("wayland-0"), false, all, ok), Choice::X11);
        assert_eq!(choose(Some(":0"), None, false, all, ok), Choice::X11);
        // Asked to stay on Wayland, or with no X display at all: winit's choice.
        assert_eq!(choose(Some(":0"), Some("wayland-0"), true, all, ok), Choice::Default);
        assert_eq!(choose(None, Some("wayland-0"), false, all, ok), Choice::Default);
        assert_eq!(choose(Some(""), Some("wayland-0"), false, all, ok), Choice::Default);
        // Forced without a compositor: X11 is all there is.
        assert_eq!(choose(Some(":0"), None, true, all, ok), Choice::X11);
    }

    /// A stale local DISPLAY opens on Wayland rather than failing; a remote one stays X11's.
    #[test]
    fn a_stale_local_display_falls_back_to_wayland() {
        let none = |_: &Path| false;
        for d in [":1", ":1.0", "unix:1"] {
            assert_eq!(choose(Some(d), Some("wayland-0"), false, none, ok), Choice::Default, "{d}");
        }
        assert_eq!(choose(Some(":1"), None, false, none, ok), Choice::X11);
        assert_eq!(choose(Some("host:10.0"), Some("wayland-0"), false, none, ok), Choice::X11);
        // The socket checked is the display's own.
        assert_eq!(choose(Some(":3"), Some("w"), false, |p: &Path| p.ends_with("X3"), ok), Choice::X11);
        assert_eq!(choose(Some(":3"), Some("w"), false, |p: &Path| p.ends_with("X0"), ok), Choice::Default);
    }

    /// Without X11's libraries (the packages only recommend them) the window stays on Wayland,
    /// where winit would have failed to start; they're looked for only when it would be XWayland.
    #[test]
    fn missing_x11_libraries_keep_the_window_on_wayland() {
        let all = |_: &Path| true;
        let missing = || Err("libxkbcommon-x11.so.0 could not be loaded".to_string());
        assert_eq!(choose(Some(":0"), Some("wayland-0"), false, all, missing), Choice::Default);
        // No compositor: X11 is all there is, and winit says what's wrong itself.
        assert_eq!(choose(Some(":0"), None, false, all, missing), Choice::X11);
        let unasked = || -> Result<(), String> { panic!("X11's libraries looked for though it isn't XWayland") };
        assert_eq!(choose(Some(":0"), Some("wayland-0"), true, all, unasked), Choice::Default);
        assert_eq!(choose(None, Some("wayland-0"), false, all, unasked), Choice::Default);
        assert_eq!(choose(Some(":1"), Some("wayland-0"), false, |_: &Path| false, unasked), Choice::Default);
    }
}
