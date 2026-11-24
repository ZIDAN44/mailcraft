//! What a launch works out before its window opens, in the order `main` takes the steps: what
//! it was started with and where its data is ([`Launch::resolve`]), the cache
//! ([`Launch::open_store`]) and the window's options and boot card ([`window`]).

use mailcraft_ui_egui::WindowPlace;
use serde_json::Value;

use super::*;

/// What MailCraft was started with, and where its data is.
pub(crate) struct Launch {
    /// The options, as `--cmd`, `--light`, `--dark` and `--log` are read from them.
    pub(crate) args: Vec<String>,
    /// `mailto:` links and mail files to open: MailCraft as the system's mail app, or Open with;
    /// and files to send, after `--attach` (`args.rs`).
    pub(crate) handed: args::Handed,
    /// The sample mailbox (`--sample`).
    pub(crate) sample: bool,
    /// The data folder was chosen at start: `--data`, `--sample` or `MAILCRAFT_DATA`.
    pub(crate) chosen: bool,
    /// Portable (issue #4): `portable.txt` beside the app keeps everything in MailCraftData there.
    pub(crate) portable: portable::Portable,
    /// The app folder whose data files can be moved (File › Options): `None` when the data
    /// folder was chosen at start, or MailCraft is portable.
    pub(crate) base: Option<PathBuf>,
    /// The data folder (`None` when the system names no place for it).
    pub(crate) root: Option<PathBuf>,
}

impl Launch {
    /// The launch for `args` (the options, and the files to send after `--attach`; `opened`: what
    /// follows `--open`, `args.rs`).
    pub(crate) fn resolve(args: Vec<String>, opened: Option<Vec<String>>) -> Launch {
        let env_data = std::env::var_os("MAILCRAFT_DATA").is_some_and(|d| !d.is_empty());
        Launch::resolve_with(args, opened, env_data, portable::current)
    }

    /// [`Launch::resolve`], `env_data` telling whether `MAILCRAFT_DATA` names a folder, and
    /// `portable` looking for a portable copy's marker.
    fn resolve_with(args: Vec<String>, opened: Option<Vec<String>>, env_data: bool, portable: fn() -> &'static portable::Portable) -> Launch {
        let (args, attach) = args::attaching(args);
        let open = opened.unwrap_or_else(|| args::handed(&args));
        let handed = args::Handed::new(open, attach);
        let sample = args.iter().any(|a| a == "--sample");
        // Each account is a data file in Documents\MailCraft Files (as Outlook keeps .pst files
        // in Outlook Files); the account list and the app's state stay in the app folder. The
        // data files' folder can be moved (File › Options) when it's the usual one.
        let chosen = sample || args.iter().any(|a| a == "--data") || env_data;
        let portable = if chosen { portable::Portable::default() } else { portable().clone() };
        let base = (!chosen && portable.dir.is_none()).then(location::base_root).flatten();
        let data = args
            .iter()
            .position(|a| a == "--data")
            .and_then(|i| args.get(i + 1))
            .map(PathBuf::from)
            .or_else(|| portable.dir.clone())
            .or_else(mailcraft_engine::store::default_root);
        let root = data.map(|d| if sample { d.join("sample") } else { d });
        Launch { args, handed, sample, chosen, portable, base, root }
    }

    /// Whether MailCraft starts with its window put away (issue #116): kept running in the
    /// notification area and set to start there (`saved_ui`'s settings), and started with
    /// nothing to open (a link, a file or files to send bring the window up).
    pub(crate) fn starts_away(&self, saved_ui: Option<&Value>) -> bool {
        let pref = |k: &str| saved_ui.and_then(|v| v.get("prefs")?.get(k)?.as_bool()).unwrap_or(false);
        pref("closeToTray") && pref("startInTray") && self.handed.is_empty()
    }

    /// Whether `option` was given.
    pub(crate) fn has(&self, option: &str) -> bool {
        self.args.iter().any(|a| a == option)
    }

    /// Where the window's settings are kept (`ui.json` in the data folder).
    pub(crate) fn state_path(&self) -> Option<PathBuf> {
        self.root.as_ref().map(|r| r.join("ui.json"))
    }

    /// Where the data files are: none for the sample mailbox, which stays a folder of files;
    /// beside the data folder for `--data`; the usual place or where they were moved otherwise.
    /// Asked after a move waiting for this start was carried out.
    pub(crate) fn files(&self) -> Option<PathBuf> {
        (!self.sample)
            .then(|| self.root.as_ref().map(|r| self.base.as_deref().map_or_else(|| r.join(location::FILES_FOLDER), location::files_dir)))
            .flatten()
    }

