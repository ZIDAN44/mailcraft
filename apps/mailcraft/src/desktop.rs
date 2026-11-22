//! The desktop's side of the window: the network (the worker pool) and the platform's
//! services (`mailcraft_ui_egui::Services`): file dialogs, opening files and links, video frames,
//! notifications, the data folder, the updater, Craft apps, threads for slow work.

use super::*;

pub(crate) mod agents;
pub(crate) mod boot;
pub(crate) mod fonts;
mod lanes;
pub(crate) mod launch;
pub(crate) mod start;
pub(crate) mod state_writer;
pub(crate) mod tray;
pub(super) mod video;
pub(crate) mod video_frames;
pub(crate) mod window_frame;

pub(crate) struct PoolBackend(pub(crate) Pool);

impl Backend for PoolBackend {
    fn submit(&mut self, job: Job) {
        self.0.submit(job);
    }
    fn poll(&mut self) -> Vec<Event> {
        self.0.poll()
    }
    fn shutdown(&mut self) {
        self.0.shutdown();
    }
}

#[derive(Default)]
pub(crate) struct Desktop {
    /// Video frames being made, a few at a time.
    pub(crate) frames: video_frames::VideoFrames,
    /// The data files' folder in use, and the app folder whose data files can be moved (`None`
    /// when the place was chosen at start: `--data`, `--sample`, `MAILCRAFT_DATA`).
    pub(crate) data: Option<PathBuf>,
    pub(crate) base: Option<PathBuf>,
    /// How a move carried out at this start went (told once).
    pub(crate) moved: Option<Result<PathBuf, String>>,
    /// Portable mode (`portable.rs`): MailCraft's name isn't written to Windows' registry.
    #[cfg(windows)]
    pub(crate) portable: bool,
    /// Test builds' updater (`self_update.rs`), when this one has it.
    pub(crate) updater: Option<self_update::Updater>,
    /// The desktop's light or dark preference where winit doesn't report one (`appearance.rs`).
    pub(crate) dark: appearance::SystemDark,
    /// The folder of the last file picked or saved: the next file dialog opens there (attaching
    /// several files from one folder, saving attachments beside each other), as figcraft's do.
    pub(crate) last_dir: Option<PathBuf>,
    /// The threads slow work runs on (decoding pictures).
    pub(crate) lanes: lanes::Lanes,
    /// The window draws its own title bar (Windows, issue #22), and what keeps it borderless.
    pub(crate) own_title_bar: bool,
    pub(crate) frame_guard: Option<window_frame::FrameGuard>,
    /// The window opened on X11 (Linux and the BSDs, `window/display.rs`): through XWayland on a
    /// Wayland desktop too, where attachments drag out as on X11 itself.
    pub(crate) x11: bool,
}

impl Desktop {
    /// A file dialog titled `title`, open at the folder last used when it's still there.
    fn dialog(&self, title: &str) -> rfd::FileDialog {
        let d = rfd::FileDialog::new().set_title(title);
        match self.last_dir.as_ref().filter(|d| d.is_dir()) {
            Some(dir) => d.set_directory(dir),
            None => d,
        }
    }

    /// Notes where a dialog's answer was (a file's folder, or the folder picked); passes it on.
    fn used<T: AsRef<Path>>(&mut self, picked: Option<T>, is_folder: bool) -> Option<T> {
        if let Some(p) = &picked {
            let p = p.as_ref();
            self.last_dir = if is_folder { Some(p.to_path_buf()) } else { p.parent().map(Path::to_path_buf) }.or(self.last_dir.take());
        }
        picked
    }
}

impl Services for Desktop {
    fn own_title_bar(&self) -> bool {
        self.own_title_bar
    }

    fn keep_frame(&mut self) {
        if let Some(guard) = &self.frame_guard {
            guard.keep();
        }
    }

    fn before_zoom(&mut self) {
        if let Some(guard) = &self.frame_guard {
            guard.before_zoom();
        }
    }

    /// The system's cursor, in physical pixels (winit makes MailCraft per-monitor DPI aware).
    #[cfg(target_os = "windows")]
    fn pointer_on_screen(&mut self) -> Option<[i32; 2]> {
        winsafe::GetCursorPos().ok().map(|p| [p.x, p.y])
    }

