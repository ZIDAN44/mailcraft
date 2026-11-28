//! New mail announced, and the notification clicked (issue #273): what it was about, kept for the
//! window's next frame, and the window woken for it (minimized or in the notification area, it
//! draws none by itself).
//!
//! Windows: a toast of MailCraft's own carries a `mailcraft:` link that reaches this MailCraft
//! (`platform/toast/link.rs`), so one clicked in the notification centre comes back through a
//! launch, as a link opened does; without one (a portable copy, or one installed before the
//! installer registered the link), its click while it shows. Linux
//! and the BSDs: the notification's default action, a click on it, while the notification server
//! keeps it (its list of them too). macOS: a click, in Notification Center too, until it's
//! cleared.

use std::sync::{Arc, Mutex, PoisonError};

use mailcraft_engine::model::MessageKey;

/// How this MailCraft announces mail: portable (Windows: nothing written to the registry), and
/// on which data folder it runs (a notification's link starts the MailCraft on that one). Only
/// Windows' toasts carry a link.
#[derive(Clone, Debug, Default)]
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) struct Sending {
    pub(crate) portable: bool,
    pub(crate) root: Option<std::path::PathBuf>,
}

pub(crate) use crate::platform::Chime;

/// Show the notification for new mail, `opens` the message a click shows, or a reminder, with
/// `chime`'s sound; on the calling thread (the notification service can be slow), which waits for
/// the click where that's how it comes.
#[cfg(windows)]
pub(crate) fn send(title: &str, body: &str, opens: Option<MessageKey>, clicks: &Clicks, sending: &Sending, chime: Chime) {
    use crate::platform::toast;
    // Windows names a toast after its app id, and shows one only for an id it knows. A portable
    // copy writes nothing to the registry: it uses an installed MailCraft's, else PowerShell's.
    let ours = if sending.portable { toast::installed() } else { toast::registered() };
    let app_id = if ours { toast::APP_ID } else { toast::POWERSHELL };
    let reaches = !sending.portable && sending.root.as_deref().is_some_and(toast::link_ready);
    let link = reaches.then(|| mailcraft_engine::model::app_link::show_link(opens.as_ref()));
    log::info!(
        "notification shown as {}, {}",
        if ours { "MailCraft" } else { "PowerShell" },
        if reaches { "a click starting its link" } else { "its click waited for" }
    );
    match toast::show(app_id, title, body, link.as_deref(), chime) {
        Ok(true) => {
            log::info!("notification clicked");
            clicks.clicked(opens);
        }
        Ok(false) => {}
        Err(e) => log::warn!("notification: {e}"),
    }
}

/// The most notifications waited on for a click at once: each holds a thread until it's clicked
/// or gone, which on macOS is when it's cleared from Notification Center. One past them is shown
/// all the same, its click not followed.
#[cfg(not(windows))]
const WAITING_MAX: usize = 32;

/// [`send`] on Linux, the BSDs and macOS: notify-rust's notification, its click waited for.
#[cfg(not(windows))]
pub(crate) fn send(title: &str, body: &str, opens: Option<MessageKey>, clicks: &Clicks, _sending: &Sending, chime: Chime) {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static WAITING: AtomicUsize = AtomicUsize::new(0);
    let mut n = notify_rust::Notification::new();
    n.summary(title).body(body).appname("MailCraft");
    // A click on the notification is its default action (the freedesktop spec); on a Mac it's
    // reported by itself, and an action would be a button.
    #[cfg(not(target_os = "macos"))]
    n.action("default", mailcraft_ui_egui::i18n::t("Open"));
    // The freedesktop sound theme's new mail sound or its alarm; a Mac's own sound for either.
    match chime {
        Chime::Silent => {}
        _ if cfg!(target_os = "macos") => {
            n.sound_name("default");
        }
        Chime::NewMail => {
            n.sound_name("message-new-email");
        }
        Chime::Reminder => {
            n.sound_name("alarm-clock-elapsed");
        }
    }
    match n.show() {
        Ok(shown) => {
            if WAITING.fetch_add(1, Ordering::SeqCst) >= WAITING_MAX {
                WAITING.fetch_sub(1, Ordering::SeqCst);
                log::info!("notification: {WAITING_MAX} already wait for a click; this one's isn't followed");
                // Dropped, on a Mac it goes out by itself.
                return;
            }
            shown.wait_for_action(|action| {
                if action == "default" {
                    log::info!("notification clicked");
                    clicks.clicked(opens);
                }
            });
            WAITING.fetch_sub(1, Ordering::SeqCst);
        }
        Err(e) => log::warn!("notification: {e}"),
    }
}

/// The click waiting for the window, and the window to wake.
#[derive(Clone, Default)]
pub(crate) struct Clicks {
    inner: Arc<Mutex<Inner>>,
}

#[derive(Default)]
struct Inner {
    /// Clicked: the message it was about (`None`: nothing in particular).
    clicked: Option<Option<MessageKey>>,
    wake: Option<egui::Context>,
}

impl Clicks {
    /// Clicks that wake `ctx`'s window.
    pub(crate) fn waking(ctx: &egui::Context) -> Clicks {
        let c = Clicks::default();
        c.inner.lock().unwrap_or_else(PoisonError::into_inner).wake = Some(ctx.clone());
        c
    }

    /// A notification about `opens` clicked: kept (the latest), the window woken.
    pub(crate) fn clicked(&self, opens: Option<MessageKey>) {
        let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        inner.clicked = Some(opens);
        if let Some(ctx) = &inner.wake {
            ctx.request_repaint();
        }
    }

    /// The click since last asked, if any.
    pub(crate) fn take(&self) -> Option<Option<MessageKey>> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner).clicked.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A click is kept until it's taken, once; the latest of two before then.
    #[test]
    fn a_click_is_taken_once() {
        let c = Clicks::default();
        assert_eq!(c.take(), None);
        let key = |uid| MessageKey { account: "a".into(), folder: "INBOX".into(), validity: 1, uid };
        c.clicked(Some(key(1)));
        c.clone().clicked(Some(key(2)));
        assert_eq!(c.take(), Some(Some(key(2))));
        assert_eq!(c.take(), None);
        c.clicked(None);
        assert_eq!(c.take(), Some(None));
    }
}