    /// The cache in the data folder, its data files (`files`) not opened yet: they open behind
    /// the window (`boot.rs`). Folders from before that can't come into data files are said in
    /// `moved`; a cache that can't be opened at all, on standard error (MailCraft runs without).
    pub(crate) fn open_store(&self, files: Option<&Path>, moved: &mut Option<Result<PathBuf, String>>) -> Option<Store> {
        let root = self.root.as_ref()?;
        let opened = match files {
            // The folder layout is held alone: a second window on it (where the single-instance
            // check can't keep one out) runs without the cache rather than writing over the
            // first's.
            Some(f) => Store::packed(root, f).or_else(|e| {
                *moved = Some(Err(e.to_string()));
                Store::open_alone(root)
            }),
            None => Store::open_alone(root),
        };
        match opened {
            Ok(s) => Some(s),
            Err(e) => {
                say(&format!("the mail cache can't be opened ({e}); running without one"));
                None
            }
        }
    }
}

/// The window's options and the boot card it shows first, from the settings it was closed with
/// (`saved_ui`), the theme asked for (`theme`) and the windowing system (`x11`: Linux and the
/// BSDs).
pub(crate) fn window(
    sample: bool,
    saved_ui: Option<&Value>,
    theme: Option<bool>,
    #[cfg_attr(not(all(unix, not(target_os = "macos"))), allow(unused_variables))] x11: bool,
) -> (eframe::NativeOptions, BootCard) {
    // The main window opens where it was, at the size it was, maximized if it was.
    let main_place: Option<WindowPlace> = saved_ui.and_then(|v| v.get("windows")?.get("main").cloned()).and_then(|m| serde_json::from_value(m).ok());
    // Until MailCraft is in it, the window is in the language and the light or dark chosen.
    let dark = theme.or_else(|| saved_ui.and_then(|v| v.get("dark")?.as_bool()));
    let card = BootCard::new(saved_ui.and_then(|v| v.get("language")?.as_str()).unwrap_or(mailcraft_ui_egui::i18n::AUTO), dark);
    // In the colour and bar MailCraft will have once it's in the window.
    let prefs = saved_ui.and_then(|v| v.get("prefs"));
    let card = card.in_colours(
        prefs.and_then(|p| p.get("accent")?.as_str()).unwrap_or_default(),
        prefs.and_then(|p| p.get("plainTopBar")?.as_bool()).unwrap_or(false),
    );
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(if sample { "MailCraft – Sample mailbox" } else { "MailCraft" })
            // Matches the desktop entry (StartupWMClass) and icon name in packaging/linux.
            .with_app_id(mailcraft_ui_egui::APP_ID)
            .with_inner_size([1320.0, 820.0])
            .with_min_inner_size(mailcraft_ui_egui::MIN_WINDOW_SIZE)
            .with_icon(Arc::new(app_icon())),
        ..Default::default()
    };
    let mut options = match main_place {
        // Opened before the text size is set: at the interface's own.
        Some(p) => eframe::NativeOptions { viewport: p.apply(options.viewport, 1.0), ..options },
        None => options,
    };
    // Read here, before the window opens.
    let own_title_bar = own_title_bar(saved_ui);
    options.viewport = options.viewport.with_decorations(!own_title_bar);
    graphics::configure(&mut options);
    // Cut, Copy and Paste keys are MailCraft's, in every window: Ctrl+V attaches files copied
    // in a file manager that puts no text with them (Explorer, PCManFM; issue #10). Not in a
    // Wayland window, where egui-winit reads the clipboard without waiting on the event loop.
    #[cfg(all(unix, not(target_os = "macos")))]
    let own_keys = x11 || std::env::var_os("WAYLAND_DISPLAY").is_none();
    #[cfg(not(all(unix, not(target_os = "macos"))))]
    let own_keys = true;
    if own_keys {
        options.clipboard_shortcuts = false;
        options.viewport_hooks = Some(Arc::new(ClipboardKeys { pages: mailcraft_ui_egui::alt_code_pages() }));
    }
    // Linux and the BSDs: through XWayland on a Wayland desktop, so attachments drag in and out.
    #[cfg(all(unix, not(target_os = "macos")))]
    if x11 {
        options.event_loop_builder = Some(Box::new(|b| {
            use winit::platform::x11::EventLoopBuilderExtX11;
            b.with_x11();
        }));
    }
    (options, card)
}

/// Whether MailCraft's top bar is the window's title bar, in its colour, as Outlook's (issue #22):
/// on Windows, and on Linux and the BSDs (as GridCraft's), where the desktop's title bar sat above
/// it; unless File › Options asked for the system's. A Mac keeps its own.
fn own_title_bar(saved_ui: Option<&Value>) -> bool {
    cfg!(any(windows, all(unix, not(target_os = "macos"))))
        && !saved_ui.and_then(|v| v.get("prefs")?.get("systemTitleBar")?.as_bool()).unwrap_or(false)
}

