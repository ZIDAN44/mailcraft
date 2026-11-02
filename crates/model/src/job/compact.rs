//! A data file made smaller: how it's doing ([`DataFileHealth`], measured by the store without
//! reading any mail), how it's compacted ([`plan`]), and what that came to ([`CompactReport`]).
//!
//! There are two ways. In place, what a build before 0.32 kept raw is compressed again in the
//! background (stopped and gone on with at any batch), then the database's own compaction gives
//! the room back to the drive; that last part has the file to itself, so nothing else of the
//! account can be read or written meanwhile, and it can't be stopped. Measured cold on a 44 GB
//! data file it read the whole file about twice and took 125 s: about 1.8 s for every GiB of the
//! file plus 5.3 s for every GiB moved. So it's done in place only when that's short enough for
//! how it was asked ([`CompactMode::limit_s`]). A data file that is both bloated (kept raw by an
//! earlier build) and too big for that is copied into a new file instead, in the background,
//! when the drive has room for the copy; when it hasn't, importing it again is suggested.

use serde::{Deserialize, Serialize};

/// A GiB.
const GIB: u64 = 1 << 30;
/// What compacting in place costs, measured cold (`store/src/packed/tests/compaction.rs`): this
/// many ms for each GiB of the file (it's read whole however little moves), and for each GiB moved.
const MS_PER_GIB_LOOKED: u64 = 1_800;
const MS_PER_GIB_MOVED: u64 = 5_300;
/// A pace as kept: millionths of the measured cost. Unknown, it's the measured cost itself.
pub const PACE_ONE: u64 = 1_000_000;

/// How a data file is doing ([`DataFileHealth`] is measured by the store's
/// `data_file_health`), measured without reading any mail.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DataFileHealth {
    /// The file's size on disk.
    pub file_bytes: u64,
    /// What what it keeps takes in it, pages and all.
    pub live_bytes: u64,
    /// What compacting it would give back: its size over what it keeps, less what the file has
    /// over that just after a compaction (2% until one has measured it).
    pub reclaimable: u64,
    /// How many files (messages, lists, documents) it keeps, and their bytes as kept.
    pub files: u64,
    pub kept_bytes: u64,
    /// Of what was sampled, the share kept raw that would compress (a build before 0.32 kept
    /// base64 attachments raw), 0 to 1.
    pub raw_fraction: f32,
    /// About what compressing those again would save.
    pub recompress_gain: u64,
    /// Everything in it is kept compressed as this build keeps it (a file made by this build, or
    /// compressed again); nothing was sampled then.
    pub recompressed: bool,
    /// A recompress was stopped part way: there's more to do.
    pub recompress_stopped: bool,
    /// How this file's compactions in place went against the measured cost, in millionths
    /// ([`PACE_ONE`]: as measured; 0, none has run yet).
    #[serde(default)]
    pub pace: u64,
}

/// Events compare whole: its one fraction is a share of what was sampled, never NaN.
impl Eq for DataFileHealth {}

impl DataFileHealth {
    /// Compacting is offered (File › Info) from this much to give back.
    pub const OFFER_FROM: u64 = 64 << 20;
    /// Compacted without being asked from this much to give back, and at least
    /// [`DataFileHealth::AUTO_SHARE`] of the file.
    pub const AUTO_FROM: u64 = 1 << 30;
    pub const AUTO_SHARE: f64 = 0.20;
    /// Compressed again without being asked from this share of it kept raw.
    pub const AUTO_RAW: f32 = 0.10;

    /// Worth compacting when MailCraft is idle: a gigabyte and a fifth of the file to give back,
    /// or a gigabyte to save by compressing what's kept raw.
    pub fn needs_compacting(&self) -> bool {
        let share = (self.file_bytes as f64 * Self::AUTO_SHARE) as u64;
        self.reclaimable >= Self::AUTO_FROM.max(share)
            || (!self.recompressed && self.raw_fraction >= Self::AUTO_RAW && self.recompress_gain >= Self::AUTO_FROM)
    }

    /// Worth offering to compact.
    pub fn can_compact(&self) -> bool {
        self.reclaimable >= Self::OFFER_FROM || self.recompress_stopped || (!self.recompressed && self.recompress_gain >= Self::OFFER_FROM)
    }

    /// What it keeps raw is worth compressing again first (a recompress stopped part way goes on).
    pub fn wants_recompress(&self) -> bool {
        !self.recompressed && (self.recompress_stopped || self.recompress_gain >= Self::OFFER_FROM)
    }

