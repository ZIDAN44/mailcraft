//! What MailCraft's window is opened with: the graphics adapter it's drawn with, the windowing
//! system it opens on (Linux and the BSDs), its icon, the clipboard and its keys, and dragging
//! files out of it.

mod clipboard;
pub(super) mod display;
pub(super) mod graphics;
pub(super) mod work_areas;
#[cfg(all(unix, not(target_os = "macos")))]
mod xdnd;

pub(super) use clipboard::{ClipboardKeys, clipboard_files, clipboard_html, read_clipboard};

use std::path::PathBuf;

/// The committed app icon (`cargo xtask icons`), at 64 pixels.
const APP_ICON_PNG: &[u8] = include_bytes!("../../../assets/app-icon/hicolor/64x64/apps/io.github.zidan44.MailCraft.png");

/// The window icon: the app icon, or egui's default if it can't be decoded.
pub(super) fn app_icon() -> egui::IconData {
    match image::load_from_memory_with_format(APP_ICON_PNG, image::ImageFormat::Png) {
        Ok(img) => {
            let img = img.into_rgba8();
            egui::IconData { width: img.width(), height: img.height(), rgba: img.into_raw() }
        }
        Err(_) => egui::IconData::default(),
    }
}

/// Hand files to the system's drag and drop (Windows and macOS; elsewhere egui has no way to).
#[cfg(any(target_os = "windows", target_os = "macos"))]
pub(super) fn drag_out(frame: &eframe::Frame, paths: Vec<PathBuf>) {
    let options = drag::Options { skip_animatation_on_cancel_or_failure: false, mode: drag::DragMode::Copy };
    // The drag crate is native code; a failure there must not take the app down.
    let started = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        drag::start_drag(frame, drag::DragItem::Files(paths), drag::Image::Raw(APP_ICON_PNG.to_vec()), |_, _| {}, options)
    }));
    match started {
        Ok(Ok(())) => {}
        Ok(Err(e)) => log::warn!("drag out: {e}"),
        Err(_) => log::warn!("drag out failed"),
    }
}

/// Hand files to X11's drag and drop (XDND), on a thread of its own while the button is held.
#[cfg(all(unix, not(target_os = "macos")))]
pub(super) fn drag_out(frame: &eframe::Frame, paths: Vec<PathBuf>) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let own = match frame.window_handle().map(|h| h.as_raw()) {
        Ok(RawWindowHandle::Xlib(h)) => u32::try_from(h.window).ok(),
        Ok(RawWindowHandle::Xcb(h)) => Some(h.window.get()),
        _ => None,
    };
    let Some(own) = own else {
        log::info!("drag out: not an X11 window");
        return;
    };
    let spawned = std::thread::Builder::new().name("drag-out".into()).spawn(move || {
        if let Err(e) = xdnd::drag(None, own, &paths) {
            log::warn!("drag out: {e}");
        }
    });
    if let Err(e) = spawned {
        log::warn!("drag out: {e}");
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos", unix)))]
pub(super) fn drag_out(_frame: &eframe::Frame, _paths: Vec<PathBuf>) {}
