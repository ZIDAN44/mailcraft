use mailcraft_model::Flag;

use super::*;

fn key(uid: u32) -> MessageKey {
    MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 3, uid }
}

fn seen(uid: u32, on: bool) -> Op {
    Op::SetFlag { keys: vec![key(uid)], flag: Flag::Seen, on }
}

/// A store of each kind: in memory, on the folder layout, in data files; with the folder to
/// take away after.
fn stores(name: &str) -> Vec<(Store, Option<PathBuf>)> {
    let dir = std::env::temp_dir().join(format!("mailcraft-waiting-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let stores = vec![
        (Store::memory(), None),
        (Store::open(dir.join("folders")).unwrap(), Some(dir.join("folders"))),
        (Store::open_packed(dir.join("packed").join("app"), dir.join("packed").join("Files")).unwrap(), Some(dir.join("packed"))),
    ];
    for (s, _) in &stores {
        s.save_accounts(&[AccountConfig { id: "a1".into(), email: "zoe@studio.example".into(), ..Default::default() }]).unwrap();
    }
    stores
}

/// Each change taken off as it's made leaves on disk just the ones not made yet: a stop after
/// any of them (the app closed, the connection lost) replays the rest, none twice, none lost.
/// Two equal changes queued apart each come off in their turn.
#[test]
fn each_change_made_leaves_just_the_rest_on_disk() {
    for (s, dir) in stores("rest") {
        let ops = vec![seen(1, true), seen(2, true), seen(1, true), Op::Move { keys: vec![key(3)], to: "Archive".into() }, seen(1, false)];
        s.save_ops("a1", &ops).unwrap();
        let mut waiting = s.waiting_ops("a1").unwrap();
        assert_eq!(waiting.iter().cloned().collect::<Vec<_>>(), ops);
        for made in 1..=ops.len() {
            assert_eq!(waiting.first(), ops.get(made - 1));
            s.op_made("a1", &mut waiting).unwrap();
            assert_eq!(s.load_ops("a1").unwrap(), ops[made..], "after {made}");
            assert_eq!(waiting.iter().cloned().collect::<Vec<_>>(), ops[made..]);
            // Stopped here: read again, the same rest.
            assert_eq!(s.waiting_ops("a1").unwrap().iter().cloned().collect::<Vec<_>>(), ops[made..]);
        }
        assert!(waiting.is_empty() && waiting.first().is_none());
        assert!(s.load_ops("a1").unwrap().is_empty());
        if let Some(dir) = dir {
            assert!(!dir.join("a1").join("ops.json").exists(), "no file left");
            drop(s);
            let _ = fs::remove_dir_all(dir);
        }
    }
}

/// A change queued while others replay (the window's, while the first is on its way) is kept
/// and comes after them, and one queued after the last is taken off still waits.
#[test]
fn a_change_queued_meanwhile_is_kept_and_comes_last() {
    for (s, dir) in stores("queued") {
        s.save_ops("a1", &[seen(1, true), seen(2, true)]).unwrap();
        let mut waiting = s.waiting_ops("a1").unwrap();
        s.add_op("a1", seen(3, true)).unwrap();
        s.op_made("a1", &mut waiting).unwrap();
        assert_eq!(s.load_ops("a1").unwrap(), [seen(2, true), seen(3, true)]);
        assert_eq!(waiting.iter().cloned().collect::<Vec<_>>(), [seen(2, true), seen(3, true)], "read in");
        s.op_made("a1", &mut waiting).unwrap();
        s.op_made("a1", &mut waiting).unwrap();
        assert!(waiting.is_empty() && s.load_ops("a1").unwrap().is_empty());
        // Queued after the last was read: still on disk for the next pass.
        s.save_ops("a1", &[seen(4, true)]).unwrap();
        let mut waiting = s.waiting_ops("a1").unwrap();
        s.op_made("a1", &mut waiting).unwrap();
        s.add_op("a1", seen(5, true)).unwrap();
        assert!(waiting.is_empty());
        assert_eq!(s.load_ops("a1").unwrap(), [seen(5, true)]);
        if let Some(dir) = dir {
            drop(s);
            let _ = fs::remove_dir_all(dir);
        }
    }
}

/// The account removed while its changes replay: its queue isn't written back.
#[test]
fn a_removed_account_s_queue_isnt_written_back() {
    for (s, dir) in stores("removed") {
        s.save_ops("a1", &[seen(1, true), seen(2, true)]).unwrap();
        let mut waiting = s.waiting_ops("a1").unwrap();
        s.delete_account("a1").unwrap();
        s.op_made("a1", &mut waiting).unwrap();
        assert!(waiting.is_empty());
        assert!(s.load_ops("a1").unwrap().is_empty());
        if let Some(dir) = dir {
            assert!(!dir.join("a1").join("ops.json").exists());
            drop(s);
            let _ = fs::remove_dir_all(dir);
        }
    }
}

/// A queue that can't be written stops the replay with the change still first, on disk and
/// here; a damaged one is set aside and reads as none, as `load_ops` has it.
#[test]
fn a_queue_that_cant_be_written_keeps_the_change() {
    let s = Store::failing();
    s.save_ops("a1", &[seen(1, true), seen(2, true)]).unwrap();
    let mut waiting = s.waiting_ops("a1").unwrap();
    s.fail(None, Some("ops.json"));
    assert!(s.op_made("a1", &mut waiting).is_err());
    s.fail(None, None);
    assert_eq!(waiting.first(), Some(&seen(1, true)));
    assert_eq!(s.load_ops("a1").unwrap(), [seen(1, true), seen(2, true)]);
    s.fail(Some("ops.json"), None);
    assert!(s.waiting_ops("a1").is_err() && s.op_made("a1", &mut waiting).is_err());
    s.fail(None, None);

    let s = Store::memory_from(vec![("a1/ops.json".into(), b"[{\"SetFl".to_vec())]);
    let waiting = s.waiting_ops("a1").unwrap();
    assert!(waiting.is_empty() && s.load_ops("a1").unwrap().is_empty());
}