    /// About how long compacting it in place would have it to itself, in ms: after compressing
    /// again what it keeps raw when `recompressed` (the room that frees moves too).
    pub fn exclusive_ms(&self, recompressed: bool) -> u64 {
        let moved = self.reclaimable.saturating_add(if recompressed { self.recompress_gain } else { 0 });
        compact_ms(self.file_bytes, moved, self.pace)
    }

    /// About what compacting it gives back to the drive: the room it has over what it keeps,
    /// and what compressing again would save when an earlier build kept some of it raw.
    pub fn saving(&self) -> u64 {
        let gain = if self.recompressed { 0 } else { self.recompress_gain };
        self.reclaimable.saturating_add(gain)
    }

    /// About how long compacting it by hand (Compact now) has the file to itself, in seconds, as
    /// [`plan`] would do it with room on the drive: `None` when it's copied into a new file
    /// instead (in the background; how long that takes depends on the drive) or there's
    /// nothing to do.
    pub fn by_hand_s(&self) -> Option<u64> {
        match plan(self, CompactMode::Manual, Some(u64::MAX)) {
            CompactPlan::InPlace { recompress } => Some(self.exclusive_ms(recompress).div_ceil(1000)),
            _ => None,
        }
    }

    /// The room a copy of it needs on its drive: what it keeps (less what compressing again
    /// saves), a tenth more for the copy's own pages and its growing, and 2 GiB left over.
    pub fn copy_need(&self) -> u64 {
        let gain = if self.recompressed { 0 } else { self.recompress_gain };
        let live = self.live_bytes.saturating_sub(gain);
        live.saturating_add(live / 10).saturating_add(2 * GIB)
    }
}

/// About how long compacting a data file of `file` bytes in place takes when `moved` bytes move,
/// in ms, at `pace` (millionths of the measured cost; 0: as measured).
pub fn compact_ms(file: u64, moved: u64, pace: u64) -> u64 {
    let per = |bytes: u64, ms: u64| u64::try_from(u128::from(bytes) * u128::from(ms) / u128::from(GIB)).unwrap_or(u64::MAX);
    let base = per(file, MS_PER_GIB_LOOKED).saturating_add(per(moved, MS_PER_GIB_MOVED));
    let pace = if pace == 0 { PACE_ONE } else { pace };
    u64::try_from(u128::from(base) * u128::from(pace) / u128::from(PACE_ONE)).unwrap_or(u64::MAX)
}

/// The pace learned from one more compaction in place: how long it took (`took_ms`) against
/// what the measured cost says (`base_ms`, at [`PACE_ONE`]), weighed in at three tenths, and
/// kept within a twentieth and twenty times the measured cost.
pub fn next_pace(pace: u64, took_ms: u64, base_ms: u64) -> u64 {
    let old = if pace == 0 { PACE_ONE } else { pace };
    if base_ms == 0 {
        return old;
    }
    let now = u64::try_from(u128::from(took_ms) * u128::from(PACE_ONE) / u128::from(base_ms)).unwrap_or(u64::MAX);
    let blended = u128::from(old) * 7 / 10 + u128::from(now) * 3 / 10;
    u64::try_from(blended).unwrap_or(u64::MAX).clamp(PACE_ONE / 20, PACE_ONE * 20)
}

/// Why a data file is compacted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CompactMode {
    /// MailCraft has been left alone: anything the user does stops it.
    #[default]
    Idle,
    /// MailCraft is closing.
    Close,
    /// Asked for (File › Info › Compact now).
    Manual,
}

impl CompactMode {
    /// The longest it may have the data file to itself, in seconds (`None`: as long as it
    /// takes): at idle, short enough that coming back to it hardly waits; closing, what's
    /// worth waiting for.
    pub fn limit_s(self) -> Option<u64> {
        match self {
            CompactMode::Idle => Some(15),
            CompactMode::Close => Some(30),
            CompactMode::Manual => None,
        }
    }
}

/// Where a compaction is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CompactPhase {
    /// Measuring the data file.
    #[default]
    Measuring,
    /// Compressing again what an earlier build kept raw (in the background; stops at once).
    Recompress,
    /// Copying it into a new file (in the background; stops at once).
    Copy,
    /// The data file to itself: nothing else of the account can be read or written, and it can't
    /// be stopped.
    Exclusive,
}