/// The theme asked for at start: `--light` (false) before `--dark` (true).
pub(crate) fn theme_asked(args: &[String]) -> Option<bool> {
    if args.iter().any(|a| a == "--light") {
        Some(false)
    } else if args.iter().any(|a| a == "--dark") {
        Some(true)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The top bar is the window's title bar on Windows, Linux and the BSDs, unless the settings
    /// ask for the system's; a Mac keeps its own.
    #[test]
    fn the_top_bar_is_the_title_bar_but_on_a_mac() {
        let own = cfg!(any(windows, all(unix, not(target_os = "macos"))));
        assert_eq!(own_title_bar(None), own);
        assert_eq!(own_title_bar(Some(&serde_json::json!({"prefs": {"systemTitleBar": false}}))), own);
        assert!(!own_title_bar(Some(&serde_json::json!({"prefs": {"systemTitleBar": true}}))));
    }

    fn launch(args: &[&str], opened: Option<&[&str]>, env_data: bool) -> Launch {
        let strings = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        static NOT_PORTABLE: portable::Portable = portable::Portable { dir: None, unwritable: None };
        Launch::resolve_with(strings(args), opened.map(strings), env_data, || &NOT_PORTABLE)
    }

    /// Where a launch's data is, by what it was started with.
    #[test]
    fn a_launch_finds_its_data_folder() {
        let usual = launch(&[], None, false);
        assert!(!usual.chosen && !usual.sample);
        assert_eq!(usual.root, mailcraft_engine::store::default_root());
        assert_eq!(usual.base, location::base_root());
        assert_eq!(usual.state_path(), usual.root.as_ref().map(|r| r.join("ui.json")));

        let data = launch(&["--data", "D:/Mail", "--log"], None, false);
        assert_eq!((data.root.clone(), data.base.clone(), data.chosen), (Some(PathBuf::from("D:/Mail")), None, true));
        assert!(data.has("--log") && !data.has("--sample"));

        let sample = launch(&["--sample", "--data", "D:/Mail"], None, false);
        assert_eq!(sample.root, Some(PathBuf::from("D:/Mail").join("sample")));
        assert_eq!(sample.files(), None, "the sample mailbox is a folder of files");
        let data = launch(&["--data", "D:/Mail"], None, false);
        assert_eq!(data.files(), Some(PathBuf::from("D:/Mail").join(location::FILES_FOLDER)));

        // MAILCRAFT_DATA: chosen, so no move and no portable copy.
        let env = launch(&[], None, true);
        assert!(env.chosen && env.base.is_none());
        // A missing `--data` value is the usual place.
        assert_eq!(launch(&["--data"], None, false).root, mailcraft_engine::store::default_root());
    }

    /// Issue #116: MailCraft starts put away when it's kept running in the notification area
    /// and set to start there, and was started with nothing to open.
    #[test]
    fn a_launch_starts_away_when_set_to() {
        let ui = |keep: bool, start: bool| serde_json::json!({"prefs": {"closeToTray": keep, "startInTray": start}});
        assert!(launch(&[], None, false).starts_away(Some(&ui(true, true))));
        assert!(!launch(&[], None, false).starts_away(Some(&ui(false, true))), "not kept running");
        assert!(!launch(&[], None, false).starts_away(Some(&ui(true, false))), "not set to");
        assert!(!launch(&[], None, false).starts_away(None), "no settings yet");
        assert!(!launch(&["mailto:a@x.example"], None, false).starts_away(Some(&ui(true, true))), "a link to open");
        assert!(!launch(&["--attach", "/u/a.pdf"], None, false).starts_away(Some(&ui(true, true))), "files to send");
    }

    /// What's handed over comes from `--open` when MailCraft was started by the system,
    /// otherwise from the arguments that aren't options; files to send from after `--attach`,
    /// whose options before it are read and nothing after it (issue #96).
    #[test]
    fn a_launch_hands_over_its_links() {
        let open = |a: &[&str]| args::Handed::opening(a.iter().map(|s| s.to_string()).collect());
        assert_eq!(launch(&[], Some(&["mailto:a@x.example"]), false).handed, open(&["mailto:a@x.example"]));
        assert_eq!(launch(&["--sample", "mailto:b@x.example"], None, false).handed, open(&["mailto:b@x.example"]));
        let sending = launch(&["--sample", "--attach", "/u/a.pdf", "--data"], None, false);
        assert!(sending.sample && sending.chosen && sending.handed.open.is_empty(), "the options before it");
        assert_eq!(sending.handed.attach.len(), 1, "one message");
        assert_eq!(sending.handed.attach[0].len(), 2, "`--data` after it is a file");
        assert!(!sending.has("--data") && sending.root.as_deref().is_some_and(|r| r.ends_with("sample")), "not an option");
        assert!(launch(&["--sample", "--attach"], None, false).handed.is_empty());
        assert_eq!(theme_asked(&["--dark".into(), "--light".into()]), Some(false));
        assert_eq!(theme_asked(&["--dark".into()]), Some(true));
        assert_eq!(theme_asked(&["--sample".into()]), None);
    }
}
