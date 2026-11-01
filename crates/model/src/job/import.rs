//! A data file's import as the engine and the user see it: how it's going, what it came to
//! (every loss counted by its reason), what a file holds before it's imported, and where an
//! account's mail came from.

use serde::{Deserialize, Deserializer, Serialize};

use super::progress::{Work, permille};

/// A value read back that this version may not know (a newer one wrote it), or that isn't one at
/// all: the value when it reads, else `fallback`, so the record around it still reads.
fn known_or<'de, D: Deserializer<'de>, T: Deserialize<'de>>(d: D, fallback: T) -> Result<T, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Read<T> {
        Known(T),
        Unknown(serde::de::IgnoredAny),
    }
    Ok(match Read::<T>::deserialize(d)? {
        Read::Known(v) => v,
        Read::Unknown(_) => fallback,
    })
}

fn known_or_default<'de, D: Deserializer<'de>, T: Deserialize<'de> + Default>(d: D) -> Result<T, D::Error> {
    known_or(d, T::default())
}

/// A stage this version doesn't know reads as stopped: nothing happens by itself, and what came
/// in is kept.
fn known_stage<'de, D: Deserializer<'de>>(d: D) -> Result<ImportStage, D::Error> {
    known_or(d, ImportStage::Stopped)
}

/// What one item weighs in an import's progress besides its own bytes: reading an item costs
/// about as long as reading this many bytes (measured on a large `.pst`: ~2 ms an item, ~18 ns a
/// byte), so a folder of many small messages isn't shown as nearly free.
pub const ITEM_WEIGHT: u64 = 128 * 1024;

/// Where an import is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ImportPhase {
    /// Walking the file's folders for the totals.
    #[default]
    Planning,
    Importing,
    /// Everything read; the last of it being written.
    Finishing,
    /// Asked to stop; finishing the item in hand.
    Stopping,
    /// Stopped, to go on the next time MailCraft opens.
    Paused,
    /// Waiting for something the user must sort out ([`ImportProgress::blocked`]).
    Blocked,
}

/// Why an import can't go on by itself.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Blocked {
    /// The data file isn't where it was.
    SourceMissing,
    /// The data file changed since the import began.
    SourceChanged,
    /// Too little room left for the copy (`free` bytes).
    DiskFull { free: u64 },
    /// Another program has the file and won't share it.
    Locked,
}

/// How an import is stopped: kept and continued when asked (`Cancel`), kept and continued the
/// next time MailCraft opens (`Pause`), or thrown away with the account (`Discard`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StopHow {
    #[default]
    Cancel,
    Pause,
    Discard,
}

/// Where an account's import stands, as kept between runs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ImportStage {
    #[default]
    Running,
    /// Cancelled: what came in is kept, and it can go on.
    Stopped,
    /// The account is being removed: its file goes once the import has let go of it.
    Removing,
    Done,
}

/// How an import is going: sent by the network layer as it goes (a newer one replaces an older
/// one). Times are the worker's own, so the engine never reads a clock.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ImportProgress {
    #[serde(deserialize_with = "known_or_default")]
    pub phase: ImportPhase,
    /// Items read and their bytes (the file's own message sizes, not what is written), of
    /// the totals the walk over the folders found (0: not known).
    pub work: Work,
    pub folders_done: u32,
    pub folders_total: u32,
    /// The folder being read ("" while planning).
    pub folder: String,
    /// Items written, and items left out.
    pub imported: u64,
    pub skipped: u64,
    /// Time spent on this import over all its runs, and in this run.
    pub elapsed_ms: u64,
    pub run_ms: u64,
    /// The work done when this run began (an import that goes on from where it stopped).
    pub run_start_work: u64,
    #[serde(deserialize_with = "known_or_default")]
    pub blocked: Option<Blocked>,
}

impl ImportProgress {
    /// The work done, each item weighing [`ITEM_WEIGHT`] besides its bytes.
    pub fn work_done(&self) -> u64 {
        self.work.weighted_done(ITEM_WEIGHT)
    }

    pub fn work_total(&self) -> u64 {
        self.work.weighted_total(ITEM_WEIGHT)
    }

    /// Thousandths done (0..=999), `None` when the total isn't known.
    pub fn permille(&self) -> Option<u16> {
        permille(self.work_done(), self.work_total())
    }
}