/// What compacting a data file comes to, from its health, how it was asked, and the room on its
/// drive (`None`: it can't be told).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "plan", rename_all = "camelCase")]
pub enum CompactPlan {
    /// Not worth it.
    Nothing,
    /// In place: compressed again first when `recompress`, then compacted.
    InPlace { recompress: bool },
    /// Copied into a new file.
    Copy,
    /// It would have the file to itself for about `seconds`, longer than asked for.
    TooLong { seconds: u64 },
    /// Kept raw and too big to compact in place, with no room on its drive for a copy: importing
    /// it again makes it as small.
    Reimport,
}

/// How compacting a data file goes, by [`CompactPlan`]: in place when that has the file to itself
/// no longer than `mode` allows; a file kept raw by an earlier build that's too big for that,
/// copied when its drive has room for it (on close, never: it takes minutes), else imported
/// again; any other, left until asked for (when it may take as long as it takes).
pub fn plan(health: &DataFileHealth, mode: CompactMode, free: Option<u64>) -> CompactPlan {
    let wanted = match mode {
        CompactMode::Manual => health.can_compact(),
        CompactMode::Idle | CompactMode::Close => health.needs_compacting() || health.recompress_stopped,
    };
    if !wanted {
        return CompactPlan::Nothing;
    }
    let recompress = health.wants_recompress();
    let ms = health.exclusive_ms(recompress);
    let seconds = ms.div_ceil(1000);
    // Asked for, a file kept raw is copied rather than held as long as closing would wait.
    let limit = match mode {
        CompactMode::Manual if recompress => CompactMode::Close.limit_s(),
        mode => mode.limit_s(),
    };
    if limit.is_none_or(|limit| seconds <= limit) {
        return CompactPlan::InPlace { recompress };
    }
    match (mode, recompress) {
        (CompactMode::Close, _) | (CompactMode::Idle, false) => CompactPlan::TooLong { seconds },
        (_, true) if free.is_some_and(|f| f >= health.copy_need()) => CompactPlan::Copy,
        (_, true) => CompactPlan::Reimport,
        (CompactMode::Manual, false) => CompactPlan::InPlace { recompress: false },
    }
}

