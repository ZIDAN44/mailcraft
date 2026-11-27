//! The interface's font: the system's own UI typeface, read from where the system keeps it (never
//! bundled), with a Semibold for the interface's emphasis as Windows' and Apple's guidelines have
//! it (a premium look: one UI face, Regular and Semibold, no Bold in the interface).
//!
//! - Windows 11: Segoe UI Variable, one variable font drawn at each weight; Windows 10: Segoe UI's
//!   Regular, Semibold and Bold files.
//! - macOS: the system font (SF), a variable font; Arial where it can't be read.
//! - Linux and the BSDs: Noto Sans with its SemiBold; DejaVu Sans, which has none, after it.
//!
//! PDFs keep embedding the static faces they always have (`system_fonts`).

use std::path::{Path, PathBuf};

use mailcraft_ui_egui::theme::UiFont;

/// The weights a variable UI font is drawn at: Regular, Semibold and Bold (its `wght` axis).
const WEIGHTS: [f32; 3] = [400.0, 600.0, 700.0];

/// The size, in points, the Mac's system font's letters are drawn for (its `opsz` axis): the
/// interface's text.
const MAC_OPTICAL_SIZE: f32 = 13.0;

/// The systems whose fonts are looked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum System {
    Windows,
    Mac,
    Other,
}

impl System {
    pub(crate) fn this() -> System {
        if cfg!(target_os = "windows") {
            System::Windows
        } else if cfg!(target_os = "macos") {
            System::Mac
        } else {
            System::Other
        }
    }
}

/// The interface's font on this computer.
pub(crate) fn ui_font() -> UiFont {
    let windows = super::windows_fonts_dir();
    choose(System::this(), &windows, &|path| super::fonts::first_readable([(path.to_path_buf(), 0)]).map(|(bytes, _)| bytes))
}