/// Why something in a data file didn't come in (each has a sentence of its own in the
/// interface). The order is the order a report lists them in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SkipReason {
    MessageUnreadable,
    MessageNotRebuilt,
    NotWritten,
    /// A whole folder's list couldn't be written (its messages are in the file, unlisted).
    ListsUnwritten,
    AttachmentUnreadable,
    AttachmentTooBig,
    /// An attachment that only links to a file elsewhere (nothing to bring in).
    AttachmentLink,
    AttachedMessageUnreadable,
    AttachedMessageFilesLeftOut,
    /// Messages attached inside messages, deeper than MailCraft follows.
    EmbeddedTooDeep,
    RecipientWithoutAddress,
    BodyNotConverted,
    /// A body in a character set MailCraft can't read.
    CharsetUnsupported,
    FolderUnreadable,
    FolderTooDeep,
    /// A folder of a kind MailCraft has no place for (RSS feeds, a list of rules), holding items; and
    /// those items.
    FolderLeftOut,
    ItemInFolderLeftOut,
    /// A contact, calendar item, task or note that couldn't be read at all…
    ItemUnreadable,
    /// …or that was read but can't be used (an empty note, a task with no date, a contact or
    /// a sharing invitation in a mail folder).
    ItemNotUsable,
    /// The file is damaged past this point: reading stopped.
    StoppedEarly,
    /// A reason a newer MailCraft wrote that this one doesn't know, or none at all.
    #[default]
    #[serde(other)]
    Other,
}

/// How many items were left out for one reason.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SkipCount {
    pub reason: SkipReason,
    pub count: u64,
}

/// What an import came to, run by run: kept with the account, and shown at the end.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ImportReport {
    /// The data file read, and the file it was copied into.
    pub file: String,
    pub data_file: String,
    pub messages: u64,
    pub folders: u32,
    pub contacts: u64,
    pub events: u64,
    pub notes: u64,
    /// What was left out, by reason: only reasons that happened, in [`SkipReason`]'s order.
    pub skips: Vec<SkipCount>,
    /// How it was stopped, when it was; and the folder it was in, when that was mid-folder.
    #[serde(deserialize_with = "known_or_default")]
    pub stopped: Option<StopHow>,
    pub stopped_in: Option<String>,
    pub elapsed_ms: u64,
    pub runs: u32,
}

impl ImportReport {
    /// Count `count` more items left out for `reason`, keeping [`ImportReport::skips`] in order.
    pub fn skip(&mut self, reason: SkipReason, count: u64) {
        if count == 0 {
            return;
        }
        match self.skips.binary_search_by_key(&reason, |s| s.reason) {
            Ok(i) => {
                if let Some(s) = self.skips.get_mut(i) {
                    s.count = s.count.saturating_add(count);
                }
            }
            Err(i) => self.skips.insert(i, SkipCount { reason, count }),
        }
    }

    /// Items left out for `reason`.
    pub fn skipped_for(&self, reason: SkipReason) -> u64 {
        self.skips.iter().filter(|s| s.reason == reason).fold(0, |n, s| n.saturating_add(s.count))
    }

    /// Items left out for any reason.
    pub fn skipped(&self) -> u64 {
        self.skips.iter().fold(0, |n, s| n.saturating_add(s.count))
    }

    /// A report read back from a file (hostile): its losses put in order, each reason once, none
    /// of zero.
    pub fn tidy(&mut self) {
        let skips = std::mem::take(&mut self.skips);
        for s in skips {
            self.skip(s.reason, s.count);
        }
    }
}

/// What a data file holds and what importing it takes, found before it's imported.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DataFileFacts {
    pub path: String,
    /// "pst", "pst-ansi", "ost" or "mailcraft".
    pub kind: String,
    pub size: u64,
    /// Last changed (Unix seconds).
    pub modified: i64,
    /// The file's own record key (hex; "" when not known), the same however it's named.
    pub record_key: String,
    pub messages: u64,
    pub folders: u32,
    pub pim_items: u64,
    /// The messages' own sizes, summed.
    pub bytes: u64,
    /// How big its copy is expected to be, and the room asked for to write it.
    pub estimate: u64,
    pub need: u64,
    /// Room left where it would be written (`None`: couldn't be told), and that folder.
    pub free: Option<u64>,
    pub destination: String,
    /// Why the file couldn't be looked at.
    pub error: Option<String>,
}

