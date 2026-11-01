//! A data file's health, measured cheaply and only read: how much of it is mail still kept, how
//! much a compaction would give back, and whether what's kept raw would compress now (a build
//! whose fast deflate kept base64 at 0.995 of its size stored most attachments raw). And its
//! `meta` table, where the file says how it was written.
//!
//! The database can count its free pages itself, but only by walking every page in a write
//! transaction (minutes on a cold 37 GB file, holding off every writer). The pages of `FILES`
//! (whole values, and the notes of files in pieces) are counted as the database counts them, from
//! a walk of that table alone; every file in pieces has its length in its note, and pieces fill
//! their page runs exactly. So one read of `FILES` comes within a fraction of a percent of it,
//! without reading a piece: 0.32 s cold (opening the file included) and 6 ms warm for the 34,741
//! files of a 37 GB data file, in 34 MB. What it can't see (the pieces' own tree) is allowed for
//! with what the file had over it when it was last compacted.

use redb::ReadableTableMetadata;

use super::*;

/// How the data file was written, a few values by name. Old files have none, and old builds
/// never open it; `FILES` alone is walked for paths, so none of this is ever taken for one.
pub(super) const META: TableDefinition<&str, &[u8]> = TableDefinition::new("meta");
/// The layout of `META` (a u64): written with every change to it.
const VERSION: &str = "version";
const VERSION_NOW: u64 = 1;
/// Which compression what's kept was written with (a u64): [`DEFLATE_NOW`] once every value is.
pub(super) const DEFLATE: &str = "deflate";
/// Every value written by a build that calls miniz itself (`pack`), or compressed again.
pub(super) const DEFLATE_NOW: u64 = 2;
/// What the file had over what the walk counts just after it was compacted, in millionths of
/// the walk's count (a u64): the tree's own pages, and the end of the file.
const OVERHEAD: &str = "compact.overhead";
/// Until a compaction measures it: 2%.
const OVERHEAD_DEFAULT: u64 = 20_000;
/// Where a recompress that was stopped got to: the last key it did (UTF-8).
pub(super) const CURSOR: &str = "recompress.cursor";
/// The longest key a cursor can be (a store path is at most 512 bytes).
const CURSOR_MAX: usize = 1024;
/// What recompressing has found over every run of it, each a u64 of bytes as they were kept: the
/// files tried that stayed as they were (they don't compress: pictures, archives), and what the
/// others came down by.
pub(super) const STAYED_RAW: &str = "recompress.raw";
pub(super) const SAVED: &str = "recompress.saved";

/// One file in this many is sampled for whether it's kept raw (the first of them too).
const SAMPLE_EVERY: u64 = 128;
/// How much of a sampled file is compressed to see whether it would be.
const TRIAL_MAX: usize = 64 << 10;
/// The search index keeps its own pages packed already: not sampled, nor compressed again.
pub(super) const SEARCH: &str = "search/";

/// What `META` says, each value read on its own: one that's missing or can't be read as what it
/// should be is as if it weren't there.
#[derive(Debug, Default, Clone, PartialEq)]
pub(super) struct Meta {
    pub(super) deflate: u64,
    pub(super) overhead: Option<u64>,
    pub(super) cursor: Option<String>,
    pub(super) stayed_raw: u64,
    pub(super) saved: u64,
}

/// A u64 as kept (8 bytes, little-endian).
fn u64_of(v: &[u8]) -> Option<u64> {
    Some(u64::from_le_bytes(v.get(..8)?.try_into().ok()?))
}

/// The data file's `META`. One that isn't there (an old file), or isn't a table of this kind (a
/// crafted file), says nothing.
pub(super) fn read_meta(tx: &redb::ReadTransaction) -> io::Result<Meta> {
    let table = match tx.open_table(META) {
        Ok(t) => t,
        Err(redb::TableError::Storage(e)) => return Err(disk(e)),
        Err(_) => return Ok(Meta::default()),
    };
    let value = |key: &str| -> io::Result<Option<Vec<u8>>> { Ok(table.get(key).map_err(disk)?.map(|g| g.value().to_vec())) };
    Ok(Meta {
        deflate: value(DEFLATE)?.as_deref().and_then(u64_of).unwrap_or(0),
        overhead: value(OVERHEAD)?.as_deref().and_then(u64_of).filter(|ppm| *ppm <= 1_000_000),
        cursor: value(CURSOR)?.filter(|c| !c.is_empty() && c.len() <= CURSOR_MAX).and_then(|c| String::from_utf8(c).ok()),
        stayed_raw: value(STAYED_RAW)?.as_deref().and_then(u64_of).unwrap_or(0),
        saved: value(SAVED)?.as_deref().and_then(u64_of).unwrap_or(0),
    })
}

