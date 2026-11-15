//! An account switched between its data file and memory (online only, issue #4), measured: how
//! long it takes and how much memory it needs at most, the account's size given.

use super::measure::Process;
use super::*;

/// The account: this many sources of 1 MiB, each different and not compressible.
const SOURCES: u32 = 1024;
const SOURCE: usize = 1 << 20;

fn key(account: &str, uid: u32) -> MessageKey {
    MessageKey { account: account.into(), folder: "INBOX".into(), validity: 1, uid }
}

fn source(uid: u32) -> Vec<u8> {
    let mut rng = crate::tests::random::Rng((0x9E37_79B9_7F4A_7C15u64 ^ u64::from(uid).wrapping_mul(0x2545_F491_4F6C_DD1D)) | 1);
    let mut v = format!("Subject: {uid}\r\n\r\n").into_bytes();
    while v.len() < SOURCE {
        v.extend_from_slice(&rng.next_u64().to_le_bytes());
    }
    v.truncate(SOURCE);
    v
}

/// What every source of `account` reads back as, in one number.
fn digest(s: &Store, account: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for uid in 0..SOURCES {
        s.read_body(&key(account, uid)).unwrap_or_else(|| panic!("{account} {uid}: gone")).hash(&mut h);
    }
    h.finish()
}

fn expected() -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for uid in 0..SOURCES {
        source(uid).hash(&mut h);
    }
    h.finish()
}

/// The most memory this process has had so far (MiB): its working set and its private bytes, as
/// the system counts them.
fn peak() -> (u64, u64) {
    let p = Process::now().expect("the system's counts for this process");
    (p.peak >> 20, p.peak_private >> 20)
}

/// Run by [`measure_online_only_switch`] in a process of its own, so its peak is the switch's:
/// `on` takes the account its parent wrote into memory, `off` fills an online-only account in
/// memory and writes it out to a data file.
#[test]
#[ignore = "run by measure_online_only_switch"]
fn switched_in_a_child() {
    let (Ok(dir), Ok(mode)) = (std::env::var("MC_CHILD_DIR"), std::env::var("MC_SWITCH")) else { return };
    let t = std::path::PathBuf::from(dir);
    let s = Store::open_packed(t.join("app"), t.join("Files")).unwrap();
    let account = if mode == "on" { "a1" } else { "a2" };
    if mode == "off" {
        s.set_online_only(account, true).unwrap();
        for uid in 0..SOURCES {
            s.write_body(&key(account, uid), &source(uid)).unwrap();
        }
    }
    let before = peak();
    let start = std::time::Instant::now();
    s.set_online_only(account, mode == "on").unwrap();
    let took = start.elapsed();
    let after = peak();
    let size = (u64::from(SOURCES) * SOURCE as u64) >> 20;
    println!(
        "MEASURE online only {mode}: {size} MiB account, {} ms, peak working set {} -> {} MiB, peak private {} -> {} MiB",
        took.as_millis(),
        before.0,
        after.0,
        before.1,
        after.1
    );
    // What it has reads back the same, there or (once written out and opened again) on disk.
    if mode == "off" {
        drop(s);
        let s = Store::open_packed(t.join("app"), t.join("Files")).unwrap();
        let file = s.data_file(account).unwrap();
        println!("MEASURE online only off: data file {} bytes", std::fs::metadata(file).unwrap().len());
        assert_eq!(digest(&s, account), expected());
    } else {
        assert_eq!(digest(&s, account), expected());
    }
    println!("MEASURE online only {mode}: sources read back as written");
}

/// Release, by hand: an account of 1 GiB switched to memory and back, each in a process of its
/// own (the account written to disk here first).
#[test]
#[ignore = "a measurement: holds 1 GiB and more"]
fn measure_online_only_switch() {
    let t = temp("online-measure");
    {
        let s = Store::open_packed(t.join("app"), t.join("Files")).unwrap();
        s.save_accounts(&[account("a1", "zoe@studio.example"), account("a2", "ana@studio.example")]).unwrap();
        for batch in (0..SOURCES).collect::<Vec<_>>().chunks(32) {
            let bodies: Vec<_> = batch.iter().map(|&uid| (key("a1", uid), source(uid))).collect();
            s.write_bodies(bodies).unwrap();
        }
    }
    for mode in ["on", "off"] {
        let env = [("MC_CHILD_DIR", t.display().to_string()), ("MC_SWITCH", mode.to_string())];
        let out = child("packed::tests::online::switched_in_a_child", &env).wait_with_output().unwrap();
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(out.status.success(), "{mode}: {text}\n{}", String::from_utf8_lossy(&out.stderr));
        // (The child's first line follows its test's name.)
        for line in text.lines().filter_map(|l| l.find("MEASURE").and_then(|at| l.get(at..))) {
            println!("{line}");
        }
    }
}
