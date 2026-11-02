//! A long write (a big local move): its data file sealed as it goes, so a crash part-way reopens
//! the file at once rather than checking the whole of it.

use super::commits::{child, inbox, repairs_on_open};
use super::*;

fn source(i: u32) -> Vec<u8> {
    format!("Subject: {i}\r\n\r\n{}", "the quick brown fox ".repeat(300)).into_bytes()
}

/// Killed part-way through a long write: before its first commit that waits for the disk
/// (`begun`: the file written before it was sealed as it began), or after some (`batches`), the
/// file reopens with no repair and with what the last of them wrote. The same writes with no long
/// write (`plain`) need one: each commit that waits for the disk isn't sealed by itself.
#[test]
fn a_crash_in_a_long_write_needs_no_repair() {
    for (mode, sealed) in [("plain", false), ("begun", true), ("batches", true)] {
        let t = temp(&format!("long-{mode}"));
        let s = Store::open_packed(t.join("app"), t.join("Files")).unwrap();
        s.save_accounts(&[account("a1", "zoe@studio.example")]).unwrap();
        s.write_body(&inbox(99), b"Subject: first\r\n\r\nx").unwrap();
        let file = s.data_file("a1").unwrap();
        drop(s);
        let env = [("MC_CHILD_DIR", t.display().to_string()), ("MC_CHILD_MODE", mode.to_string())];
        let out = child("packed::tests::long::killed_in_a_long_write", &env).wait_with_output().unwrap();
        assert!(!out.status.success() && String::from_utf8_lossy(&out.stdout).contains("written"), "{mode}: the child was to write, then be killed");
        let (repairs, there) = repairs_on_open(&t, &file);
        assert!(there, "{mode}: the write that waited for the disk was lost");
        assert_eq!(repairs == 0, sealed, "{mode}: {repairs} repairs");
        if mode == "batches" {
            let s = Store::open_packed(t.join("app"), t.join("Files")).unwrap();
            assert!((1000..1040).all(|i| s.read_body(&inbox(i)) == Some(source(i))), "the batches before the sealed commit are there");
            drop(s);
        }
        let _ = std::fs::remove_dir_all(&t);
    }
}

/// The child of [`a_crash_in_a_long_write_needs_no_repair`]: a write that waits for the disk (so
/// the file waits for a seal), then, in a long write but for `plain`, batches that don't wait and
/// (but for `begun`) a commit that does; then killed.
#[test]
#[ignore = "run by a_crash_in_a_long_write_needs_no_repair"]
fn killed_in_a_long_write() {
    let Some(dir) = std::env::var_os("MC_CHILD_DIR").map(std::path::PathBuf::from) else { return };
    let mode = std::env::var("MC_CHILD_MODE").unwrap_or_default();
    let s = Store::open_packed(dir.join("app"), dir.join("Files")).unwrap();
    s.write_body(&inbox(1), b"Subject: kept\r\n\r\nx").unwrap();
    let _long = (mode != "plain").then(|| s.long_write("a1"));
    let batch = |from: u32| (from..from + 20).map(|i| (inbox(i), Store::prepare(true, source(i)))).collect::<Vec<_>>();
    s.write_prepared(batch(1000), false).unwrap();
    if mode != "begun" {
        s.write_prepared(batch(1020), true).unwrap();
    }
    println!("written");
    std::process::abort();
}

/// Batches in a long write run longer before one waits for the disk (each that does is sealed,
/// which costs megabytes on a big file), and the long write ends with its guard however many
/// there were.
#[test]
fn a_long_write_lets_batches_run_longer_and_ends() {
    let t = temp("long-run");
    let s = Store::open_packed(t.join("app"), t.join("Files")).unwrap();
    s.save_accounts(&[account("a1", "zoe@studio.example")]).unwrap();
    let crate::Files::Packed(packs) = &s.files.main else { panic!("packed") };
    let over = crate::packed::commit::UNDURABLE_MAX + 1;
    let big = |uid: u32| vec![(inbox(uid), Store::prepare(false, vec![b'x'; over]))];
    s.write_body(&inbox(99), b"Subject: first\r\n\r\nx").unwrap();
    {
        let _a = s.long_write("a1");
        let _b = s.long_write("a1");
        assert!(packs.sealing("a1"));
        let before = s.commit_times();
        s.write_prepared(big(1), false).unwrap();
        assert_eq!(s.commit_times(), before, "past the usual run, the batch didn't wait for the disk");
        assert!(packs.seals.undurable("a1").0 >= over);
    }
    assert!(!packs.sealing("a1"), "over with its last guard");
    let before = s.commit_times();
    s.write_prepared(big(2), false).unwrap();
    assert_eq!(s.commit_times().sealed, before.sealed);
    assert_eq!(s.commit_times().plain.count, before.plain.count + 1, "outside it, a batch past the run waits for the disk, unsealed");
    // Other stores have no long write to take.
    drop(Store::memory().long_write("a1"));
    drop(s);
    let _ = std::fs::remove_dir_all(&t);
}

/// A background commit that began before a long write did (so isn't sealed) and commits as it
/// begins is sealed as it begins: it looked whether the file waited for a seal before that commit
/// said so, and went on unsealed, so a crash before the long write's first commit that waited for
/// the disk checked the whole file.
#[test]
fn a_long_write_seals_a_commit_begun_before_it() {
    let t = temp("long-race");
    let s = Store::open_packed(t.join("app"), t.join("Files")).unwrap();
    s.save_accounts(&[account("a1", "zoe@studio.example")]).unwrap();
    s.write_body(&inbox(99), b"Subject: first\r\n\r\nx").unwrap();
    let crate::Files::Packed(packs) = &s.files.main else { panic!("packed") };
    drop(s.long_write("a1"));
    assert!(!packs.seals.seal_due("a1"), "sealed as the first long write began");
    let ((inside, is_inside), (go, gone)) = (std::sync::mpsc::channel(), std::sync::mpsc::channel::<()>());
    std::thread::scope(|scope| {
        let background = scope.spawn(move || {
            packs.change_with("a1", crate::packed::commit::Commit::PLAIN, 0, |tables| {
                tables.files.insert("background", b"x".as_slice()).map_err(std::io::Error::other)?;
                let _ = inside.send(());
                let _ = gone.recv();
                Ok(())
            })
        });
        is_inside.recv().unwrap();
        let long = scope.spawn(|| drop(s.long_write("a1")));
        std::thread::sleep(std::time::Duration::from_millis(100));
        go.send(()).unwrap();
        background.join().unwrap().unwrap();
        long.join().unwrap();
    });
    assert!(!packs.seals.seal_due("a1"), "the background commit was left unsealed");
    drop(s);
    let _ = std::fs::remove_dir_all(&t);
}
