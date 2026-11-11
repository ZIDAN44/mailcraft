//! MailCraft kept running with its window closed (issue #67): with "Keep MailCraft running when
//! its window is closed" on, closing the window puts it away and MailCraft goes on checking for
//! mail and announcing it.
//!
//! Where it can, the window is hidden and MailCraft's icon in the system's notification area
//! opens it again (a click, or Open MailCraft in its menu) and ends MailCraft (Exit). The icon is
//! there while the option is on, the window open or not, so it's known to be there before a close
//! relies on it. Where a hidden window couldn't be brought back, the window is minimized instead,
//! and the taskbar is the way back:
//!
//! - **Windows:** the icon is `tray-icon`'s.
//! - **Linux and the BSDs:** a StatusNotifierItem (`ksni`, over D-Bus: KDE, Xfce, Cinnamon, MATE,
//!   and GNOME with its AppIndicator extension). With no notification area to take it, or under
//!   Wayland (whose windows can't be hidden and shown again by the app), the window is minimized.
//! - **macOS:** the window is minimized to the Dock, whose icon brings it back, as a Mac app's
//!   does; Quit (⌘Q, the menu's, the Dock's) ends MailCraft, as Exit does.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};

use mailcraft_ui_egui::{KeptOnClose, MailApp};

/// Whether the icon is in the notification area now.
static SHOWN: AtomicBool = AtomicBool::new(false);
/// Whether the window is a Wayland one: hidden, it couldn't be shown again.
static WAYLAND: AtomicBool = AtomicBool::new(false);

/// Whether the icon couldn't be made: there's no notification area to take it.
static FAILED: AtomicBool = AtomicBool::new(false);
/// Where a Quit from outside the window (the Mac's menu and Dock) is sent: [`ask_exit`].
static ASKS: std::sync::Mutex<Option<Sender<Ask>>> = std::sync::Mutex::new(None);

/// Whether the window can be hidden and brought back by an icon here, as far as is known.
fn hides() -> bool {
    !cfg!(target_os = "macos") && !WAYLAND.load(Ordering::Relaxed) && !FAILED.load(Ordering::Relaxed)
}

/// What a close of the window would do with the option on (what the option says of itself).
pub(crate) fn offered() -> Option<KeptOnClose> {
    Some(if hides() { KeptOnClose::Hidden } else { KeptOnClose::Minimized })
}

/// What a close of the window does now, the option on: hidden only with the icon there.
pub(crate) fn kept_on_close() -> Option<KeptOnClose> {
    Some(if hides() && SHOWN.load(Ordering::Relaxed) { KeptOnClose::Hidden } else { KeptOnClose::Minimized })
}

/// Quit, asked from outside the window (the Mac's menu, ⌘Q, the Dock): MailCraft ends, where a
/// close of its window would only put it away. False when nothing is there to take it yet.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn ask_exit(ctx: &egui::Context) -> bool {
    let sent = ASKS.lock().ok().and_then(|a| a.as_ref().map(|a| a.send(Ask::Exit).is_ok())).unwrap_or(false);
    if sent {
        ctx.request_repaint();
    }
    sent
}

/// What was asked for, from the icon or the system.
#[cfg_attr(target_os = "macos", allow(dead_code))]
enum Ask {
    Open,
    Exit,
}

/// The tooltip (the window's title, with the unread count) and the menu's two lines.
#[cfg(not(target_os = "macos"))]
type Words = (String, String, String);

pub(crate) struct Tray {
    #[cfg(not(target_os = "macos"))]
    icon: Option<native::Icon>,
    /// It couldn't be made (no notification area): not tried again at every frame.
    #[cfg(not(target_os = "macos"))]
    failed: bool,
    asks: Receiver<Ask>,
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    ask: Sender<Ask>,
}

impl Tray {
    pub(crate) fn new(native_wayland: bool) -> Self {
        WAYLAND.store(native_wayland, Ordering::Relaxed);
        let (ask, asks) = std::sync::mpsc::channel();
        if let Ok(mut to) = ASKS.lock() {
            *to = Some(ask.clone());
        }
        Self {
            #[cfg(not(target_os = "macos"))]
            icon: None,
            #[cfg(not(target_os = "macos"))]
            failed: false,
            asks,
            ask,
        }
    }

    /// Once a pass, the window drawn or not: the icon there while the option is on, saying what
    /// the window's title says, and what it was asked for done.
    pub(crate) fn follow(&mut self, app: &mut MailApp, ctx: &egui::Context) {
        #[cfg(not(target_os = "macos"))]
        self.keep(app, ctx);
        for ask in self.asks.try_iter() {
            match ask {
                Ask::Open => app.show_window(ctx),
                Ask::Exit => app.exit(ctx),
            }
        }
    }