    fn clock(&self) -> (i64, i32) {
        let now = chrono::Local::now();
        (now.timestamp(), now.offset().local_minus_utc())
    }
    fn zone(&self) -> Option<String> {
        iana_time_zone::get_timezone().ok()
    }
    fn system_dark(&self) -> Option<bool> {
        self.dark.get()
    }
    fn pick_files(&mut self) -> Vec<PathBuf> {
        let files = self.dialog("Attach files").pick_files().unwrap_or_default();
        self.used(files.first(), false);
        files
    }
    fn save_path(&mut self, suggested: &str) -> Option<PathBuf> {
        let mut d = self.dialog("Save as").set_file_name(suggested);
        // Save as type: the suggested name's kind first, so the system keeps its extension.
        if let Some((_, ext)) = suggested.rsplit_once('.').filter(|(_, e)| !e.is_empty() && e.len() <= 8) {
            let kind = match ext.to_ascii_lowercase().as_str() {
                "pst" => "Outlook data file".to_string(),
                "eml" => "Email message".to_string(),
                "pdf" => "PDF document".to_string(),
                "ics" => "Calendar".to_string(),
                "vcf" => "Contact card".to_string(),
                "txt" => "Text".to_string(),
                "json" => "MailCraft rules".to_string(),
                "html" | "htm" => "Web page".to_string(),
                e => format!("{} file", e.to_ascii_uppercase()),
            };
            d = d.add_filter(kind, &[ext]).add_filter("All files", &["*"]);
        }
        let saved = d.save_file();
        self.used(saved, false)
    }
    fn open_path(&mut self, path: &Path) -> Result<(), String> {
        shell_open(path.as_os_str())
    }
    fn update_state(&mut self) -> mailcraft_ui_egui::UpdateState {
        self.updater.as_ref().map(self_update::Updater::state).unwrap_or_default()
    }
    fn restart_to_update(&mut self) -> bool {
        self.updater.as_ref().is_some_and(self_update::Updater::restart_after_close)
    }
    fn keep_update_for_later(&mut self) {
        if let Some(u) = &self.updater {
            u.keep_for_later();
        }
    }
    fn data_folder(&self) -> Option<(PathBuf, bool)> {
        self.data.clone().map(|d| (d, self.base.is_some()))
    }
    fn take_data_moved(&mut self) -> Option<Result<PathBuf, String>> {
        self.moved.take()
    }
    fn pending_data_move(&self) -> Option<PathBuf> {
        self.base.as_deref().and_then(location::pending_move)
    }
    fn carried_over_from(&self) -> Option<&'static str> {
        crate::self_update::renamed::carried_over().then_some(crate::self_update::renamed::OLD_ID)
    }
    fn move_data_folder(&mut self, to: Option<&Path>) -> Result<Option<PathBuf>, String> {
        let base = self.base.as_deref().ok_or("the data folder was chosen when MailCraft started")?;
        match to {
            Some(to) => location::schedule_move(base, to).map(Some),
            None => {
                location::cancel_move(base);
                Ok(None)
            }
        }
    }
    fn craft_app(&mut self, command: &str) -> Option<PathBuf> {
        let exe = if cfg!(windows) { format!("{command}.exe") } else { command.to_string() };
        // On the PATH (the CLI installers and Linux packages put it there)…
        if let Some(paths) = std::env::var_os("PATH") {
            for dir in std::env::split_paths(&paths) {
                let p = dir.join(&exe);
                if p.is_file() {
                    return Some(p);
                }
            }
        }
        // …or where each platform's installer puts apps.
        let title = match command {
            "pdfcraft" => "PdfCraft",
            "wordcraft" => "WordCraft",
            "gridcraft" => "GridCraft",
            "deckcraft" => "DeckCraft",
            "photocraft" => "PhotoCraft",
            "filmcraft" => "FilmCraft",
            "soundcraft" => "SoundCraft",
            _ => return None,
        };
        let mut places: Vec<PathBuf> = Vec::new();
        if cfg!(windows) {
            for var in ["ProgramFiles", "LOCALAPPDATA"] {
                if let Some(base) = std::env::var_os(var) {
                    let base = PathBuf::from(base);
                    places.push(base.join(title).join(&exe));
                    places.push(base.join("Programs").join(title).join(&exe));
                }
            }
        } else if cfg!(target_os = "macos") {
            places.push(PathBuf::from(format!("/Applications/{title}.app")));
            // Installed for one user (an account that can't write /Applications).
            if let Some(home) = std::env::var_os("HOME") {
                places.push(PathBuf::from(home).join("Applications").join(format!("{title}.app")));
            }
        } else {
            places.push(PathBuf::from(format!("/opt/{command}/bin/{command}")));
        }
        places.into_iter().find(|p| p.exists())
    }
    fn open_with(&mut self, app: &Path, file: &Path) -> Result<(), String> {
        let mut cmd = if cfg!(target_os = "macos") && app.extension().is_some_and(|e| e == "app") {
            let mut c = std::process::Command::new("open");
            c.arg("-a").arg(app).arg(file);
            c
        } else {
            let mut c = std::process::Command::new(app);
            c.arg(file);
            c
        };
        cmd.spawn().map(|_| ()).map_err(|e| e.to_string())
    }
    fn cjk_font(&mut self, lang: &str) -> Option<(Vec<u8>, u32)> {
        // The language's own font first (Chinese, Japanese and Korean draw shared characters
        // differently), then any that has the characters at all.
        let (windows, noto_index): (&[&str], u32) = match lang {
            "ja" => (&["YuGothM.ttc", "msgothic.ttc"], 0),
            "ko" => (&["malgun.ttf", "gulim.ttc"], 1),
            "zh-hant" => (&["msjh.ttc", "mingliu.ttc"], 3),
            _ => (&["msyh.ttc", "simsun.ttc"], 2),
        };
        let mut candidates: Vec<(PathBuf, u32)> = Vec::new();
        if cfg!(target_os = "windows") {
            let fonts = windows_fonts_dir();
            for f in windows.iter().chain(["msyh.ttc", "YuGothM.ttc", "malgun.ttf"].iter()) {
                candidates.push((fonts.join(f), 0));
            }
        } else if cfg!(target_os = "macos") {
            candidates.extend(macos_cjk_fonts(lang).into_iter().map(|p| (p, 0)));
        } else {
            // Noto Sans CJK collections hold Japanese, Korean, Simplified and Traditional faces.
            for p in [
                "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
                "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
                "/usr/share/fonts/google-noto-cjk/NotoSansCJK-Regular.ttc",
            ] {
                candidates.push((PathBuf::from(p), noto_index));
            }
            candidates.push((PathBuf::from("/usr/share/fonts/truetype/droid/DroidSansFallbackFull.ttf"), 0));
        }
        candidates.iter().find_map(|(p, i)| std::fs::read(p).ok().map(|b| (b, *i)))
    }
    fn rtl_font(&mut self) -> Option<Vec<u8>> {
        // Fonts with both Arabic and Hebrew first.
        let candidates: Vec<PathBuf> = if cfg!(target_os = "windows") {
            let fonts = windows_fonts_dir();
            vec![fonts.join("tahoma.ttf"), fonts.join("arial.ttf")]
        } else if cfg!(target_os = "macos") {
            ["/System/Library/Fonts/Supplemental/Arial.ttf", "/Library/Fonts/Arial.ttf", "/System/Library/Fonts/Supplemental/Tahoma.ttf"]
                .map(PathBuf::from)
                .into()
        } else {
            [
                "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
                "/usr/share/fonts/dejavu/DejaVuSans.ttf",
                "/usr/share/fonts/TTF/DejaVuSans.ttf",
                "/usr/share/fonts/truetype/noto/NotoSansArabic-Regular.ttf",
                "/usr/share/fonts/noto/NotoSansArabic-Regular.ttf",
            ]
            .map(PathBuf::from)
            .into()
        };
        candidates.iter().find_map(|p| std::fs::read(p).ok())
    }
    fn keeps_running(&self) -> Option<mailcraft_ui_egui::KeptOnClose> {
        tray::offered()
    }
    fn kept_on_close(&self) -> Option<mailcraft_ui_egui::KeptOnClose> {
        tray::kept_on_close()
    }
    fn can_drag_out(&self) -> bool {
        // Elsewhere, from a window on X11 only (a Wayland window isn't an X window). That's the
        // window's own: on a Wayland desktop it opens through XWayland, `WAYLAND_DISPLAY` still
        // set, and attachments didn't drag out there at all (#120).
        cfg!(any(target_os = "windows", target_os = "macos")) || self.x11
    }
    fn video_frame(&mut self, path: &Path) -> VideoFrame {
        self.frames.frame(path)
    }
    fn background(&mut self, lane: mailcraft_ui_egui::Lane, work: mailcraft_ui_egui::Work) -> Result<(), mailcraft_ui_egui::Work> {
        self.lanes.submit(lane, work)
    }
    fn has_background(&self) -> bool {
        self.lanes.available()
    }
    fn tend_background(&mut self) {
        self.lanes.tend();
    }
    fn pick_folder(&mut self) -> Option<PathBuf> {
        let folder = self.dialog("Save attachments to").pick_folder();
        self.used(folder, true)
    }
    fn pick_certificate(&mut self) -> Option<PathBuf> {
        let file =
            self.dialog("Import certificate").add_filter("Certificate with its key", &["p12", "pfx"]).add_filter("All files", &["*"]).pick_file();
        self.used(file, false)
    }
    fn pick_server_certificate(&mut self) -> Option<PathBuf> {
        let file = self
            .dialog("Trust a certificate file")
            .add_filter("Certificate", &["pem", "crt", "cer", "der"])
            .add_filter("All files", &["*"])
            .pick_file();
        self.used(file, false)
    }
    fn pick_pgp_key(&mut self) -> Option<PathBuf> {
        let file =
            self.dialog("Import OpenPGP key").add_filter("OpenPGP key", &["asc", "gpg", "key", "pgp"]).add_filter("All files", &["*"]).pick_file();
        self.used(file, false)
    }
    fn pick_rules_file(&mut self) -> Option<PathBuf> {
        let file = self.dialog("Import rules").add_filter("MailCraft rules", &["json"]).add_filter("All files", &["*"]).pick_file();
        self.used(file, false)
    }
    fn pick_categories_file(&mut self) -> Option<PathBuf> {
        let file = self.dialog("Import categories").add_filter("MailCraft categories", &["json"]).add_filter("All files", &["*"]).pick_file();
        self.used(file, false)
    }
    fn pick_data_file(&mut self) -> Option<PathBuf> {
        let mut d = rfd::FileDialog::new();
        // Where the data files are (a removed account's stays there), as Outlook opens its own.
        if let Some(dir) = self.data.as_ref().filter(|d| d.is_dir()) {
            d = d.set_directory(dir);
        }
        d.set_title("Open data file")
            .add_filter("Data files", &["pst", "mailcraft"])
            .add_filter("Outlook data files", &["pst"])
            .add_filter("MailCraft data files", &["mailcraft"])
            .pick_file()
    }
    fn pick_mail_file(&mut self) -> Option<PathBuf> {
        let file = self.dialog("Open mail file").add_filter("Mail files", &["eml", "msg"]).add_filter("All files", &["*"]).pick_file();
        self.used(file.as_ref(), false);
        file
    }
    fn pick_pictures(&mut self) -> Vec<PathBuf> {
        let files =
            self.dialog("Insert pictures").add_filter("Pictures", &["png", "jpg", "jpeg", "gif", "webp", "bmp"]).pick_files().unwrap_or_default();
        self.used(files.first(), false);
        files
    }
    fn clipboard_picture(&mut self) -> Option<Vec<u8>> {
        let img = read_clipboard(|c| c.get_image().ok())?;
        let (w, h) = (u32::try_from(img.width).ok()?, u32::try_from(img.height).ok()?);
        let rgba = image::RgbaImage::from_raw(w, h, img.bytes.into_owned())?;
        let mut png = Vec::new();
        image::DynamicImage::ImageRgba8(rgba).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).ok()?;
        Some(png)
    }
    fn clipboard_files(&mut self) -> Vec<PathBuf> {
        crate::clipboard_files()
    }
    fn open_url(&mut self, url: &str) -> Result<(), String> {
        // Web addresses go to Windows' URL handler: ShellExec_RunDLL (right for files) started
        // and quietly showed nothing, so sign-in pages never reached the browser.
        #[cfg(windows)]
        if url.starts_with("https://") || url.starts_with("http://") || url.starts_with("mailto:") {
            return std::process::Command::new("rundll32.exe")
                .arg("url.dll,FileProtocolHandler")
                .arg(url)
                .spawn()
                .map(drop)
                .map_err(|e| e.to_string());
        }
        shell_open(std::ffi::OsStr::new(url))
    }
    fn notify(&mut self, title: &str, body: &str) {
        let (title, body) = (title.to_string(), body.to_string());
        #[cfg(windows)]
        let portable = self.portable;
        // Off the UI thread: the notification service can be slow, or missing.
        off_thread(std::thread::Builder::new().name("notify".into()), "notification", move || {
            let mut n = notify_rust::Notification::new();
            n.summary(&title).body(&body).appname("MailCraft");
            // Windows names a toast after its app id (PowerShell's when none is given). A
            // portable copy writes nothing to the registry: it uses an installed MailCraft's.
            #[cfg(windows)]
            if (!portable && toast::registered()) || (portable && toast::installed()) {
                n.app_id(toast::APP_ID);
            }
            if let Err(e) = n.show() {
                log::warn!("notification: {e}");
            }
        });
    }
}

