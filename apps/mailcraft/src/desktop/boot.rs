//! The window comes before the data files are open. They open on a `data-open` thread, and the
//! session is made from them there; meanwhile the window draws `BootCard`, which says so when a
//! data file not closed properly is checked (a full read of it: 305 s for a 37 GB file, which
//! used to pass with no window at all). Launches handed over and closing are answered meanwhile.

use std::sync::{Mutex, mpsc};

use mailcraft_engine::store::{OpenProgress, OpenStage};
use mailcraft_ui_egui::BootCard;

use super::*;

/// What MailCraft opens with, from the `data-open` thread.
pub(crate) struct Opened {
    pub(crate) store: Option<Store>,
    /// `None` when opening failed altogether: MailCraft then runs without its cache.
    pub(crate) session: Option<Session>,
    /// A data-file folder moved at start-up, or folders from before that couldn't come in.
    pub(crate) moved: Option<Result<PathBuf, String>>,
}

fn take<T>(slot: &Mutex<Option<T>>) -> Option<T> {
    slot.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take()
}

/// What a panic said.
fn panic_text(p: &(dyn std::any::Any + Send)) -> String {
    p.downcast_ref::<&str>()
        .map(|s| (*s).to_string())
        .or_else(|| p.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "it stopped unexpectedly".into())
}

/// Open `store`'s data files on a thread of their own, then the session (`session` makes it). A
/// cache whose folders from before couldn't come into data files is opened as it was, from
/// `root`. Returns how it goes, and where what MailCraft opens with comes.
pub(crate) fn open(
    store: Option<Store>,
    root: Option<PathBuf>,
    moved: Option<Result<PathBuf, String>>,
    session: impl FnOnce(Option<Store>) -> Session + Send + 'static,
) -> (Arc<OpenProgress>, mpsc::Receiver<Opened>) {
    let progress = store.as_ref().map_or_else(|| Arc::new(OpenProgress::finished()), Store::open_progress);
    let (tx, rx) = mpsc::channel();
    let told = progress.clone();
    let body = move || {
        let ran = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            let (mut store, mut moved) = (store, moved);
            if let Some(Err(e)) = store.as_ref().map(Store::open_data_files) {
                // Folders from before that couldn't come into data files stay as they are.
                moved = Some(Err(e.to_string()));
                store = root.and_then(|r| Store::open(r).ok());
            }
            let session = session(store.clone());
            Opened { store, session: Some(session), moved }
        }));
        // Told as the whole opening failing (no file name): the card says so in its own words,
        // and that MailCraft runs without its cache this time; what the panic said is logged.
        let opened = ran.unwrap_or_else(|p| {
            let why = panic_text(p.as_ref());
            log::error!("opening the data files: {why}");
            told.fail("", why);
            Opened { store: None, session: None, moved: None }
        });
        told.finish();
        let _ = tx.send(opened);
    };
    // Opened here, before the window, when no thread can be started.
    let slot = Arc::new(Mutex::new(Some(body)));
    let theirs = slot.clone();
    let spawned = std::thread::Builder::new().name("data-open".into()).spawn(move || {
        if let Some(body) = take(&theirs) {
            body();
        }
    });
    if let Err(e) = spawned {
        log::warn!("no thread to open the data files on ({e}): opened before the window");
        if let Some(body) = take(&slot) {
            body();
        }
    }
    (progress, rx)
}

/// Makes MailCraft's window content once its data files are open: from what they opened with,
/// and the single-instance server (kept by [`Booting`] meanwhile).
pub(crate) type Start = Box<dyn FnOnce(&egui::Context, Opened, Option<single_instance::Server>) -> App>;

/// How often a window that isn't drawn (minimized or hidden) looks whether MailCraft can come
/// into it.
const LOOK_AGAIN: std::time::Duration = std::time::Duration::from_millis(250);