/// Values written to `META` in the transaction `tx` (a u64 as 8 bytes), with its version.
pub(super) fn write_meta(tx: &redb::WriteTransaction, values: &[(&str, &[u8])]) -> io::Result<()> {
    let mut table = tx.open_table(META).map_err(disk)?;
    table.insert(VERSION, VERSION_NOW.to_le_bytes().as_slice()).map_err(disk)?;
    for (k, v) in values {
        table.insert(*k, *v).map_err(disk)?;
    }
    Ok(())
}

/// The bytes a file of `kept` bytes in pieces takes: its pieces' page runs, as
/// [`piece_sizes`] cuts it (counted, not cut: a crafted note can say 1 GiB).
pub(super) fn pieced_footprint(kept: u64) -> u64 {
    let (page, slack) = (PAGE as u64, PAGE_SLACK as u64);
    // As `piece_sizes`: as many pieces of each size as fit, the biggest first.
    let (mut used, mut left) = (0u64, kept);
    for pages in [16u64, 8, 4, 2, 1] {
        let cap = pages * page - slack;
        used = used.saturating_add((left / cap).saturating_mul(pages * page));
        left %= cap;
    }
    if left > 0 {
        used = used.saturating_add((left + slack).div_ceil(page) * page);
    }
    used
}

/// The raw deflate of `bytes` would save a tenth or more: its length then.
fn compresses_to(bytes: &[u8]) -> Option<usize> {
    let n = miniz_oxide::deflate::compress_to_vec(bytes, 1).len();
    (n.saturating_mul(10) <= bytes.len().saturating_mul(9)).then_some(n)
}

/// What the walk of `FILES` adds up.
#[derive(Default)]
struct Walk {
    files: u64,
    kept: u64,
    live: u64,
    /// The kept bytes of the files a sample is drawn from (big enough to compress, not the
    /// index's): what the sample stands for.
    eligible_kept: u64,
    /// Of the files sampled: their kept bytes, those of the ones kept raw that would compress,
    /// and what compressing those would save (scaled from what was tried).
    sampled: u64,
    raw: u64,
    gain: u64,
}

impl Packs {
    /// How `account`'s data file is doing ([`crate::Store::data_file_health`]), from one read
    /// transaction; `None` when it has none.
    pub(crate) fn health(&self, account: &str) -> io::Result<Option<crate::DataFileHealth>> {
        let Some(path) = self.file(account) else { return Ok(None) };
        let Some(db) = self.db(account, false)? else { return Ok(None) };
        let file_bytes = fs::metadata(&path)?.len();
        let tx = db.begin_read().map_err(other)?;
        let meta = read_meta(&tx)?;
        let walk = walk(&tx, meta.deflate < DEFLATE_NOW)?;
        let overhead = u64::try_from(u128::from(walk.live) * u128::from(meta.overhead.unwrap_or(OVERHEAD_DEFAULT)) / 1_000_000).unwrap_or(u64::MAX);
        let scale = |part: u64| {
            if walk.sampled == 0 {
                0
            } else {
                u64::try_from(u128::from(part) * u128::from(walk.eligible_kept) / u128::from(walk.sampled)).unwrap_or(u64::MAX)
            }
        };
        Ok(Some(crate::DataFileHealth {
            file_bytes,
            live_bytes: walk.live,
            reclaimable: file_bytes.saturating_sub(walk.live.saturating_add(overhead)),
            files: walk.files,
            kept_bytes: walk.kept,
            raw_fraction: if walk.sampled == 0 { 0.0 } else { (walk.raw as f64 / walk.sampled as f64) as f32 },
            recompress_gain: scale(walk.gain),
            recompressed: meta.deflate >= DEFLATE_NOW,
            recompress_stopped: meta.cursor.is_some(),
        }))
    }

