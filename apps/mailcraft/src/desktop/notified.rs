//! A notification clicked (issue #273): what it was about, kept for the window's next frame, and
//! the window woken for it (minimized or in the notification area, it draws none by itself).

use std::sync::{Arc, Mutex, PoisonError};

use mailcraft_engine::model::MessageKey;

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