/// The window until MailCraft is in it.
pub(crate) struct Booting {
    pub(crate) card: BootCard,
    pub(crate) progress: Arc<OpenProgress>,
    pub(crate) opened: mpsc::Receiver<Opened>,
    /// What MailCraft opens with, once it has come.
    pub(crate) came: Option<Opened>,
    pub(crate) start: Option<Start>,
    pub(crate) instance: Option<single_instance::Server>,
    /// Installs a downloaded update if MailCraft is closed meanwhile.
    pub(crate) updater: Option<self_update::Updater>,
    /// What later launches handed over meanwhile, opened once MailCraft is in the window.
    pub(crate) handed: Vec<String>,
    pub(crate) native_wayland: bool,
}

impl Booting {
    /// Take what MailCraft opens with, once it has come. A thread that ended without it (it
    /// couldn't have: its panics are caught) leaves MailCraft running without its cache.
    fn look(&mut self) {
        if self.came.is_some() {
            return;
        }
        match self.opened.try_recv() {
            Ok(o) => self.came = Some(o),
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => {
                self.progress.fail("", "it stopped unexpectedly".into());
                self.progress.finish();
                self.came = Some(Opened { store: None, session: None, moved: None });
            }
        }
    }

    /// Draw the card; MailCraft, once it can be in the window.
    fn ui(&mut self, ui: &mut egui::Ui) -> Option<App> {
        self.look();
        if !self.card.ui(ui, &self.progress, self.came.is_some()) {
            return None;
        }
        self.start(ui.ctx())
    }

    /// MailCraft, from what it opens with (once that has come).
    fn start(&mut self, ctx: &egui::Context) -> Option<App> {
        let (opened, start) = (self.came.take()?, self.start.take()?);
        let mut app = start(ctx, opened, self.instance.take());
        for h in std::mem::take(&mut self.handed) {
            app.app.open_handed(&h);
        }
        Some(app)
    }
}

/// The window: the card while the data files open, then MailCraft.
pub(crate) enum Window {
    Booting(Box<Booting>),
    Running(Box<App>),
}

impl eframe::App for Window {
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        match self {
            Window::Running(app) => eframe::App::logic(app.as_mut(), ctx, frame),
            // Started again meanwhile: this window comes forward, and opens what was handed over
            // once MailCraft is in it.
            Window::Booting(b) => {
                if let Some(handed) = b.instance.as_ref().and_then(single_instance::Server::take) {
                    single_instance::show(ctx, b.native_wayland);
                    b.handed.extend(handed);
                }
                b.look();
                // On a Mac a close quits through AppKit (`apple_events::quit`), as MailCraft's does
                // (nothing here asks a question that could hold the window).
                #[cfg(target_os = "macos")]
                MailApp::quit_instead_of_close(ctx, apple_events::quit);
                // Minimized or hidden, the window draws nothing (no `ui`, which asks for the next
                // frame while it draws the card): MailCraft comes into it here all the same, so
                // mail syncs and reminders come, looked for every so often until opening ends.
                // Continue for a problem is pressed in `ui`.
                if b.card.may_go(&b.progress, b.came.is_some())
                    && let Some(app) = b.start(ctx)
                {
                    *self = Window::Running(Box::new(app));
                } else if b.came.is_none() || b.progress.stage() != OpenStage::Done {
                    ctx.request_repaint_after(LOOK_AGAIN);
                }
            }
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        if let Window::Booting(b) = self
            && let Some(app) = b.ui(ui)
        {
            *self = Window::Running(Box::new(app));
        }
        if let Window::Running(app) = self {
            eframe::App::ui(app.as_mut(), ui, frame);
        }
    }

    fn on_exit(&mut self) {
        match self {
            Window::Running(app) => eframe::App::on_exit(app.as_mut()),
            // Closed before MailCraft was in it: the data files are left to the thread opening
            // them (a check stopped part-way is done again next time).
            Window::Booting(b) => {
                if let Some(server) = &b.instance {
                    server.close();
                }
                if let Some(u) = &b.updater {
                    u.closing();
                }
                // macOS: ended here, before eframe tears the window down, as MailCraft's own
                // `on_exit` does (that teardown ends a quit in a crash report there).
                #[cfg(target_os = "macos")]
                std::process::exit(0);
            }
        }
    }
}