/// Windows' Fonts folder, in the Windows folder (`%WINDIR%`), which needn't be `C:\Windows`.
pub(crate) fn windows_fonts_dir() -> PathBuf {
    windows_fonts_dir_in(std::env::var_os("WINDIR"))
}

/// [`windows_fonts_dir`] with `windir` for `%WINDIR%` (`C:\Windows` when it's unset, empty or
/// relative).
fn windows_fonts_dir_in(windir: Option<std::ffi::OsString>) -> PathBuf {
    windir.map(PathBuf::from).filter(|d| d.is_absolute()).unwrap_or_else(|| PathBuf::from("C:\\Windows")).join("Fonts")
}

/// Where macOS 26 keeps PingFang, out of `/System/Library/Fonts`, where it was before: its first
/// face is the Simplified Chinese one.
const MACOS_PINGFANG_UI: &str = "/System/Library/PrivateFrameworks/FontServices.framework/Resources/Reserved/PingFangUI.ttc";

/// The Mac's fonts for Chinese, Japanese or Korean (`lang`), first preferred: the language's own,
/// then any that has the characters at all. PingFang moved in macOS 26 (PdfCraft found it): where
/// `PingFang.ttc` is gone, Chinese came out in a Japanese or Korean font. For Traditional Chinese,
/// STHeiti comes before the moved PingFang's first face, which is Simplified.
fn macos_cjk_fonts(lang: &str) -> Vec<PathBuf> {
    let own: &[&str] = match lang {
        "ja" => &["ヒラギノ角ゴシック W3.ttc", "Hiragino Sans GB.ttc"],
        "ko" => &["AppleSDGothicNeo.ttc"],
        "zh-hant" => &["PingFang.ttc", "STHeiti Light.ttc", "STHeiti Medium.ttc", MACOS_PINGFANG_UI],
        _ => &["PingFang.ttc", MACOS_PINGFANG_UI, "Hiragino Sans GB.ttc"],
    };
    let fonts = Path::new("/System/Library/Fonts");
    let mut list: Vec<PathBuf> = Vec::new();
    for f in own.iter().chain(&["PingFang.ttc", MACOS_PINGFANG_UI, "AppleSDGothicNeo.ttc"]) {
        // An absolute path is taken as it is.
        let path = fonts.join(f);
        if !list.contains(&path) {
            list.push(path);
        }
    }
    list
}

