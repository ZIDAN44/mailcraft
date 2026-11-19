//! The computer's fonts, to write in (issue #166): the font box offers the families installed,
//! and text in one is drawn in it in the message being written. The desktop app finds them (a
//! [`FontSource`]: the Craft apps' font catalog); the browser has none, and offers no box.
//!
//! egui's own families are the interface's. A family chosen is a family of its own, named after
//! it ([`PREFIX`], [`BOLD_PREFIX`] for its bold), filled by a [`FontProvider`] the first time it's
//! drawn: its face, then the interface's fonts for letters it hasn't (Chinese, Arabic...). A
//! family egui is asked for that no provider fills draws nothing at all, so one is asked for only
//! where the provider was installed: this thread (the window's), with a source that has it.

use std::cell::RefCell;
use std::collections::HashSet;
use std::sync::{Arc, Mutex, PoisonError};

use egui::epaint::text::{FontInsert, FontProvider};
use egui::{FontData, FontDefinitions, FontFamily};

/// The computer's fonts, as the desktop app finds them.
pub trait FontSource: Send + Sync {
    /// The families installed, sorted (none until they've been looked for).
    fn families(&self) -> Arc<[String]>;
    /// A family's face, its bold one for `bold` (else the nearest it has): the file's bytes and the
    /// face's index in it, checked to be a font egui reads (one it couldn't read would stop
    /// MailCraft). `None`: not installed, or it can't be read.
    fn face(&self, family: &str, bold: bool) -> Option<(Arc<[u8]>, u32)>;
}

/// The name of a chosen family's egui family, before the family's own name.
const PREFIX: &str = "mail-font:";
/// The same, for its bold.
const BOLD_PREFIX: &str = "mail-font-bold:";

/// The source on this thread, and its families (lower case, to find one fast) as last read.
struct Here {
    source: Arc<dyn FontSource>,
    families: Arc<[String]>,
    known: HashSet<String>,
}

thread_local! {
    static HERE: RefCell<Option<Here>> = const { RefCell::new(None) };
}

/// The interface's fonts, after a chosen family's face: the proportional family's and the bold
/// one's, as [`crate::theme`] installed them last.
static AFTER: Mutex<(Vec<FontInsert>, Vec<FontInsert>)> = Mutex::new((Vec::new(), Vec::new()));

/// Offer the computer's fonts in the window `ctx` draws, on this thread (the window's).
pub fn install(ctx: &egui::Context, source: Arc<dyn FontSource>) {
    ctx.add_font_provider(Arc::new(Provider { source: Arc::clone(&source) }));
    HERE.with(|h| *h.borrow_mut() = Some(Here { source, families: Arc::from([]), known: HashSet::new() }));
}

/// The interface's fonts as `fonts` installs them, for after a chosen family's face.
pub(crate) fn follow(fonts: &FontDefinitions) {
    let chain = |family: &FontFamily| -> Vec<FontInsert> {
        let names = fonts.families.get(family).cloned().unwrap_or_default();
        names
            .into_iter()
            .filter_map(|name| {
                let data = fonts.font_data.get(&name)?;
                Some(FontInsert { name, data: FontData::clone(data), families: Vec::new() })
            })
            .collect()
    };
    let both = (chain(&FontFamily::Proportional), chain(&FontFamily::Name("bold".into())));
    *AFTER.lock().unwrap_or_else(PoisonError::into_inner) = both;
}

/// The families installed, sorted (none in the browser, or before they've been looked for).
pub(crate) fn families() -> Arc<[String]> {
    HERE.with(|h| {
        let mut here = h.borrow_mut();
        let Some(here) = here.as_mut() else { return Arc::from([]) };
        let now = here.source.families();
        if !Arc::ptr_eq(&now, &here.families) {
            here.known = now.iter().map(|f| f.to_lowercase()).collect();
            here.families = Arc::clone(&now);
        }
        now
    })
}

/// The egui family to draw text in `name` with (its bold for `bold`): `None` where the computer's
/// fonts aren't offered or it hasn't that one (the interface's font draws it then).
pub(crate) fn family(name: &str, bold: bool) -> Option<FontFamily> {
    families();
    let known = HERE.with(|h| h.borrow().as_ref().is_some_and(|here| here.known.contains(&name.to_lowercase())));
    known.then(|| FontFamily::Name(format!("{}{name}", if bold { BOLD_PREFIX } else { PREFIX }).into()))
}

/// Fills a chosen family's egui family.
struct Provider {
    source: Arc<dyn FontSource>,
}

impl FontProvider for Provider {
    fn fonts_for_family(&self, family: &FontFamily) -> Vec<FontInsert> {
        let FontFamily::Name(egui_name) = family else { return Vec::new() };
        let (bold, name) = match (egui_name.strip_prefix(BOLD_PREFIX), egui_name.strip_prefix(PREFIX)) {
            (Some(name), _) => (true, name),
            (None, Some(name)) => (false, name),
            (None, None) => return Vec::new(),
        };
        let mut fonts = Vec::new();
        if let Some((bytes, index)) = self.source.face(name, bold) {
            let mut data = FontData::from_owned(bytes.to_vec());
            data.index = index;
            fonts.push(FontInsert { name: egui_name.to_string(), data, families: Vec::new() });
        }
        let after = AFTER.lock().unwrap_or_else(PoisonError::into_inner);
        fonts.extend(if bold { after.1.iter() } else { after.0.iter() }.cloned());
        fonts
    }
}