/// The font for `system`, its files read with `read` (`None`: missing or not a font egui reads);
/// `windows` is Windows' font folder.
pub(crate) fn choose(system: System, windows: &Path, read: &dyn Fn(&Path) -> Option<Vec<u8>>) -> UiFont {
    // With a static Bold face beside it, for pictures of PDF pages (they don't vary a weight).
    let variable = |path: PathBuf, opsz: Option<f32>, bold: &Path| {
        read(&path).map(|bytes| UiFont { regular: Some(bytes), bold: read(bold), weights: Some(WEIGHTS), opsz, ..UiFont::default() })
    };
    let faces = |files: [&Path; 3]| {
        let [regular, semibold, bold] = files.map(read);
        regular.map(|r| UiFont { regular: Some(r), semibold, bold, ..UiFont::default() })
    };
    let found = match system {
        System::Windows => variable(windows.join("SegUIVar.ttf"), None, &windows.join("segoeuib.ttf"))
            .or_else(|| faces([&windows.join("segoeui.ttf"), &windows.join("seguisb.ttf"), &windows.join("segoeuib.ttf")])),
        // At the interface's size (its optical-size axis): left at its default, a larger size's,
        // SF's small letters were drawn squeezed together (seen on macOS 26).
        System::Mac => variable(
            PathBuf::from("/System/Library/Fonts/SFNS.ttf"),
            Some(MAC_OPTICAL_SIZE),
            Path::new("/System/Library/Fonts/Supplemental/Arial Bold.ttf"),
        )
        .or_else(|| {
            faces([
                Path::new("/System/Library/Fonts/Supplemental/Arial.ttf"),
                Path::new("/System/Library/Fonts/Supplemental/Arial Bold.ttf"),
                Path::new("/System/Library/Fonts/Supplemental/Arial Bold.ttf"),
            ])
        }),
        System::Other => ["/usr/share/fonts/truetype/noto", "/usr/share/fonts/noto", "/usr/share/fonts/google-noto"]
            .iter()
            .map(Path::new)
            .find_map(|dir| faces([&dir.join("NotoSans-Regular.ttf"), &dir.join("NotoSans-SemiBold.ttf"), &dir.join("NotoSans-Bold.ttf")]))
            .or_else(|| {
                let dir = Path::new("/usr/share/fonts/truetype/dejavu");
                faces([&dir.join("DejaVuSans.ttf"), &dir.join("DejaVuSans-Bold.ttf"), &dir.join("DejaVuSans-Bold.ttf")])
            }),
    };
    found.unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The files a computer has, each read as its own name's bytes.
    fn computer(files: &'static [&'static str]) -> impl Fn(&Path) -> Option<Vec<u8>> {
        move |p: &Path| {
            let name = p.to_string_lossy().replace('\\', "/");
            files.iter().find(|f| name.ends_with(*f)).map(|f| f.as_bytes().to_vec())
        }
    }

    fn named(f: &Option<Vec<u8>>) -> Option<&str> {
        f.as_deref().and_then(|b| std::str::from_utf8(b).ok())
    }

    /// Windows 11's Segoe UI Variable at three weights; Windows 10's Segoe UI files with their
    /// Semibold; nothing found, egui's own (all `None`).
    #[test]
    fn windows_draws_in_segoe_ui_with_a_semibold() {
        let dir = Path::new("C:/Windows/Fonts");
        let eleven = choose(System::Windows, dir, &computer(&["SegUIVar.ttf", "segoeui.ttf", "seguisb.ttf", "segoeuib.ttf"]));
        assert_eq!((named(&eleven.regular), eleven.weights, named(&eleven.bold)), (Some("SegUIVar.ttf"), Some(WEIGHTS), Some("segoeuib.ttf")));
        let ten = choose(System::Windows, dir, &computer(&["segoeui.ttf", "seguisb.ttf", "segoeuib.ttf"]));
        assert_eq!(
            (named(&ten.regular), named(&ten.semibold), named(&ten.bold), ten.weights),
            (Some("segoeui.ttf"), Some("seguisb.ttf"), Some("segoeuib.ttf"), None)
        );
        assert_eq!(choose(System::Windows, dir, &computer(&[])), UiFont::default());
    }

    /// A Mac draws in its system font, a variable one, at the interface's optical size; Arial
    /// where that can't be read.
    #[test]
    fn a_mac_draws_in_its_system_font() {
        let mac = choose(System::Mac, Path::new(""), &computer(&["SFNS.ttf", "Arial.ttf", "Arial Bold.ttf"]));
        assert_eq!((named(&mac.regular), mac.weights, mac.opsz), (Some("SFNS.ttf"), Some(WEIGHTS), Some(MAC_OPTICAL_SIZE)));
        let old = choose(System::Mac, Path::new(""), &computer(&["Arial.ttf", "Arial Bold.ttf"]));
        assert_eq!((named(&old.regular), named(&old.bold)), (Some("Arial.ttf"), Some("Arial Bold.ttf")));
    }

    /// Linux: Noto Sans with its SemiBold, wherever the distribution keeps it; DejaVu Sans, which
    /// has none (its Bold is the emphasis), after it.
    #[test]
    fn linux_draws_in_noto_sans_with_its_semibold() {
        let fedora = choose(
            System::Other,
            Path::new(""),
            &computer(&["google-noto/NotoSans-Regular.ttf", "google-noto/NotoSans-SemiBold.ttf", "google-noto/NotoSans-Bold.ttf"]),
        );
        assert_eq!(named(&fedora.semibold), Some("google-noto/NotoSans-SemiBold.ttf"));
        let dejavu = choose(System::Other, Path::new(""), &computer(&["dejavu/DejaVuSans.ttf", "dejavu/DejaVuSans-Bold.ttf"]));
        assert_eq!((named(&dejavu.regular), named(&dejavu.semibold)), (Some("dejavu/DejaVuSans.ttf"), Some("dejavu/DejaVuSans-Bold.ttf")));
    }
}