/// What a compaction came to ([`crate::Event::Compacted`]).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CompactReport {
    pub mode: CompactMode,
    /// The file's size before and after (the same when it didn't get as far as giving room back).
    pub before: u64,
    pub after: u64,
    pub elapsed_ms: u64,
    /// What compressing again saved (bytes as kept), in this run.
    pub recompressed: u64,
    /// The database's own compaction ran (the file had to itself).
    pub compacted: bool,
    /// It was copied into a new file.
    pub copied: bool,
    /// Stopped before the end (what was compressed again is kept; a copy is thrown away).
    pub cancelled: bool,
    /// Not compacted in place: it would have had the file to itself for about this long.
    pub too_long_s: Option<u64>,
    /// Too big to compact in place, and no room for a copy: importing it again is suggested.
    pub reimport: bool,
    pub error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn health(file_gib: u64, reclaimable_gib: u64, gain_gib: u64) -> DataFileHealth {
        DataFileHealth {
            file_bytes: file_gib * GIB,
            live_bytes: (file_gib - reclaimable_gib) * GIB,
            reclaimable: reclaimable_gib * GIB,
            raw_fraction: if gain_gib > 0 { 0.7 } else { 0.0 },
            recompress_gain: gain_gib * GIB,
            recompressed: gain_gib == 0,
            ..DataFileHealth::default()
        }
    }

    #[test]
    fn the_measured_cost_is_what_a_cold_compaction_took() {
        // 44 GB, 8.5 GiB given back: 125 s measured cold.
        let ms = compact_ms(44_817 << 20, 8_490 << 20, 0);
        assert!((120_000..=130_000).contains(&ms), "{ms}");
        assert_eq!(compact_ms(GIB, 0, PACE_ONE * 2), 3_600, "a slower file, twice as long");
        assert_eq!(compact_ms(u64::MAX, u64::MAX, u64::MAX), u64::MAX, "saturated, not wrapped");
    }

    #[test]
    fn the_pace_is_learned_slowly_and_kept_sane() {
        assert_eq!(next_pace(0, 2_000, 1_000), 1_300_000, "twice as slow, weighed in at three tenths");
        assert_eq!(next_pace(PACE_ONE, 0, 1_000), 700_000);
        assert_eq!(next_pace(PACE_ONE, u64::MAX, 1), PACE_ONE * 20, "a wild one is held to twenty times");
        assert_eq!(next_pace(PACE_ONE / 20, 0, 1_000), PACE_ONE / 20);
        assert_eq!(next_pace(PACE_ONE, 5, 0), PACE_ONE, "nothing to weigh against");
    }

    #[test]
    fn plans_follow_the_limits_and_the_room() {
        // A small file with deleted mail: quick in place, whichever way.
        let small = health(3, 1, 0);
        for mode in [CompactMode::Idle, CompactMode::Close, CompactMode::Manual] {
            assert_eq!(plan(&small, mode, None), CompactPlan::InPlace { recompress: false }, "{mode:?}");
        }
        // A big one with deleted mail: only when asked; too long at idle and on close.
        let big = health(40, 10, 0);
        assert!(matches!(plan(&big, CompactMode::Idle, Some(u64::MAX)), CompactPlan::TooLong { seconds } if seconds > 100));
        assert!(matches!(plan(&big, CompactMode::Close, Some(u64::MAX)), CompactPlan::TooLong { .. }));
        assert_eq!(plan(&big, CompactMode::Manual, Some(0)), CompactPlan::InPlace { recompress: false });
        // A big one an earlier build kept raw: copied when there's room, else imported again.
        let bloated = health(47, 0, 9);
        let room = bloated.copy_need();
        assert!(room > 38 * GIB && room < 45 * GIB, "{room}");
        for mode in [CompactMode::Idle, CompactMode::Manual] {
            assert_eq!(plan(&bloated, mode, Some(room)), CompactPlan::Copy, "{mode:?}");
            assert_eq!(plan(&bloated, mode, Some(room - 1)), CompactPlan::Reimport, "{mode:?}");
            assert_eq!(plan(&bloated, mode, None), CompactPlan::Reimport, "room unknown: none counted on");
        }
        assert!(matches!(plan(&bloated, CompactMode::Close, Some(u64::MAX)), CompactPlan::TooLong { .. }), "never a copy on close");
        // A small one kept raw: compressed again, then compacted, in place.
        assert_eq!(plan(&health(4, 0, 1), CompactMode::Idle, None), CompactPlan::InPlace { recompress: true });
        // Nothing to give back: nothing done; a recompress stopped part way goes on.
        assert_eq!(plan(&health(40, 0, 0), CompactMode::Manual, None), CompactPlan::Nothing);
        let stopped = DataFileHealth { recompress_stopped: true, ..health(2, 0, 0) };
        let stopped = DataFileHealth { recompressed: false, ..stopped };
        assert_eq!(plan(&stopped, CompactMode::Idle, None), CompactPlan::InPlace { recompress: true });
    }

    #[test]
    fn what_compacting_by_hand_saves_and_takes_is_told_honestly() {
        // Deleted mail in a big file: in place, as long as it takes, which is more than closing
        // waits for.
        let big = health(40, 10, 0);
        assert_eq!(big.saving(), 10 * GIB);
        let by_hand = big.by_hand_s().unwrap();
        assert!(by_hand > CompactMode::Close.limit_s().unwrap(), "{by_hand}");
        assert_eq!(by_hand, big.exclusive_ms(false).div_ceil(1000));
        // Kept raw by an earlier build: what compressing again saves counts, and a big one is
        // copied (no estimate), a small one compressed again and compacted in place.
        let bloated = health(47, 1, 9);
        assert_eq!(bloated.saving(), 10 * GIB);
        assert_eq!(bloated.by_hand_s(), None, "copied");
        let small = health(4, 0, 1);
        assert_eq!(small.by_hand_s(), Some(small.exclusive_ms(true).div_ceil(1000)));
        assert_eq!(health(40, 0, 0).by_hand_s(), None, "nothing to give back");
        let odd = DataFileHealth { reclaimable: u64::MAX, recompress_gain: u64::MAX, ..DataFileHealth::default() };
        assert_eq!(odd.saving(), u64::MAX, "saturated, not wrapped");
    }

    #[test]
    fn reports_and_health_read_what_earlier_versions_wrote() {
        let h: DataFileHealth = serde_json::from_str(
            r#"{"file_bytes":1,"live_bytes":1,"reclaimable":0,"files":1,"kept_bytes":1,"raw_fraction":0.0,"recompress_gain":0,"recompressed":true,"recompress_stopped":false}"#,
        )
        .unwrap();
        assert_eq!(h.pace, 0, "none run yet");
        let r: CompactReport = serde_json::from_str(r#"{"before":5,"unknownField":1}"#).unwrap();
        assert_eq!((r.before, r.mode), (5, CompactMode::Idle));
    }
}