/// Where an account's mail came from: a data file, as it was when its import began, and how far
/// that import got.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DataSource {
    pub path: String,
    pub size: u64,
    pub modified: i64,
    pub record_key: String,
    #[serde(deserialize_with = "known_stage")]
    pub stage: ImportStage,
    pub report: Option<ImportReport>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_weighs_items_and_bytes() {
        let mut p = ImportProgress::default();
        assert_eq!(p.permille(), None, "nothing planned: indeterminate");
        p.work = Work { done: 1, total: 2, bytes_done: 0, bytes_total: 0 };
        assert_eq!(p.permille(), Some(500));
        // A small message read, a big one to go: the bytes weigh more than the items.
        p.work = Work { done: 1, total: 2, bytes_done: 1024, bytes_total: 1024 + 50 * ITEM_WEIGHT };
        assert_eq!(p.permille(), Some(19));
        p.work = Work { done: u64::MAX, total: u64::MAX, bytes_done: u64::MAX, bytes_total: u64::MAX };
        assert_eq!(p.permille(), Some(999), "saturated totals stay below the end");
    }

    #[test]
    fn a_report_keeps_its_losses_in_order() {
        let mut r = ImportReport::default();
        r.skip(SkipReason::BodyNotConverted, 3);
        r.skip(SkipReason::MessageUnreadable, 1);
        r.skip(SkipReason::AttachmentTooBig, 0);
        r.skip(SkipReason::BodyNotConverted, u64::MAX);
        let reasons: Vec<SkipReason> = r.skips.iter().map(|s| s.reason).collect();
        assert_eq!(reasons, [SkipReason::MessageUnreadable, SkipReason::BodyNotConverted], "no zeros, in order");
        assert_eq!(r.skipped_for(SkipReason::BodyNotConverted), u64::MAX, "saturated");
        assert_eq!(r.skipped(), u64::MAX);

        // Read back from a file: repeated, zero and out of order, and a reason from a newer version.
        let mut r: ImportReport = serde_json::from_str(
            r#"{"skips":[{"reason":"stoppedEarly","count":1},{"reason":"notWritten","count":0},
                {"reason":"somethingNew","count":2},{"reason":"stoppedEarly","count":4}],"runs":2}"#,
        )
        .unwrap();
        r.tidy();
        assert_eq!(r.skips, [SkipCount { reason: SkipReason::StoppedEarly, count: 5 }, SkipCount { reason: SkipReason::Other, count: 2 }]);
        assert_eq!(r.runs, 2);
    }

    #[test]
    fn records_round_trip_and_read_old_ones() {
        let s = DataSource {
            path: "C:/Mail/archive.pst".into(),
            size: 9,
            modified: 1_700_000_000,
            record_key: "00ff".into(),
            stage: ImportStage::Removing,
            report: Some(ImportReport { messages: 4, stopped: Some(StopHow::Pause), ..Default::default() }),
        };
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("\"recordKey\"") && json.contains("\"removing\"") && json.contains("\"pause\""), "{json}");
        assert_eq!(serde_json::from_str::<DataSource>(&json).unwrap(), s);
        // Fields a later version adds, or an earlier one lacks, don't stop it from reading.
        let old: DataSource = serde_json::from_str(r#"{"path":"a.pst","extra":1}"#).unwrap();
        assert_eq!((old.path.as_str(), old.stage, old.report), ("a.pst", ImportStage::Running, None));
        // A stage, stop or phase a newer version wrote reads, and the rest of the record with it.
        let newer: DataSource = serde_json::from_str(r#"{"path":"a.pst","stage":"compacting","report":{"messages":3,"stopped":"suspend"}}"#).unwrap();
        assert_eq!((newer.path.as_str(), newer.stage), ("a.pst", ImportStage::Stopped), "an unknown stage does nothing by itself");
        assert_eq!(newer.report.map(|r| (r.messages, r.stopped)), Some((3, None)));
        let p: ImportProgress = serde_json::from_str(r#"{"phase":"verifying","blocked":{"kind":"offline"},"imported":2}"#).unwrap();
        assert_eq!((p.phase, p.blocked, p.imported), (ImportPhase::Planning, None, 2));
        let stopped: ImportReport = serde_json::from_str(r#"{"stopped":"pause","stoppedIn":null}"#).unwrap();
        assert_eq!(stopped.stopped, Some(StopHow::Pause));
        // A loss with no reason isn't put down to any one.
        let mut r: ImportReport = serde_json::from_str(r#"{"skips":[{"count":5}]}"#).unwrap();
        r.tidy();
        assert_eq!(r.skips, [SkipCount { reason: SkipReason::Other, count: 5 }]);
        let p = ImportProgress { blocked: Some(Blocked::DiskFull { free: 7 }), phase: ImportPhase::Blocked, ..Default::default() };
        assert_eq!(serde_json::from_str::<ImportProgress>(&serde_json::to_string(&p).unwrap()).unwrap(), p);
    }
}