/// Run `f` on a thread of its own made by `builder`: false, and `what` logged, when the system
/// won't start one (short of resources), where `std::thread::spawn` panicked on the window's
/// thread.
fn off_thread(builder: std::thread::Builder, what: &str, f: impl FnOnce() + Send + 'static) -> bool {
    match builder.spawn(f) {
        Ok(_) => true,
        Err(e) => {
            log::warn!("{what}: no thread to run it on ({e})");
            false
        }
    }
}

/// Open a file or a web address with the system's choice of app, as Explorer would.
///
/// On Windows this goes through `rundll32 shell32.dll,ShellExec_RunDLL`, a process of its own:
/// opening from MailCraft (directly, or through the hidden PowerShell the `open` crate starts)
/// silently did nothing for apps such as Acrobat, and a file no app opens gets Windows' "How do
/// you want to open this file?", as Outlook does.
pub(crate) fn shell_open(target: &std::ffi::OsStr) -> Result<(), String> {
    #[cfg(windows)]
    {
        // A folder opens in Explorer: ShellExec_RunDLL quietly does nothing with one (Options ›
        // Data files › Open folder never opened).
        if std::path::Path::new(target).is_dir() {
            return std::process::Command::new("explorer.exe").arg(target).spawn().map(drop).map_err(|e| e.to_string());
        }
        std::process::Command::new("rundll32.exe").arg("shell32.dll,ShellExec_RunDLL").arg(target).spawn().map(drop).map_err(|e| e.to_string())
    }
    #[cfg(not(windows))]
    {
        open::that_detached(target).map_err(|e| e.to_string())
    }
}

