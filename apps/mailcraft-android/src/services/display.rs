use super::*;

impl AndroidServices {
    pub(crate) fn cached_zone_name(&self) -> Option<String> {
        let mut zone = self.zone.lock().ok()?;
        if zone.checked_at.is_none_or(|checked_at| checked_at.elapsed() >= std::time::Duration::from_secs(300)) {
            zone.name = platform::timezone(&self.activity);
            zone.checked_at = Some(Instant::now());
        }
        zone.name.clone()
    }

    pub(crate) fn cached_dark_mode(&self) -> Option<bool> {
        let mut dark = self.dark.lock().ok()?;
        if dark.checked_at.is_none_or(|checked_at| checked_at.elapsed() >= std::time::Duration::from_secs(60)) {
            dark.value = platform::dark_mode(&self.activity);
            dark.checked_at = Some(Instant::now());
        }
        dark.value
    }
}
