//! The clipboard and MailCraft's own keys on the window's input.

use std::path::PathBuf;
use std::sync::Arc;

/// Runs MailCraft's own keys on each window's input: Cut, Copy and Paste
/// ([`mailcraft_ui_egui::clipboard_keys`]); on Windows, Alt+numpad character codes
/// ([`mailcraft_ui_egui::alt_codes`], in the code pages for the user's language); on a Mac,
/// Control-click as the right-click ([`mailcraft_ui_egui::control_click`]).
pub(crate) struct ClipboardKeys {
    #[cfg_attr(not(windows), allow(dead_code))]
    pub(crate) pages: (u16, u16),
}

impl eframe::ViewportHooks for ClipboardKeys {
    /// No one-pixel offset of the window's contents: see `desktop/window_frame.rs`.
    #[cfg(windows)]
    fn window_created(&self, _viewport: egui::ViewportId, window: &winit::window::Window) {
        use winit::platform::windows::WindowExtWindows as _;
        window.set_undecorated_shadow(false);
    }

    fn raw_input(&self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        #[cfg(windows)]
        mailcraft_ui_egui::alt_codes(ctx, raw_input, self.pages);
        mailcraft_ui_egui::clipboard_keys(ctx, raw_input, &mut SystemClipboard);
        mailcraft_ui_egui::control_click(ctx, raw_input, cfg!(target_os = "macos"));
    }
}

/// The system clipboard, read as egui-winit reads it for a paste: its text, else its picture.
struct SystemClipboard;

impl mailcraft_ui_egui::ClipboardRead for SystemClipboard {
    fn files(&mut self) -> Vec<PathBuf> {
        clipboard_files()
    }

    fn paste(&mut self) -> Option<egui::Event> {
        if let Some(text) = read_clipboard(|c| c.get_text().ok()) {
            let text = text.replace("\r\n", "\n");
            return (!text.is_empty()).then_some(egui::Event::Paste(text));
        }
        let img = read_clipboard(|c| c.get_image().ok())?;
        // Its size and its bytes must agree (egui stops on a mismatch).
        if img.width.checked_mul(img.height).and_then(|n| n.checked_mul(4)) != Some(img.bytes.len()) {
            return None;
        }
        let image = egui::ColorImage::from_rgba_unmultiplied([img.width, img.height], &img.bytes);
        Some(egui::Event::PasteImage(Arc::new(image)))
    }
}

/// How long a clipboard read may wait on the app that copied (Linux and the BSDs).
#[cfg(all(unix, not(target_os = "macos")))]
const CLIPBOARD_WAIT: std::time::Duration = std::time::Duration::from_secs(2);

/// Read the clipboard. On Linux and the BSDs the read runs on a thread of its own and gives up
/// after [`CLIPBOARD_WAIT`]: a Wayland or X11 read waits on the app that copied, with no time
/// limit of its own, so an app that hangs would hang MailCraft's window too (FigCraft found).
#[cfg(all(unix, not(target_os = "macos")))]
pub(crate) fn read_clipboard<T: Send + 'static>(read: fn(&mut arboard::Clipboard) -> Option<T>) -> Option<T> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("clipboard".into())
        .spawn(move || {
            let _ = tx.send(arboard::Clipboard::new().ok().and_then(|mut c| read(&mut c)));
        })
        .ok()?;
    rx.recv_timeout(CLIPBOARD_WAIT).ok().flatten()
}

/// HTML on the clipboard: what was copied with its look (`None`: none, or nothing in it).
pub(crate) fn clipboard_html() -> Option<String> {
    #[cfg(windows)]
    let html = windows_html();
    #[cfg(not(windows))]
    let html = read_clipboard(|c| c.get().html().ok());
    html.filter(|h| !h.trim().is_empty())
}

/// Windows' "HTML Format" as the program that copied wrote it, read
/// ([`mailcraft_engine::mime::windows_clipboard_html`]) for what was copied: arboard reads its
/// fragment alone, and cells copied in Excel lost the table they're in (issue #268).
#[cfg(windows)]
fn windows_html() -> Option<String> {
    let format = clipboard_win::register_format("HTML Format")?;
    let raw: Vec<u8> = clipboard_win::get_clipboard(clipboard_win::formats::RawData(format.get())).ok()?;
    mailcraft_engine::mime::windows_clipboard_html(&raw)
}

/// Files copied in a file manager. PCManFM ends its list with a NUL byte, which stayed on the
/// last file's name: of two files copied, one was pasted (issue #10).
pub(crate) fn clipboard_files() -> Vec<PathBuf> {
    let files = read_clipboard(|c| c.get().file_list().ok()).unwrap_or_default();
    let nul = char::from(0);
    files.into_iter().map(|p| p.to_str().filter(|s| s.ends_with(nul)).map_or(p.clone(), |s| PathBuf::from(s.trim_end_matches(nul)))).collect()
}

/// Read the clipboard (Windows' and macOS's reads don't wait on the app that copied).
#[cfg(not(all(unix, not(target_os = "macos"))))]
pub(crate) fn read_clipboard<T>(read: fn(&mut arboard::Clipboard) -> Option<T>) -> Option<T> {
    read(&mut arboard::Clipboard::new().ok()?)
}