/// Most bytes a picture taken from Office's clipboard folder may have.
const OFFICE_PICTURE_MAX: u64 = 1 << 20;

/// Copied HTML with the pictures Office wrote as files of its own written into it instead
/// (`data:`): older Word and Outlook point a copied logo at `file:///…/msohtmlclip1/01/clip_image001.png`,
/// which the paste would leave out (issue #11). Only files in Office's clipboard folder, in the
/// temporary folder, are read, and only pictures up to 1 MB: copied HTML from anywhere else (a
/// web page) can't have a file of the user's attached.
pub(crate) fn office_pictures(html: String) -> String {
    office_pictures_in(html, &std::env::temp_dir())
}

fn office_pictures_in(html: String, temp: &Path) -> String {
    let Ok(temp) = temp.canonicalize() else { return html };
    mailcraft_engine::mime::local_pictures(&html, |src| {
        let path = office_file(src)?.canonicalize().ok()?;
        let rest = path.strip_prefix(&temp).ok()?;
        let first = rest.components().next()?.as_os_str().to_string_lossy().to_ascii_lowercase();
        let meta = std::fs::metadata(&path).ok()?;
        if !first.starts_with("msohtmlclip") || !meta.is_file() || meta.len() > OFFICE_PICTURE_MAX {
            return None;
        }
        let name = path.file_name()?.to_string_lossy().to_string();
        Some((mailcraft_engine::mime::mime_from_name(&name).to_string(), std::fs::read(&path).ok()?))
    })
}

