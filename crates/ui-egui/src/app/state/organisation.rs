//! The window's part of an organisation's profile (issue #245): the theme and the language, each
//! put in once as a default (and again only when the organisation gives another) or held as it
//! locked them; and the organisation's name and logo for File › About.

use mailcraft_engine::policy::Organisation;

use super::*;

/// The largest logo shown, across or down; a bigger one is drawn smaller.
pub(crate) const LOGO_SIDE: u32 = 512;

impl MailApp {
    /// The organisation's profile, once the window's own settings are back: given to the session
    /// (its defaults and locks over them), and its theme, language and logo taken here. `logo` is
    /// the picture file's bytes, when the profile names one that could be read.
    pub fn set_organisation(&mut self, o: Organisation, logo: Option<Vec<u8>>) {
        self.session.set_organisation(o);
        self.follow_organisation();
        self.ui.org_logo = logo.and_then(|b| decode_logo(&b));
    }

    /// The theme and the language as the organisation gives them: a default put in once, a lock
    /// held. Every frame: an account added may bring its domain's profile, and its locks. Only
    /// what differs is written.
    pub fn follow_organisation(&mut self) {
        let org = &self.session.org;
        let applied = &self.session.prefs.org_applied;
        let theme = org.theme.as_ref().filter(|g| g.locked || applied.get("theme") != Some(&g.value)).map(|g| g.value.clone());
        let language = org
            .language
            .as_ref()
            .filter(|g| g.locked || applied.get("language") != Some(&g.value))
            .and_then(|g| g.value.as_str().map(|c| (c.to_string(), g.value.clone())))
            .filter(|(code, _)| code == i18n::AUTO || i18n::Lang::from_code(code).is_some());
        if let Some(value) = theme {
            let dark = match value.as_str() {
                Some("light") => Some(false),
                Some("dark") => Some(true),
                _ => None,
            };
            if self.ui.dark != dark {
                self.ui.dark = dark;
            }
            if self.session.prefs.org_applied.get("theme") != Some(&value) {
                self.session.prefs.org_applied.insert("theme".into(), value);
            }
        }
        if let Some((code, value)) = language {
            if self.ui.language != code {
                self.ui.language = code;
            }
            if self.session.prefs.org_applied.get("language") != Some(&value) {
                self.session.prefs.org_applied.insert("language".into(), value);
            }
        }
    }
}

/// A logo's picture, at most [`LOGO_SIDE`] across and down; `None` for one that can't be read.
fn decode_logo(bytes: &[u8]) -> Option<egui::ColorImage> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(64 << 20);
    reader.limits(limits);
    let img = reader.decode().ok()?;
    let img = if img.width() > LOGO_SIDE || img.height() > LOGO_SIDE { img.thumbnail(LOGO_SIDE, LOGO_SIDE) } else { img };
    let rgba = img.to_rgba8();
    let size = [usize::try_from(rgba.width()).ok()?, usize::try_from(rgba.height()).ok()?];
    Some(egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_logo_is_read_and_kept_small_and_junk_isnt() {
        let mut png = Vec::new();
        image::RgbaImage::from_pixel(1600, 400, image::Rgba([10, 80, 160, 255]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let logo = decode_logo(&png).unwrap();
        assert_eq!(logo.size, [512, 128]);
        assert!(decode_logo(b"not a picture").is_none());
        assert!(decode_logo(&png[..png.len() / 2]).is_none());
    }
}
