//! Files bigger than a batch holds through a data file: two messages with a 120 MB file each are
//! exported in one batch with the memory of one big file at a time, every byte in the file.
//!
//! The export runs in a process of its own (this test binary again, running only
//! [`export_big_files_alone`]), whose peak working set is the export's, not the making of the
//! messages' sources nor another test's.

use super::*;
use crate::data_files::scratch::scratch;
use crate::fakes::measure::peak_working_set as peak;

/// Each file's size: more than the import keeps (100 MB).
const BIG: usize = 120 << 20;
/// Where the cache to export is, for the export's own process.
const DIR_VAR: &str = "MC_EXPORT_BIG_DIR";
/// The most the export may add to its process's peak working set: one message's source and its
/// file decoded beside it (289 MB measured), and the writer's blocks on their way. Both files
/// held with their batch, as before, came to 411 MB.
const MAX_GROWTH: u64 = 350 << 20;

/// Whether this system says what a process's peak working set is ([`peak`]): elsewhere the
/// memory bound isn't checked, only the file.
const MEASURED: bool = cfg!(any(windows, target_os = "linux"));

/// The export, alone in its process: started by [`big_files_are_written_with_bounded_memory`].
#[test]
#[ignore = "the export of big_files_are_written_with_bounded_memory, in a process of its own"]
fn export_big_files_alone() {
    let Ok(dir) = std::env::var(DIR_VAR) else { return };
    let dir = std::path::PathBuf::from(dir);
    let store = Store::open(dir.join("cache")).unwrap();
    let before = peak();
    let done = export(&store, "a1", "Mail", &dir.join("Mail.pst"), NONE, &mut |_| {}).unwrap();
    assert_eq!(done.messages, 2);
    if let (Some(before), Some(after)) = (before, peak()) {
        println!("peak-growth:{}", after.saturating_sub(before));
    }
}

/// How many of the big files are in the data file at `path`, each checked byte for byte.
fn files_in(path: &Path) -> usize {
    let any = read_file(path);
    let mut seen = 0;
    for m in messages_in(&any, &folder_at(&any, &["Inbox"])) {
        use mailcraft_pst::{Attachment, AttachmentData};
        let subject = subject_of(&m);
        let n: usize = subject.trim_start_matches("Footage ").parse().unwrap();
        let rows: Vec<_> = m.attachment_table().unwrap().rows_matrix().map(|r| r.id()).collect();
        assert_eq!(rows.len(), 1);
        let a = Attachment::read(m.clone(), rows[0]).unwrap();
        let Some(AttachmentData::Binary(b)) = a.data() else { panic!("{subject}: no file") };
        let data = b.buffer();
        assert_eq!(data.len(), BIG);
        assert!(data.iter().enumerate().all(|(at, x)| *x == big_byte(n, at)), "{subject}: the file isn't as it was");
        seen += 1;
    }
    seen
}

#[test]
fn big_files_are_written_with_bounded_memory() {
    // About 1 GB, taken away when the test ends, passed or not.
    let dir = scratch("pst-export-big");
    let store = Store::open(dir.join("cache")).unwrap();
    let mut messages = Vec::new();
    for n in 1..=2u32 {
        let data = big_file(n as usize, BIG);
        let d = mime::Draft {
            to: vec![EmailAddress::new("Ana", "ana@studio.example")],
            subject: format!("Footage {n}"),
            body: "The whole take.".into(),
            attachments: vec![mime::OutAttachment {
                name: format!("take{n}.bin"),
                mime: "application/octet-stream".into(),
                data,
                ..Default::default()
            }],
            ..Default::default()
        };
        let raw = mime::build(&d, &EmailAddress::new("Riley", "riley@studio.example"), 1_790_000_000, &format!("big{n}@studio.example")).unwrap();
        drop(d);
        let key = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid: n };
        store.write_body(&key, &raw).unwrap();
        messages.push(mime::summary_from_header(&raw, key, Flags::default(), 10, 0).into());
    }
    store.save_folder("a1", "INBOX", &FolderState { validity: 1, uid_next: 3, messages, ..Default::default() }).unwrap();
    let inbox =
        mailcraft_model::Folder { account: "a1".into(), path: "INBOX".into(), role: FolderRole::Inbox, selectable: true, ..Default::default() };
    store.save_folders("a1", &[inbox]).unwrap();
    drop(store);

    // The export, in a process of its own.
    let name = format!("{}::export_big_files_alone", module_path!().split_once("::").map_or("", |(_, rest)| rest));
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", &name, "--ignored", "--nocapture", "--test-threads=1"])
        .env(DIR_VAR, &dir)
        .output()
        .unwrap();
    let said = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{said}{}", String::from_utf8_lossy(&out.stderr));
    assert!(said.contains("1 passed"), "{said}");
    match said.split("peak-growth:").nth(1).and_then(|g| g.split_whitespace().next()?.parse::<u64>().ok()) {
        Some(growth) => {
            println!("the export's peak working set grew by {} MB", growth >> 20);
            assert!(growth <= MAX_GROWTH, "the export held {} MB", growth >> 20);
        }
        None if MEASURED => panic!("the export's peak working set wasn't measured: {said}"),
        None => {}
    }

    // Both files in the data file, byte for byte.
    let path = dir.join("Mail.pst");
    assert_eq!(files_in(&path), 2);

    // The import keeps files up to 100 MB: both messages come in, their files counted as too big.
    let back = Store::open(dir.join("back")).unwrap();
    let imported = crate::pst::import(&path, &back, "b1", &mut |_| {}).unwrap();
    assert_eq!(imported.messages, 2, "{imported:?}");
    assert_eq!(imported.skips.get(&mailcraft_model::job::SkipReason::AttachmentTooBig), Some(&2), "{imported:?}");
}