    #[cfg(not(target_os = "macos"))]
    fn keep(&mut self, app: &MailApp, ctx: &egui::Context) {
        if !app.session.prefs.close_to_tray {
            if self.icon.take().is_some() {
                SHOWN.store(false, Ordering::Relaxed);
            }
            self.failed = false;
            FAILED.store(false, Ordering::Relaxed);
            return;
        }
        let words = app.tray_words();
        match &mut self.icon {
            Some(icon) => icon.say(&words),
            None if !self.failed => match native::Icon::new(&words, self.ask.clone(), ctx.clone()) {
                Ok(icon) => {
                    self.icon = Some(icon);
                    SHOWN.store(true, Ordering::Relaxed);
                }
                Err(e) => {
                    log::info!("notification area: no icon ({e}); the window is minimized when closed");
                    self.failed = true;
                    FAILED.store(true, Ordering::Relaxed);
                }
            },
            None => {}
        }
    }
}

/// A tooltip of a length every system shows (Windows: at most 127 characters).
#[cfg(not(target_os = "macos"))]
fn tooltip(title: &str) -> String {
    let title = if title.trim().is_empty() { "MailCraft" } else { title };
    title.chars().take(120).collect()
}

#[cfg(windows)]
mod native {
    use tray_icon::menu::{Menu, MenuEvent, MenuItem};
    use tray_icon::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

    use super::{Ask, Words, tooltip};

    pub(super) struct Icon {
        icon: TrayIcon,
        open: MenuItem,
        exit: MenuItem,
        words: Words,
    }

    impl Icon {
        pub(super) fn new(words: &Words, ask: std::sync::mpsc::Sender<Ask>, ctx: egui::Context) -> Result<Self, String> {
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
        pub(super) fn say(&mut self, words: &Words) {
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
}

#[cfg(all(unix, not(target_os = "macos")))]
mod native {
    use ksni::blocking::TrayMethods as _;

    use super::{Ask, Words, tooltip};

    /// MailCraft as a StatusNotifierItem: what the notification area asks it, answered from here.
    struct Item {
        words: Words,
        picture: Vec<ksni::Icon>,
        ask: std::sync::mpsc::Sender<Ask>,
        ctx: egui::Context,
    }

    impl Item {
        /// Asked from the notification area's thread: the window is woken to do it (hidden, it
        /// draws nothing and would not look).
        fn asked(&self, ask: Ask) {
            let _ = self.ask.send(ask);
            self.ctx.request_repaint();
        }
    }

    impl ksni::Tray for Item {
        fn id(&self) -> String {
            mailcraft_ui_egui::APP_ID.to_string()
        }
        fn title(&self) -> String {
            "MailCraft".to_string()
        }
        /// The installed icon, by the name the packages give it; the picture below where it
        /// isn't installed (a build run from its folder).
        fn icon_name(&self) -> String {
            mailcraft_ui_egui::APP_ID.to_string()
        }
        fn icon_pixmap(&self) -> Vec<ksni::Icon> {
            self.picture.clone()
        }
        fn tool_tip(&self) -> ksni::ToolTip {
            ksni::ToolTip { title: tooltip(&self.words.0), ..Default::default() }
        }
        fn activate(&mut self, _x: i32, _y: i32) {
            self.asked(Ask::Open);
        }
        fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
            use ksni::menu::StandardItem;
            vec![
                StandardItem { label: self.words.1.clone(), activate: Box::new(|item: &mut Self| item.asked(Ask::Open)), ..Default::default() }
                    .into(),
                StandardItem { label: self.words.2.clone(), activate: Box::new(|item: &mut Self| item.asked(Ask::Exit)), ..Default::default() }
                    .into(),
            ]
        }
    }

    pub(super) struct Icon {
        handle: ksni::blocking::Handle<Item>,
        words: Words,
    }

    impl Icon {
        /// Fails where nothing on the session's bus takes notification-area icons (GNOME without
        /// its AppIndicator extension, a bare window manager), or there's no bus.
        pub(super) fn new(words: &Words, ask: std::sync::mpsc::Sender<Ask>, ctx: egui::Context) -> Result<Self, String> {
            let app = crate::app_icon();
            // Its pixels as the specification wants them: ARGB, most significant byte first.
            let data: Vec<u8> = app.rgba.as_chunks::<4>().0.iter().flat_map(|[r, g, b, a]| [*a, *r, *g, *b]).collect();
            let picture = match (i32::try_from(app.width), i32::try_from(app.height)) {
                (Ok(width), Ok(height)) if !data.is_empty() => vec![ksni::Icon { width, height, data }],
                _ => Vec::new(),
            };
            let item = Item { words: words.clone(), picture, ask, ctx };
            let handle = item.spawn().map_err(|e| e.to_string())?;
            Ok(Self { handle, words: words.clone() })
        }

        /// The title's unread count, or the language, changed.
        pub(super) fn say(&mut self, words: &Words) {
            if *words == self.words {
                return;
            }
            self.words = words.clone();
            let words = words.clone();
            let _ = self.handle.update(move |item: &mut Item| item.words = words);
        }
    }

    impl Drop for Icon {
        fn drop(&mut self) {
            // Taken out of the notification area; not waited for, on the window's thread.
            let _ = self.handle.shutdown();
        }
    }
}
