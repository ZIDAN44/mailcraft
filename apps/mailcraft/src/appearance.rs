//! Linux and the BSDs: the desktop's light or dark preference, which winit doesn't report there
//! (its X11 window has no theme, its Wayland one only the one MailCraft sets), read from the
//! freedesktop portal (`org.freedesktop.appearance` / `color-scheme`) and followed as it changes.
//! As figcraft does it. Elsewhere winit reports the system's theme and this is never started.

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

/// What the portal said: not known (no portal, no preference), dark or light.
#[cfg(any(test, all(unix, not(target_os = "macos"))))]
const UNKNOWN: u8 = 0;
const DARK: u8 = 1;
const LIGHT: u8 = 2;

/// The desktop's preference, as last read.
#[derive(Clone, Default)]
pub(crate) struct SystemDark(Arc<AtomicU8>);

impl SystemDark {
    /// Dark or light, when the desktop said.
    pub(crate) fn get(&self) -> Option<bool> {
        match self.0.load(Ordering::Relaxed) {
            DARK => Some(true),
            LIGHT => Some(false),
            _ => None,
        }
    }

    /// Reads the preference, then follows it on a thread of its own; `wake` redraws the window,
    /// idle or not, when it changes. Without a session bus or portal nothing changes.
    pub(crate) fn follow(&self, wake: impl Fn() + Send + 'static) {
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            let dark = self.0.clone();
            if let Err(e) = std::thread::Builder::new().name("mailcraft-theme".into()).spawn(move || watch(&dark, &wake)) {
                log::info!("colour scheme: can't start the watcher: {e}");
            }
        }
        #[cfg(not(all(unix, not(target_os = "macos"))))]
        let _ = wake;
    }
}

/// The portal's value: 1 prefers dark, 2 prefers light, 0 no preference (GNOME's default, which
/// apps show as light).
#[cfg(any(test, all(unix, not(target_os = "macos"))))]
fn from_portal(v: u32) -> u8 {
    match v {
        1 => DARK,
        2 => LIGHT,
        _ => UNKNOWN,
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
fn watch(dark: &AtomicU8, wake: &dyn Fn()) {
    use zbus::zvariant::Value;
    const PORTAL: &str = "org.freedesktop.portal.Desktop";
    const SETTINGS: &str = "org.freedesktop.portal.Settings";
    const APPEARANCE: &str = "org.freedesktop.appearance";
    // `Read` (portals before 1.17, which lack `ReadOne`) wraps the value in one more variant.
    let scheme = |v: &Value| match v {
        Value::U32(x) => *x,
        Value::Value(inner) => match **inner {
            Value::U32(x) => x,
            _ => 0,
        },
        _ => 0,
    };
    let store = |v: u32| {
        let new = from_portal(v);
        if dark.swap(new, Ordering::Relaxed) != new {
            wake();
        }
    };
    let Ok(conn) = zbus::blocking::connection::Builder::session().and_then(|b| b.build()) else {
        log::info!("colour scheme: no session bus, so the desktop's theme isn't followed");
        return;
    };
    let read = |method: &str| -> zbus::Result<u32> {
        let reply = conn.call_method(Some(PORTAL), "/org/freedesktop/portal/desktop", Some(SETTINGS), method, &(APPEARANCE, "color-scheme"))?;
        let body = reply.body();
        Ok(scheme(&body.deserialize::<Value>()?))
    };
    match read("ReadOne").or_else(|_| read("Read")) {
        Ok(v) => store(v),
        Err(e) => log::info!("colour scheme: can't read the desktop's setting: {e}"),
    }
    let follow = || -> zbus::Result<()> {
        let rule =
            zbus::MatchRule::builder().msg_type(zbus::message::Type::Signal).sender(PORTAL)?.interface(SETTINGS)?.member("SettingChanged")?.build();
        for msg in zbus::blocking::MessageIterator::for_match_rule(rule, &conn, Some(8))? {
            let msg = msg?;
            let body = msg.body();
            let Ok((namespace, key, value)) = body.deserialize::<(String, String, Value<'_>)>() else { continue };
            if namespace == APPEARANCE && key == "color-scheme" {
                store(scheme(&value));
            }
        }
        Ok(())
    };
    if let Err(e) = follow() {
        log::info!("colour scheme: can't follow the desktop's setting: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The portal's three answers, and anything else it might say, as MailCraft takes them.
    #[test]
    fn the_portal_values() {
        let s = SystemDark::default();
        assert_eq!(s.get(), None, "nothing said yet");
        for (v, want) in [(1, Some(true)), (2, Some(false)), (0, None), (7, None)] {
            s.0.store(from_portal(v), Ordering::Relaxed);
            assert_eq!(s.get(), want, "{v}");
        }
    }

    /// Against a desktop portal on a real session bus, run by hand (`MAILCRAFT_PORTAL=1`, with
    /// one that says dark, then changes to light): read at once, the change followed and the
    /// window woken for it.
    #[test]
    #[ignore]
    fn follows_a_desktop_portal() {
        if std::env::var_os("MAILCRAFT_PORTAL").is_none() {
            eprintln!("MAILCRAFT_PORTAL isn't set: skipped");
            return;
        }
        let s = SystemDark::default();
        let (woken, wakes) = std::sync::mpsc::channel();
        s.follow(move || {
            let _ = woken.send(());
        });
        let until = |want: Option<bool>| {
            let end = std::time::Instant::now() + std::time::Duration::from_secs(20);
            while s.get() != want {
                assert!(std::time::Instant::now() < end, "never {want:?}, still {:?}", s.get());
                let _ = wakes.recv_timeout(std::time::Duration::from_millis(100));
            }
        };
        until(Some(true));
        until(Some(false));
    }
}
