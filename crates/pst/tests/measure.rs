//! Timings of the writer, run by hand in release:
//! `cargo test --release -p mailcraft-pst --test measure -- --ignored --nocapture`.

use std::path::{Path, PathBuf};
use std::time::Instant;

use mailcraft_pst::Format;
use mailcraft_pst::write::{self, BatchMessage, NewMessage, SpecialFolder};

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mailcraft-pst-measure-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    let _ = std::fs::remove_file(&path);
    path
}

/// Where a timing's file starts from: with `MAILCRAFT_PST_MEASURE_BASE` set, a copy of the file
/// `name` in that folder (made there when missing), so two builds start from the same bytes and
/// what they write can be compared; else a new one.
fn start_from(name: &str, make: impl Fn(&Path)) -> PathBuf {
    let work = temp(name);
    match std::env::var_os("MAILCRAFT_PST_MEASURE_BASE") {
        Some(dir) => {
            let base = Path::new(&dir).join(name);
            if !base.exists() {
                make(&base);
            }
            std::fs::copy(&base, &work).unwrap();
        }
        None => make(&work),
    }
    work
}

/// The file's bytes, as one number (FNV-1a).
fn digest(path: &Path) -> u64 {
    std::fs::read(path).unwrap().iter().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3))
}

/// `n` small messages, numbered from `from`, added to the folder at `folder`.
fn add(path: &Path, folder: &[&str], from: usize, n: usize) {
    let subjects: Vec<String> = (from..from + n).map(|i| format!("Message {i}")).collect();
    let ids: Vec<String> = (from..from + n).map(|i| format!("<m{i}@example.org>")).collect();
    let batch: Vec<BatchMessage<'_>> = subjects
        .iter()
        .zip(&ids)
        .enumerate()
        .map(|(i, (subject, id))| BatchMessage {
            message: NewMessage {
                subject,
                sender_name: "Ann",
                sender_email: "ann@example.org",
                recipients: &[],
                body: "A short body.",
                html_body: None,
                message_id: id,
                delivery_time: 133_000_000_000_000_000 + i64::try_from(from + i).unwrap() * 10_000_000,
                message_class: "IPM.Note",
                properties: &[],
            },
            attachments: &[],
        })
        .collect();
    write::append(path, folder, &batch).unwrap();
}

/// Small batches added to a file that already holds much: each addition opens the file to change
/// it, and each opening reads its maps and both of its B-trees through.
#[test]
#[ignore = "a measurement"]
fn measure_small_batches_into_a_full_file() {
    let path = start_from("full.pst", |p| {
        write::create_empty(p, Format::Unicode).unwrap();
        for b in 0..10 {
            add(p, &["Archive"], b * 2000, 2000);
        }
    });
    let t = Instant::now();
    for b in 0..40 {
        add(&path, &["Inbox"], 100_000 + b * 25, 25);
        add(&path, &["Mail", "2026", "Kept"], 200_000 + b * 25, 25);
    }
    println!("MEASURE 80 batches of 25 into a file of 20000: {} ms (file {:016x})", t.elapsed().as_millis(), digest(&path));
}

/// Naming the special folders of a file with many folders.
#[test]
#[ignore = "a measurement"]
fn measure_special_folders_among_many_folders() {
    let path = temp("many-folders.pst");
    write::create_empty(&path, Format::Unicode).unwrap();
    for i in 0..200 {
        let top = format!("Folder {i}");
        for j in 0..5 {
            write::create_folder(&path, &[top.as_str(), &format!("Sub {j}")], "IPF.Note").unwrap();
        }
    }
    write::set_special_folder(&path, SpecialFolder::SentItems, &["Folder 100", "Sub 2"]).unwrap();
    write::set_special_folder(&path, SpecialFolder::Tasks, &["Folder 0", "Sub 4"]).unwrap();
    let t = Instant::now();
    for _ in 0..20 {
        assert_eq!(write::special_folder(&path, SpecialFolder::SentItems).unwrap(), Some(vec!["Folder 100".to_string(), "Sub 2".to_string()]));
        assert_eq!(write::special_folder(&path, SpecialFolder::Tasks).unwrap(), Some(vec!["Folder 0".to_string(), "Sub 4".to_string()]));
    }
    println!("MEASURE 40 special folder lookups among 1200 folders: {} ms", t.elapsed().as_millis());
}
