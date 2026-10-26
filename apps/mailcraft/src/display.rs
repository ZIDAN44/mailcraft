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
/// an X socket's path.
pub fn choose(display: Option<&str>, wayland: Option<&str>, force_wayland: bool, socket_exists: impl Fn(&Path) -> bool) -> Choice {
    let wayland = wayland.is_some_and(|w| !w.is_empty());
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
    choose(display.as_deref(), wayland.as_deref(), force, |p| p.exists())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// On a Wayland desktop with XWayland the window opens on X11, where files drag and drop.
    #[test]
    fn xwayland_when_there_is_one() {
        let all = |_: &Path| true;
        assert_eq!(choose(Some(":0"), Some("wayland-0"), false, all), Choice::X11);
        assert_eq!(choose(Some(":0"), None, false, all), Choice::X11);
        // Asked to stay on Wayland, or with no X display at all: winit's choice.
        assert_eq!(choose(Some(":0"), Some("wayland-0"), true, all), Choice::Default);
        assert_eq!(choose(None, Some("wayland-0"), false, all), Choice::Default);
        assert_eq!(choose(Some(""), Some("wayland-0"), false, all), Choice::Default);
        // Forced without a compositor: X11 is all there is.
        assert_eq!(choose(Some(":0"), None, true, all), Choice::X11);
    }

    /// A stale local DISPLAY opens on Wayland rather than failing; a remote one stays X11's.
    #[test]
    fn a_stale_local_display_falls_back_to_wayland() {
        let none = |_: &Path| false;
        for d in [":1", ":1.0", "unix:1"] {
            assert_eq!(choose(Some(d), Some("wayland-0"), false, none), Choice::Default, "{d}");
        }
        assert_eq!(choose(Some(":1"), None, false, none), Choice::X11);
        assert_eq!(choose(Some("host:10.0"), Some("wayland-0"), false, none), Choice::X11);
        // The socket checked is the display's own.
        assert_eq!(choose(Some(":3"), Some("w"), false, |p: &Path| p.ends_with("X3")), Choice::X11);
        assert_eq!(choose(Some(":3"), Some("w"), false, |p: &Path| p.ends_with("X0")), Choice::Default);
    }
}