    /// `account`'s data file was just compacted: what it has over the walk's count now is the
    /// tree's own, and later counts allow for that much (in proportion) rather than 2%.
    pub(crate) fn compacted(&self, account: &str) -> io::Result<()> {
        let Some(path) = self.file(account) else { return Ok(()) };
        let Some(db) = self.db(account, false)? else { return Ok(()) };
        let live = walk(&db.begin_read().map_err(other)?, false)?.live;
        let over = fs::metadata(&path)?.len().saturating_sub(live);
        let ppm = if live == 0 {
            OVERHEAD_DEFAULT
        } else {
            u64::try_from(u128::from(over) * 1_000_000 / u128::from(live)).unwrap_or(u64::MAX).min(1_000_000)
        };
        seal_with(&db, &self.seals, account, |tx| write_meta(tx, &[(OVERHEAD, ppm.to_le_bytes().as_slice())]))
    }
}

/// One walk of `FILES`: every file's kept length and the bytes it takes, and when `sample`, one
/// file in [`SAMPLE_EVERY`] (big enough to compress, and not the index's) looked at: kept raw, its
/// first piece (at most [`TRIAL_MAX`]) is compressed to see whether it would be.
fn walk(tx: &redb::ReadTransaction, sample: bool) -> io::Result<Walk> {
    let mut w = Walk::default();
    let files = match tx.open_table(FILES) {
        Ok(t) => t,
        Err(redb::TableError::TableDoesNotExist(_)) => return Ok(w),
        Err(e) => return Err(disk(e)),
    };
    let pieces = match tx.open_table(PIECES) {
        Ok(t) => Some(t),
        Err(redb::TableError::TableDoesNotExist(_)) => None,
        Err(e) => return Err(disk(e)),
    };
    // The pages of the list itself, whole values and notes in them, as the database counts them
    // (a value over half a page has one to itself, and pages are as full as they were left): it
    // reads the same pages the walk below does, and none of the pieces'.
    let list = files.stats().map_err(disk)?;
    w.live = list.stored_bytes().saturating_add(list.metadata_bytes()).saturating_add(list.fragmented_bytes());
    let mut eligible = 0u64;
    for item in files.iter().map_err(disk)? {
        let (k, v) = item.map_err(disk)?;
        let (key, v) = (k.value(), v.value());
        let note = pieced_len(v);
        let kept = note.unwrap_or(v.len() as u64).min(INFLATE_MAX);
        w.files = w.files.saturating_add(1);
        w.kept = w.kept.saturating_add(kept);
        // In pieces: the page runs they fill.
        if note.is_some() {
            w.live = w.live.saturating_add(pieced_footprint(kept));
        }
        if !sample || kept < DEFLATE_MIN as u64 || key.starts_with(SEARCH) {
            continue;
        }
        w.eligible_kept = w.eligible_kept.saturating_add(kept);
        let skip = !eligible.is_multiple_of(SAMPLE_EVERY);
        eligible = eligible.saturating_add(1);
        if skip {
            continue;
        }
        let head: Vec<u8> = match (note, &pieces) {
            (None, _) => v.get(..TRIAL_MAX.min(v.len())).unwrap_or_default().to_vec(),
            (Some(_), Some(p)) => match p.get((key, 0)).map_err(disk)? {
                Some(g) => g.value().get(..TRIAL_MAX.min(g.value().len())).unwrap_or_default().to_vec(),
                None => continue,
            },
            (Some(_), None) => continue,
        };
        w.sampled = w.sampled.saturating_add(kept);
        if head.is_empty() || head.starts_with(DEFLATED) {
            continue;
        }
        // Kept raw: worth compressing again only if it does compress (a picture, an archive,
        // stays as it is).
        if let Some(n) = compresses_to(&head) {
            w.raw = w.raw.saturating_add(kept);
            let saved = u128::from(kept) * (head.len().saturating_sub(n) as u128) / head.len() as u128;
            w.gain = w.gain.saturating_add(u64::try_from(saved).unwrap_or(u64::MAX));
        }
    }
    Ok(w)
}
