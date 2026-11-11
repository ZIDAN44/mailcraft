//! MailCraft in the system's notification area (issue #67): with "Keep MailCraft running when
//! its window is closed" on, closing the window hides it and MailCraft goes on checking for
//! mail and announcing it. Its icon there opens the window again (a click, or Open MailCraft in
//! its menu) and ends MailCraft (Exit). The icon is there while the option is on, the window
//! open or not, so it's known to be there before a close relies on it.
//!
//! Windows only for now: `tray-icon` draws it there with no other toolkit. On Linux it needs
//! GTK's loop beside the window's, and a Mac keeps a closed app in its Dock (`docs/gaps.md`).

use std::sync::atomic::{AtomicBool, Ordering};

use mailcraft_ui_egui::MailApp;

/// Whether the icon is in the notification area now.
static SHOWN: AtomicBool = AtomicBool::new(false);

/// Whether this build can keep MailCraft in the notification area.
pub(crate) fn offered() -> bool {
    cfg!(windows)
}

pub(crate) fn shown() -> bool {
    SHOWN.load(Ordering::Relaxed)
}

/// What the icon was asked for.
#[cfg_attr(not(windows), allow(dead_code))]
enum Ask {
    Open,
    Exit,
}

pub(crate) struct Tray {
    #[cfg(windows)]
    icon: Option<windows::Icon>,
    /// It couldn't be made: not tried again at every frame.
    #[cfg(windows)]
    failed: bool,
    asks: std::sync::mpsc::Receiver<Ask>,
    #[cfg_attr(not(windows), allow(dead_code))]
    ask: std::sync::mpsc::Sender<Ask>,
}

impl Tray {
    pub(crate) fn new() -> Self {
        let (ask, asks) = std::sync::mpsc::channel();
        Self {
            #[cfg(windows)]
            icon: None,
            #[cfg(windows)]
            failed: false,
            asks,
            ask,
        }
    }

    /// Once a pass, the window drawn or not: the icon there while the option is on, saying what
    /// the window's title says, and what it was asked for done.
    pub(crate) fn follow(&mut self, app: &mut MailApp, ctx: &egui::Context) {
        #[cfg(windows)]
        self.keep(app, ctx);
        for ask in self.asks.try_iter() {
            match ask {
                Ask::Open => app.show_window(ctx),
                Ask::Exit => app.exit(ctx),
            }
        }
    }

    #[cfg(windows)]
    fn keep(&mut self, app: &MailApp, ctx: &egui::Context) {
        let wanted = app.session.prefs.close_to_tray;
        if !wanted {
            if self.icon.take().is_some() {
                SHOWN.store(false, Ordering::Relaxed);
            }
            self.failed = false;
            return;
        }
        let words = app.tray_words();
        match &mut self.icon {
            Some(icon) => icon.say(&words),
            None if !self.failed => match windows::Icon::new(&words, self.ask.clone(), ctx.clone()) {
                Ok(icon) => {
                    self.icon = Some(icon);
                    SHOWN.store(true, Ordering::Relaxed);
                }
                Err(e) => {
                    log::warn!("notification area: {e}");
                    self.failed = true;
                }
            },
            None => {}
        }
    }
}

#[cfg(windows)]
mod windows {
    use tray_icon::menu::{Menu, MenuEvent, MenuItem};
    use tray_icon::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

    use super::Ask;

    pub(super) struct Icon {
        icon: TrayIcon,
        open: MenuItem,
        exit: MenuItem,
        /// The tooltip and the menu's two lines, as last set.
        words: (String, String, String),
    }

    impl Icon {
        pub(super) fn new(words: &(String, String, String), ask: std::sync::mpsc::Sender<Ask>, ctx: egui::Context) -> Result<Self, String> {
            let (tip, open_text, exit_text) = words;
            let open = MenuItem::new(open_text, true, None);
            let exit = MenuItem::new(exit_text, true, None);
            let menu = Menu::new();
            menu.append_items(&[&open, &exit]).map_err(|e| e.to_string())?;
            let picture = crate::app_icon();
            let picture = tray_icon::Icon::from_rgba(picture.rgba, picture.width, picture.height).map_err(|e| e.to_string())?;
            let icon = TrayIconBuilder::new()
                .with_menu(Box::new(menu))
                .with_menu_on_left_click(false)
                .with_tooltip(tooltip(tip))
                .with_icon(picture)
                .build()
                .map_err(|e| e.to_string())?;
            // Its events come on the window's thread, between frames: each wakes the window
            // (hidden, it draws nothing and would not look).
            let (open_id, exit_id) = (open.id().clone(), exit.id().clone());
            let (menu_ask, menu_ctx) = (ask.clone(), ctx.clone());
            MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
                let asked = if e.id == open_id {
                    Ask::Open
                } else if e.id == exit_id {
                    Ask::Exit
                } else {
                    return;
                };
                let _ = menu_ask.send(asked);
                menu_ctx.request_repaint();
            }));
            TrayIconEvent::set_event_handler(Some(move |e: TrayIconEvent| {
                let clicked = matches!(e, TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. })
                    || matches!(e, TrayIconEvent::DoubleClick { button: MouseButton::Left, .. });
                if clicked {
                    let _ = ask.send(Ask::Open);
                    ctx.request_repaint();
                }
            }));
            Ok(Self { icon, open, exit, words: words.clone() })
        }

        /// The title's unread count, or the language, changed.
        pub(super) fn say(&mut self, words: &(String, String, String)) {
            if *words == self.words {
                return;
            }
            let (tip, open, exit) = words;
            if let Err(e) = self.icon.set_tooltip(Some(tooltip(tip))) {
                log::warn!("notification area: {e}");
            }
            self.open.set_text(open);
            self.exit.set_text(exit);
            self.words = words.clone();
        }
    }

    impl Drop for Icon {
        fn drop(&mut self) {
            MenuEvent::set_event_handler(None::<fn(MenuEvent)>);
            TrayIconEvent::set_event_handler(None::<fn(TrayIconEvent)>);
        }
    }

    /// Windows shows at most 127 characters of a tooltip.
    fn tooltip(title: &str) -> String {
        let title = if title.trim().is_empty() { "MailCraft" } else { title };
        title.chars().take(120).collect()
    }
}