/// The file on this computer a `file:` address names (`file:///C:/Users/…/clip_image001.png`,
/// with `%20` and the like decoded), read as a `file:` link MailCraft is started with is;
/// `None` for any other address, or one to another computer.
fn office_file(src: &str) -> Option<PathBuf> {
    let src = src.trim();
    src.get(..5).filter(|s| s.eq_ignore_ascii_case("file:"))?;
    crate::args::from_file_link(src).map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Chinese on macOS 26, where PingFang is in another folder, comes in PingFang still, never in
    /// a Japanese or Korean font first; Traditional Chinese in STHeiti before the moved PingFang,
    /// whose first face is Simplified. Each font is looked for once.
    #[test]
    fn the_macs_chinese_font_is_found_where_macos_26_keeps_it() {
        let at = |list: &[PathBuf], name: &str| list.iter().position(|p| p.ends_with(name));
        let moved = PathBuf::from(MACOS_PINGFANG_UI);
        let simplified = macos_cjk_fonts("zh-hans");
        assert_eq!(simplified[0], Path::new("/System/Library/Fonts/PingFang.ttc"));
        assert_eq!(simplified[1], moved);
        assert!(at(&simplified, "Hiragino Sans GB.ttc") > Some(1) && at(&simplified, "AppleSDGothicNeo.ttc") > Some(1));
        let traditional = macos_cjk_fonts("zh-hant");
        assert!(at(&traditional, "STHeiti Medium.ttc") < traditional.iter().position(|p| *p == moved));
        assert!(traditional.iter().position(|p| *p == moved) < at(&traditional, "AppleSDGothicNeo.ttc"));
        for lang in ["ja", "ko", "zh-hans", "zh-hant"] {
            let list = macos_cjk_fonts(lang);
            assert!(list.contains(&moved), "{lang}");
            assert!(list.iter().enumerate().all(|(i, p)| !list[..i].contains(p)), "{lang}: {list:?}");
        }
        assert_eq!(macos_cjk_fonts("ko")[0], Path::new("/System/Library/Fonts/AppleSDGothicNeo.ttc"));
    }

    /// A thread the system won't start (here a stack no system gives) is said, not a panic on
    /// the window's thread, as `std::thread::spawn` made it for each new-mail notification.
    #[cfg(target_pointer_width = "64")]
    #[test]
    fn a_thread_that_cant_start_isnt_a_panic() {
        let (tx, rx) = std::sync::mpsc::channel();
        assert!(off_thread(std::thread::Builder::new(), "test", move || tx.send(()).unwrap_or_default()));
        assert!(rx.recv_timeout(std::time::Duration::from_secs(10)).is_ok(), "ran");
        assert!(!off_thread(std::thread::Builder::new().stack_size(1 << 50), "test", || {}));
    }

    /// Found in audit (#229): the UI and right-to-left fonts were looked for in `C:\Windows`
    /// whatever folder Windows is in (the CJK fonts in `%WINDIR%`). All three come from the
    /// Windows folder now; `C:\Windows` only when `%WINDIR%` says nothing usable.
    #[test]
    fn fonts_come_from_the_windows_folder() {
        let elsewhere = if cfg!(windows) { r"D:\Windows" } else { "/opt/windows" };
        assert_eq!(windows_fonts_dir_in(Some(elsewhere.into())), Path::new(elsewhere).join("Fonts"));
        for unusable in [None, Some("".into()), Some("Windows".into())] {
            assert_eq!(windows_fonts_dir_in(unusable), PathBuf::from("C:\\Windows").join("Fonts"));
        }
        #[cfg(windows)]
        assert!(windows_fonts_dir().is_dir(), "{}", windows_fonts_dir().display());
    }

    /// The next dialog opens where the last answer was: a file's folder, or the folder picked;
    /// a cancelled dialog leaves it, and a folder that's gone isn't offered.
    #[test]
    fn dialogs_open_where_the_last_one_was() {
        let root = std::env::temp_dir().join(format!("mailcraft-dialogs-{}", std::process::id()));
        let (docs, out) = (root.join("Docs"), root.join("Out"));
        std::fs::create_dir_all(&docs).unwrap();
        std::fs::create_dir_all(&out).unwrap();
        let mut d = Desktop::default();
        assert_eq!(d.used(Some(docs.join("plan.pdf")), false), Some(docs.join("plan.pdf")));
        assert_eq!(d.last_dir.as_deref(), Some(docs.as_path()));
        d.used(None::<PathBuf>, false);
        assert_eq!(d.last_dir.as_deref(), Some(docs.as_path()), "cancelled: kept");
        d.used(Some(out.clone()), true);
        assert_eq!(d.last_dir.as_deref(), Some(out.as_path()));
        let _ = d.dialog("Attach files");
        std::fs::remove_dir_all(&root).unwrap();
        assert!(d.last_dir.as_ref().is_some_and(|p| !p.is_dir()), "gone: the dialog opens where the system says");
        let _ = d.dialog("Attach files");
    }

    /// Issue #120: on a Wayland desktop the window opens through XWayland, where attachments drag
    /// out as on X11, but they didn't drag at all: dragging was told from `WAYLAND_DISPLAY`, which
    /// is still set there. It follows the window: on X11 they drag out, on Wayland not (winit can't
    /// start a drag there), and on Windows and macOS always.
    #[test]
    fn attachments_drag_out_of_a_window_on_x11() {
        let drags = |x11| Desktop { x11, ..Default::default() }.can_drag_out();
        assert!(drags(true));
        assert_eq!(drags(false), cfg!(any(target_os = "windows", target_os = "macos")));
    }

    /// Issue #11: a logo older Word and Outlook copy as a file of their own comes along as a
    /// picture written into the HTML; one anywhere else, or not a picture, or too big, doesn't.
    #[test]
    fn office_pictures_from_its_clipboard_folder_come_along() {
        let temp = std::env::temp_dir().join(format!("mailcraft-office-{}", std::process::id()));
        let clip = temp.join("msohtmlclip1").join("01");
        std::fs::create_dir_all(&clip).unwrap();
        let png = mailcraft_engine::sample::sample_png();
        std::fs::write(clip.join("clip image001.png"), &png).unwrap();
        std::fs::write(clip.join("notes.txt"), b"hello").unwrap();
        std::fs::write(clip.join("big.png"), vec![0u8; (OFFICE_PICTURE_MAX + 1) as usize]).unwrap();
        std::fs::write(temp.join("secret.png"), &png).unwrap();
        let url = |p: &Path| format!("file:///{}", p.to_string_lossy().replace('\\', "/").trim_start_matches('/').replace(' ', "%20"));
        let html = format!(
            "<p><img width=180 height=54\r\nsrc=\"{}\" v:shapes=x><img src='{}'><img src={}><img src=\"{}\"><b>Riley</b></p>",
            url(&clip.join("clip image001.png")),
            url(&temp.join("secret.png")),
            url(&clip.join("notes.txt")),
            url(&clip.join("big.png")),
        );
        let out = office_pictures_in(html, &temp);
        assert_eq!(out.matches("data:image/png;base64,").count(), 1, "{out}");
        assert!(out.contains("<img width=180 height=54\r\nsrc=\"data:image/png;base64,"), "{out}");
        assert!(out.contains("secret.png") && out.contains("notes.txt") && out.contains("big.png"), "the others left as they were: {out}");
        let (text, styles, pictures) = mailcraft_engine::mime::signature_html(&out);
        assert_eq!((text.as_str(), pictures.len()), ("\u{FFFC}Riley", 1));
        assert_eq!((pictures[0].data.clone(), styles.style(0).picture_width), (png, Some(180)));
        // Nothing to do: as it was.
        assert_eq!(
            office_pictures_in("<p>hi <img src=\"https://x.example/a.png\"></p>".into(), &temp),
            "<p>hi <img src=\"https://x.example/a.png\"></p>"
        );
        assert_eq!(office_file("http://x/a.png"), None);
        std::fs::remove_dir_all(&temp).unwrap();
    }

    /// Found in audit (#225): a picture's `file:` address is read as the `file:` links MailCraft
    /// is started with are. One to another computer was taken as a path from the folder
    /// MailCraft runs in (`server.example/share/x.png`).
    #[test]
    fn office_pictures_are_read_from_this_computer_only() {
        assert_eq!(office_file("file://server.example/share/x.png"), None);
        assert_eq!(office_file("FILE://Server/msohtmlclip1/01/a.png"), None);
        let (link, path) = if cfg!(windows) {
            (
                "file:///C:/Users/Zoe/AppData/Local/Temp/msohtmlclip1/01/clip%20image001.png",
                "C:/Users/Zoe/AppData/Local/Temp/msohtmlclip1/01/clip image001.png",
            )
        } else {
            ("file:///tmp/msohtmlclip1/01/clip%20image001.png", "/tmp/msohtmlclip1/01/clip image001.png")
        };
        assert_eq!(office_file(link), Some(PathBuf::from(path)));
        assert_eq!(office_file(&link.replace("file:///", "  File://localhost/")), Some(PathBuf::from(path)));
        assert_eq!(office_file("file:"), None);
        assert_eq!(office_file("files:///tmp/a.png"), None);
    }
}
