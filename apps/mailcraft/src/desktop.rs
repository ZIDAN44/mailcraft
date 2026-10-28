//! The desktop's side of the window: the network (the worker pool) and the platform's
//! services (`mailcraft_ui_egui::Services`): file dialogs, opening files and links, video frames,
//! notifications, the data folder, the updater, Craft apps.

use super::*;

pub(crate) struct PoolBackend(pub(crate) Pool);

impl Backend for PoolBackend {
    fn submit(&mut self, job: Job) {
        self.0.submit(job);
    }
    fn poll(&mut self) -> Vec<Event> {
        self.0.poll()
    }
}

#[derive(Default)]
pub(crate) struct Desktop {
    /// Where video frames come from (looked up once).
    pub(crate) frame_source: Option<Option<video::Source>>,
    /// Video frames being made or made, by file.
    pub(crate) frames: std::collections::HashMap<PathBuf, Arc<std::sync::Mutex<Option<VideoFrame>>>>,
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
        let (windows, macos, noto_index): (&[&str], &[&str], u32) = match lang {
            "ja" => (&["YuGothM.ttc", "msgothic.ttc"], &["ヒラギノ角ゴシック W3.ttc", "Hiragino Sans GB.ttc"], 0),
            "ko" => (&["malgun.ttf", "gulim.ttc"], &["AppleSDGothicNeo.ttc"], 1),
            "zh-hant" => (&["msjh.ttc", "mingliu.ttc"], &["PingFang.ttc"], 3),
            _ => (&["msyh.ttc", "simsun.ttc"], &["PingFang.ttc", "Hiragino Sans GB.ttc"], 2),
        };
        let mut candidates: Vec<(String, u32)> = Vec::new();
        if cfg!(target_os = "windows") {
            let dir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into());
            for f in windows.iter().chain(["msyh.ttc", "YuGothM.ttc", "malgun.ttf"].iter()) {
                candidates.push((format!("{dir}\\Fonts\\{f}"), 0));
            }
        } else if cfg!(target_os = "macos") {
            for f in macos.iter().chain(["PingFang.ttc", "AppleSDGothicNeo.ttc"].iter()) {
                candidates.push((format!("/System/Library/Fonts/{f}"), 0));
            }
        } else {
            // Noto Sans CJK collections hold Japanese, Korean, Simplified and Traditional faces.
            for p in [
                "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
                "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
                "/usr/share/fonts/google-noto-cjk/NotoSansCJK-Regular.ttc",
            ] {
                candidates.push((p.to_string(), noto_index));
            }
            candidates.push(("/usr/share/fonts/truetype/droid/DroidSansFallbackFull.ttf".into(), 0));
        }
        candidates.iter().find_map(|(p, i)| std::fs::read(p).ok().map(|b| (b, *i)))
    }
    fn rtl_font(&mut self) -> Option<Vec<u8>> {
        // Fonts with both Arabic and Hebrew first.
        let candidates: &[&str] = if cfg!(target_os = "windows") {
            &["C:\\Windows\\Fonts\\tahoma.ttf", "C:\\Windows\\Fonts\\arial.ttf"]
        } else if cfg!(target_os = "macos") {
            &["/System/Library/Fonts/Supplemental/Arial.ttf", "/Library/Fonts/Arial.ttf", "/System/Library/Fonts/Supplemental/Tahoma.ttf"]
        } else {
            &[
                "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
                "/usr/share/fonts/dejavu/DejaVuSans.ttf",
                "/usr/share/fonts/TTF/DejaVuSans.ttf",
                "/usr/share/fonts/truetype/noto/NotoSansArabic-Regular.ttf",
                "/usr/share/fonts/noto/NotoSansArabic-Regular.ttf",
            ]
        };
        candidates.iter().find_map(|p| std::fs::read(p).ok())
    }
    fn can_drag_out(&self) -> bool {
        // Elsewhere, under X11 only: a Wayland window isn't an X window.
        cfg!(any(target_os = "windows", target_os = "macos"))
            || (std::env::var_os("DISPLAY").is_some() && std::env::var_os("WAYLAND_DISPLAY").is_none())
    }
    fn video_frame(&mut self, path: &Path) -> VideoFrame {
        if let Some(slot) = self.frames.get(path) {
            return slot.lock().map(|g| g.clone().unwrap_or(VideoFrame::Pending)).unwrap_or(VideoFrame::Unavailable);
        }
        let source = self.frame_source.get_or_insert_with(video::source).clone();
        let Some(source) = source else { return VideoFrame::Unavailable };
        let slot = Arc::new(std::sync::Mutex::new(None));
        self.frames.insert(path.to_path_buf(), slot.clone());
        let video = path.to_path_buf();
        let spawned = std::thread::Builder::new().name("video-thumbnail".into()).spawn(move || {
            let png = video::frame(&source, &video);
            if let Ok(mut g) = slot.lock() {
                *g = Some(png.map_or(VideoFrame::Unavailable, VideoFrame::Ready));
            }
        });
        if spawned.is_err() {
            return VideoFrame::Unavailable;
        }
        VideoFrame::Pending
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
    fn pick_rules_file(&mut self) -> Option<PathBuf> {
        let file = self.dialog("Import rules").add_filter("MailCraft rules", &["json"]).add_filter("All files", &["*"]).pick_file();
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
        read_clipboard(|c| c.get().file_list().ok()).unwrap_or_default()
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
        std::thread::spawn(move || {
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

/// The file a `file:` address names (`file:///C:/Users/…/clip_image001.png`, with `%20` and
/// the like decoded); `None` for any other address.
fn office_file(src: &str) -> Option<PathBuf> {
    let src = src.trim();
    let rest = src.get(..5).filter(|s| s.eq_ignore_ascii_case("file:")).and_then(|_| src.get(5..))?;
    let rest = rest.trim_start_matches('/');
    let mut bytes = Vec::with_capacity(rest.len());
    let raw = rest.as_bytes();
    let mut i = 0;
    while let Some(&b) = raw.get(i) {
        let hex = |k: usize| raw.get(k).and_then(|h| (*h as char).to_digit(16));
        match (b, hex(i + 1), hex(i + 2)) {
            (b'%', Some(a), Some(b2)) => {
                bytes.push(u8::try_from(a * 16 + b2).ok()?);
                i += 3;
            }
            _ => {
                bytes.push(b);
                i += 1;
            }
        }
    }
    let path = String::from_utf8(bytes).ok()?;
    // `C:/…` on Windows; `/tmp/…` elsewhere.
    Some(if cfg!(windows) { PathBuf::from(path) } else { PathBuf::from(format!("/{path}")) })
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
