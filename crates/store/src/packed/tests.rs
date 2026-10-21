use mailcraft_model::{AccountConfig, Flags, MessageKey, Summary};

use crate::Store;

fn temp(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("mailcraft-packed-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn account(id: &str, email: &str) -> AccountConfig {
    AccountConfig { id: id.into(), email: email.into(), ..Default::default() }
}

/// A new account starts from an existing data file: its folders, mail and documents come in,
/// the messages as this account's; junk is refused and paths that climb out are dropped.
#[test]
fn an_account_starts_from_an_existing_data_file() {
    use mailcraft_model::{Folder, FolderRole};
    let t = temp("from-file");
    let old = Store::open_packed(t.join("old"), t.join("Old Files")).unwrap();
    old.save_accounts(&[account("a1", "zoe@studio.example")]).unwrap();
    old.save_folders("a1", &[Folder { path: "INBOX".into(), role: FolderRole::Inbox, ..Default::default() }]).unwrap();
    let k = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid: 7 };
    old.write_body(&k, b"Subject: hi\r\n\r\nhello").unwrap();
    let mut st = old.load_folder("a1", "INBOX").unwrap();
    st.validity = 1;
    st.messages.push(Summary { key: k.clone(), subject: "hi".into(), ..Default::default() });
    old.save_folder("a1", "INBOX", &st).unwrap();
    old.save_account_doc("a1", "pim.json", &serde_json::json!({"notes": []})).unwrap();
    // Changes still to make on its server, and mail still to send: they stay behind.
    old.save_ops("a1", &[mailcraft_model::Op::Expunge { keys: vec![k.clone()] }]).unwrap();
    old.outbox_add("a1", &crate::OutboxItem { id: 4, ..Default::default() }, b"Subject: later\r\n\r\nx").unwrap();
    let file = old.data_file("a1").unwrap();
    drop(old);

    let s = Store::open_packed(t.join("app"), t.join("MailCraft Files")).unwrap();
    s.save_accounts(&[account("p7", "ana@example.org")]).unwrap();
    assert_eq!(s.import_data_file("p7", &file).unwrap(), 1);
    let folders = s.load_folders("p7").unwrap();
    assert_eq!(folders.len(), 1);
    assert_eq!(folders[0].account, "p7", "its folders are this account's (opening one named the old account)");
    assert!(s.load_ops("p7").unwrap().is_empty() && s.outbox("p7").is_empty(), "the old account's pending work stays behind");
    let st = s.load_folder("p7", "INBOX").unwrap();
    let mine = MessageKey { account: "p7".into(), ..k.clone() };
    assert_eq!(st.messages[0].key, mine, "renumbered as this account's");
    assert_eq!(s.read_body(&mine).unwrap(), b"Subject: hi\r\n\r\nhello");
    assert_eq!(s.load_account_doc::<serde_json::Value>("p7", "pim.json").unwrap(), serde_json::json!({"notes": []}));
    assert!(file.exists(), "the file it came from stays");
    assert!(!t.join("app").join(".import-p7.tmp").exists(), "the copy read from is gone");

    // Junk: refused, and nothing written.
    let junk = t.join("junk.mailcraft");
    std::fs::write(&junk, b"not a database at all").unwrap();
    assert!(s.import_data_file("p8", &junk).is_err());
    assert!(s.load_folders("p8").unwrap().is_empty());

    // A crafted file: paths that climb out of the account are left out.
    let crafted = t.join("crafted.mailcraft");
    {
        let db = redb::Database::create(&crafted).unwrap();
        let tx = db.begin_write().unwrap();
        {
            let mut table = tx.open_table(super::FILES).unwrap();
            for key in ["../escape.json", "a/../../b", "C:/x", "/root", "ok/fine.json"] {
                table.insert(key, b"x".as_slice()).unwrap();
            }
        }
        tx.commit().unwrap();
    }
    s.import_data_file("p9", &crafted).unwrap();
    assert!(s.files.exists(&t.join("app").join("p9").join("ok").join("fine.json")));
    assert!(!t.join("app").join("escape.json").exists() && !t.join("escape.json").exists() && !t.join("b").exists());
    let _ = std::fs::remove_dir_all(&t);
}
#[test]
fn each_account_is_one_data_file_named_after_it() {
    let t = temp("one-file");
    let s = Store::open_packed(t.join("app"), t.join("MailCraft Files")).unwrap();
    s.save_accounts(&[account("a1", "zoe@studio.example"), account("a2", "old.pst")]).unwrap();
    let k = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid: 7 };
    s.write_body(&k, b"Subject: hi\r\n\r\nhello").unwrap();
    let mut st = s.load_folder("a1", "INBOX").unwrap();
    st.validity = 1;
    st.messages.push(Summary { key: k.clone(), subject: "hi".into(), flags: Flags::default(), ..Default::default() });
    s.save_folder("a1", "INBOX", &st).unwrap();
    s.save_account_doc("a2", "pim.json", &serde_json::json!({"x": 1})).unwrap();
    // One file per account, named as Outlook names a .pst; nothing loose in the app folder.
    let mut files: Vec<String> =
        std::fs::read_dir(t.join("MailCraft Files")).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    files.sort();
    assert_eq!(files, ["old.mailcraft", "zoe@studio.example.mailcraft"]);
    assert!(!t.join("app").join("a1").exists());
    assert_eq!(s.data_file("a1"), Some(t.join("MailCraft Files").join("zoe@studio.example.mailcraft")));
    // Read back, also after opening again.
    drop(s);
    let s = Store::open_packed(t.join("app"), t.join("MailCraft Files")).unwrap();
    assert_eq!(s.read_body(&k).unwrap(), b"Subject: hi\r\n\r\nhello");
    assert_eq!(s.load_folder("a1", "INBOX").unwrap().messages.len(), 1);
    assert!(s.account_size("a1") > 0);
    // Folder renames move inside the file; the account going takes its file.
    s.save_folders("a1", &[mailcraft_model::Folder { account: "a1".into(), path: "INBOX".into(), selectable: true, ..Default::default() }]).unwrap();
    s.apply_op("a1", &mailcraft_model::Op::RenameFolder { from: "INBOX".into(), to: "Kept".into() }).unwrap();
    assert_eq!(s.load_folder("a1", "Kept").unwrap().messages.len(), 1);
    assert!(s.read_body(&MessageKey { folder: "Kept".into(), ..k.clone() }).is_some(), "its messages moved too");
    // Removing the account keeps its file, as Outlook keeps a .pst: closed, and no one's.
    let kept = t.join("MailCraft Files").join("zoe@studio.example.mailcraft");
    assert_eq!(s.remove_account("a1").unwrap(), Some(kept.clone()));
    assert!(kept.exists() && s.data_file("a1").is_none());
    // It still reads (as a file opened from Open & Export).
    let mut keys = Vec::new();
    super::read_data_file(&kept, &t.join("read.tmp"), &mut |batch| {
        keys.extend(batch.into_iter().map(|(k, _)| k));
        Ok(())
    })
    .unwrap();
    assert!(keys.iter().any(|k| k.ends_with(".eml")) && keys.iter().any(|k| k == "folders.json"), "{keys:?}");
    // The same address added again: a file of its own beside the kept one, which stays as it was.
    let before = std::fs::read(&kept).unwrap();
    s.save_accounts(&[account("a1", "zoe@studio.example")]).unwrap();
    s.save_folder("a1", "INBOX", &st).unwrap();
    assert_eq!(s.data_file("a1"), Some(t.join("MailCraft Files").join("zoe@studio.example (2).mailcraft")));
    assert_eq!(std::fs::read(&kept).unwrap(), before);
    // Deleting (what never held anyone's mail) takes the file.
    s.delete_account("a1").unwrap();
    assert!(!t.join("MailCraft Files").join("zoe@studio.example (2).mailcraft").exists() && kept.exists());
    let _ = std::fs::remove_dir_all(t);
}

/// Big files go in pieces that fill their pages (a whole value took the next power of two
/// of pages: imported mail came to more than twice its size), and come back whole, through
/// renames, removals, overwrites and copies into another data file.
#[test]
fn big_files_are_kept_in_pieces_and_come_back_whole() {
    let t = temp("pieces");
    let s = Store::open_packed(t.join("app"), t.join("Files")).unwrap();
    s.save_accounts(&[account("a1", "zoe@studio.example")]).unwrap();
    // Bytes that don't compress, so they're kept in pieces as they are.
    let big = noise(1_234_567, 1);
    let edge = noise(super::WHOLE_MAX + 1, 2);
    let small = b"Subject: small

x"
    .to_vec();
    let key = |uid: u32| MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid };
    s.write_bodies(&[(key(1), big.clone()), (key(2), edge.clone()), (key(3), small.clone())]).unwrap();
    assert_eq!(s.read_body(&key(1)).unwrap(), big);
    assert_eq!(s.read_body(&key(2)).unwrap(), edge);
    assert_eq!(s.read_body(&key(3)).unwrap(), small);
    assert!(s.has_body(&key(1)));
    let dir = s.body_path(&key(1)).parent().unwrap().to_path_buf();
    // Sizes are what the files take (compressed: this test's bytes repeat), not the notes'.
    let kept = s.files.size(&dir);
    assert!(kept > 1_000 && kept <= (big.len() + edge.len() + small.len()) as u64, "{kept}");
    // Overwritten smaller: no pieces left behind; and bigger again.
    s.write_body(&key(1), &small).unwrap();
    assert_eq!(s.read_body(&key(1)).unwrap(), small);
    s.write_body(&key(1), &big).unwrap();
    assert_eq!(s.read_body(&key(1)).unwrap(), big);
    // A folder renamed takes its pieces.
    s.save_folders("a1", &[mailcraft_model::Folder { account: "a1".into(), path: "INBOX".into(), selectable: true, ..Default::default() }]).unwrap();
    s.apply_op("a1", &mailcraft_model::Op::RenameFolder { from: "INBOX".into(), to: "Kept".into() }).unwrap();
    let kept = |uid: u32| MessageKey { folder: "Kept".into(), ..key(uid) };
    assert_eq!(s.read_body(&kept(1)).unwrap(), big);
    assert!(s.read_body(&key(1)).is_none());
    // Removed: gone, pieces and all.
    s.remove_body(&kept(2));
    assert!(s.read_body(&kept(2)).is_none() && !s.has_body(&kept(2)));
    // Copied into another account's data file: whole.
    let file = s.data_file("a1").unwrap();
    drop(s);
    let other = Store::open_packed(t.join("app2"), t.join("Files2")).unwrap();
    other.save_accounts(&[account("b1", "ana@example.org")]).unwrap();
    other.import_data_file("b1", &file).unwrap();
    assert_eq!(other.read_body(&MessageKey { account: "b1".into(), ..kept(1) }).unwrap(), big);
    // The pieces fill their pages: many messages of awkward sizes leave little unused (a whole
    // value each left 40% of the file unused).
    let t2 = temp("pieces-waste");
    let s = Store::open_packed(t2.join("app"), t2.join("Files")).unwrap();
    s.save_accounts(&[account("a1", "zoe@studio.example")]).unwrap();
    let bodies: Vec<(MessageKey, Vec<u8>)> =
        (0..120u32).map(|i| (key(i), noise(70_000 + (i as usize * 2_731) % 200_000, u64::from(i) + 9))).collect();
    let stored: usize = bodies.iter().map(|(_, b)| b.len()).sum();
    s.write_bodies(&bodies).unwrap();
    let file2 = s.data_file("a1").unwrap();
    drop(s);
    let pages = |path: &std::path::Path| {
        let db = redb::Database::open(path).unwrap();
        let st = db.begin_write().unwrap().stats().unwrap();
        st.allocated_pages() * st.page_size() as u64
    };
    let pieced = pages(&file2);
    // The same bodies as whole values, for comparison.
    let whole_file = t2.join("whole.redb");
    {
        let db = redb::Database::create(&whole_file).unwrap();
        let tx = db.begin_write().unwrap();
        {
            let mut table = tx.open_table(super::FILES).unwrap();
            for (k, b) in &bodies {
                table.insert(format!("m{}", k.uid).as_str(), b.as_slice()).unwrap();
            }
        }
        tx.commit().unwrap();
    }
    let whole = pages(&whole_file);
    assert!(pieced < stored as u64 * 112 / 100, "pieces took {pieced} for {stored}");
    assert!(pieced * 100 < whole * 85, "pieces {pieced}, whole values {whole}");
    let _ = std::fs::remove_dir_all(t2);
    let sizes = super::piece_sizes(big.len());
    assert_eq!(sizes.iter().sum::<usize>(), big.len());
    assert!(sizes.iter().all(|n| *n <= 16 * super::PAGE - super::PAGE_SLACK));
    assert!(super::piece_sizes(0).is_empty() && super::piece_sizes(1) == [1]);
    let _ = std::fs::remove_dir_all(t);
}

/// Bytes that don't compress (xorshift).
fn noise(n: usize, seed: u64) -> Vec<u8> {
    let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as u8
        })
        .collect()
}

/// Text (mail, folder lists) is kept compressed and reads back as written; what doesn't
/// compress is kept as it is; files from before compression still read; and a crafted
/// compressed value that would come to more than the cap is refused, not inflated.
#[test]
fn text_is_kept_compressed() {
    let t = temp("deflate");
    let s = Store::open_packed(t.join("app"), t.join("Files")).unwrap();
    s.save_accounts(&[account("a1", "zoe@studio.example")]).unwrap();
    let key = |uid: u32| MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid };
    let text: Vec<u8> = "Subject: report

"
    .bytes()
    .chain("The figures for October are in the attached sheet. ".repeat(4000).bytes())
    .collect();
    s.write_body(&key(1), &text).unwrap();
    assert_eq!(s.read_body(&key(1)).unwrap(), text);
    let dir = s.body_path(&key(1)).parent().unwrap().to_path_buf();
    assert!(s.files.size(&dir) * 20 < text.len() as u64, "kept in {} bytes", s.files.size(&dir));
    let random = noise(5000, 3);
    assert_eq!(super::pack(&random).as_ref(), random.as_slice(), "not compressed when it doesn't help");
    assert_eq!(super::pack(b"short").as_ref(), b"short");
    assert_eq!(super::unpack(b"plain old bytes".to_vec()).unwrap(), b"plain old bytes", "from before compression");
    // A crafted value: a deflate stream of more than the cap.
    let bomb = {
        use std::io::Write;
        let mut z = flate2::write::DeflateEncoder::new(super::DEFLATED.to_vec(), flate2::Compression::best());
        let zeros = vec![0u8; 1 << 20];
        for _ in 0..(super::INFLATE_MAX >> 20) + 2 {
            z.write_all(&zeros).unwrap();
        }
        z.finish().unwrap()
    };
    assert!(super::unpack(bomb).is_err());
    let _ = std::fs::remove_dir_all(t);
}

#[test]
fn folders_from_before_come_into_the_data_file() {
    let t = temp("bring-in");
    let app = t.join("app");
    let old = Store::open(&app).unwrap();
    old.save_accounts(&[account("a1", "zoe@studio.example")]).unwrap();
    let k = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 3, uid: 1 };
    old.write_body(&k, b"Subject: kept\r\n\r\nbody").unwrap();
    old.save_account_doc("a1", "pop-seen.json", &vec!["u1".to_string()]).unwrap();
    assert!(app.join("a1").is_dir());
    let s = Store::open_packed(&app, t.join("Files")).unwrap();
    assert!(!app.join("a1").exists(), "the folder went once it was in");
    assert_eq!(s.read_body(&k).unwrap(), b"Subject: kept\r\n\r\nbody");
    assert_eq!(s.load_account_doc::<Vec<String>>("a1", "pop-seen.json").unwrap(), vec!["u1".to_string()]);
    assert!(t.join("Files").join("zoe@studio.example.mailcraft").is_file());
    let _ = std::fs::remove_dir_all(t);
}

#[test]
fn a_damaged_data_file_is_set_aside() {
    let t = temp("damaged");
    let files = t.join("Files");
    std::fs::create_dir_all(&files).unwrap();
    std::fs::write(files.join("zoe@studio.example.mailcraft"), b"not a database at all, just junk bytes").unwrap();
    std::fs::create_dir_all(t.join("app")).unwrap();
    std::fs::write(t.join("app").join("datafiles.json"), br#"{"a1": "zoe@studio.example.mailcraft"}"#).unwrap();
    let s = Store::open_packed(t.join("app"), &files).unwrap();
    let k = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid: 1 };
    assert!(s.read_body(&k).is_none());
    s.write_body(&k, b"x").unwrap();
    assert!(files.join("zoe@studio.example.mailcraft.damaged").exists());
    let _ = std::fs::remove_dir_all(t);
}

/// Whether any file under `dir` holds `needle`.
fn on_disk(dir: &std::path::Path, needle: &[u8]) -> bool {
    let Ok(rd) = std::fs::read_dir(dir) else { return false };
    rd.flatten().any(|e| {
        let p = e.path();
        if p.is_dir() { on_disk(&p, needle) } else { std::fs::read(&p).is_ok_and(|b| b.windows(needle.len()).any(|w| w == needle)) }
    })
}

/// Issue #4: an account kept online only writes none of its mail anywhere. What it had on disk
/// comes into memory and its data file goes; it works as before while MailCraft runs (lists,
/// sources, changes waiting, the Outbox), and the next start finds nothing of it. Other
/// accounts keep theirs; turned off again, what's held is written out.
#[test]
fn an_online_only_account_leaves_nothing_on_disk() {
    use mailcraft_model::{Folder, FolderRole};
    let t = temp("online-only");
    let (app, files) = (t.join("app"), t.join("MailCraft Files"));
    let s = Store::open_packed(&app, &files).unwrap();
    s.save_accounts(&[account("a1", "zoe@studio.example"), account("a2", "ana@studio.example")]).unwrap();
    let k = |a: &str, uid| MessageKey { account: a.into(), folder: "INBOX".into(), validity: 1, uid };
    let fill = |s: &Store, a: &str, uid, text: &str| {
        s.save_folders(a, &[Folder { path: "INBOX".into(), role: FolderRole::Inbox, ..Default::default() }]).unwrap();
        let mut st = s.load_folder(a, "INBOX").unwrap();
        st.validity = 1;
        st.messages.push(Summary { key: k(a, uid), subject: format!("{text} subject"), ..Default::default() });
        s.save_folder(a, "INBOX", &st).unwrap();
        s.write_body(&k(a, uid), format!("Subject: {text}\r\n\r\n{text} body").as_bytes()).unwrap();
    };
    fill(&s, "a1", 1, "EARLIER-ZOE");
    fill(&s, "a2", 1, "ANA-KEPT");
    let zoe_file = s.data_file("a1").unwrap();
    // (A data file can be read only once it's closed.)
    drop(s);
    assert!(zoe_file.exists() && on_disk(&t, b"EARLIER-ZOE"), "the search finds what's kept");
    let s = Store::open_packed(&app, &files).unwrap();

    s.set_online_only("a1", true).unwrap();
    assert!(s.is_online_only("a1") && !s.is_online_only("a2"));
    assert!(!zoe_file.exists(), "its data file goes");
    assert_eq!(s.data_file("a1"), None);
    // What it had is still there while MailCraft runs.
    assert_eq!(s.load_folder("a1", "INBOX").unwrap().messages.len(), 1);
    assert_eq!(s.read_body(&k("a1", 1)).unwrap(), b"Subject: EARLIER-ZOE\r\n\r\nEARLIER-ZOE body");
    // New mail, changes, the Outbox and documents: in memory only.
    fill(&s, "a1", 2, "SECRET-ZOE");
    s.write_bodies(&[(k("a1", 3), b"Subject: x\r\n\r\nSECRET-BATCH".to_vec()), (k("a2", 3), b"Subject: x\r\n\r\nANA-BATCH".to_vec())]).unwrap();
    s.save_ops("a1", &[mailcraft_model::Op::Expunge { keys: vec![k("a1", 1)] }]).unwrap();
    s.outbox_add("a1", &crate::OutboxItem { id: 4, ..Default::default() }, b"Subject: later\r\n\r\nSECRET-OUTBOX").unwrap();
    s.save_account_doc("a1", "pim.json", &serde_json::json!({"notes": ["SECRET-NOTE"]})).unwrap();
    assert_eq!(s.load_folder("a1", "INBOX").unwrap().messages.len(), 2);
    assert!(s.has_body(&k("a1", 2)) && s.read_body(&k("a1", 3)).is_some());
    assert_eq!(s.load_ops("a1").unwrap().len(), 1);
    assert_eq!(s.outbox("a1").len(), 1);
    assert!(s.account_size("a1") > 0);

    // MailCraft closes: nothing of it on disk, and the next start finds nothing of it.
    drop(s);
    for secret in [&b"EARLIER-ZOE"[..], b"SECRET-ZOE", b"SECRET-BATCH", b"SECRET-OUTBOX", b"SECRET-NOTE"] {
        assert!(!on_disk(&t, secret), "{}", String::from_utf8_lossy(secret));
    }
    assert!(on_disk(&t, b"ANA-BATCH") && on_disk(&t, b"ANA-KEPT"), "the other account keeps its mail on disk");
    assert!(!std::fs::read_to_string(app.join(super::NAMES)).unwrap().contains("a1"), "no data file is named for it");
    let s = Store::open_packed(&app, &files).unwrap();
    s.set_online_only("a1", true).unwrap();
    assert!(s.load_folder("a1", "INBOX").unwrap().messages.is_empty() && s.load_ops("a1").unwrap().is_empty());
    assert_eq!(s.read_body(&k("a2", 1)).unwrap(), b"Subject: ANA-KEPT\r\n\r\nANA-KEPT body");

    // Turned off: what's held is written out to a data file again.
    fill(&s, "a1", 5, "BACK-ON-DISK");
    s.set_online_only("a1", false).unwrap();
    assert!(!s.is_online_only("a1"));
    assert!(s.data_file("a1").is_some_and(|f| f.exists()));
    assert_eq!(s.read_body(&k("a1", 5)).unwrap(), b"Subject: BACK-ON-DISK\r\n\r\nBACK-ON-DISK body");
    drop(s);
    let s = Store::open_packed(&app, &files).unwrap();
    assert_eq!(s.load_folder("a1", "INBOX").unwrap().messages.len(), 1, "kept across a start");

    // Removing an online-only account: nothing to keep, nothing left.
    s.set_online_only("a1", true).unwrap();
    assert_eq!(s.remove_account("a1").unwrap(), None);
    assert!(!s.is_online_only("a1") && s.load_folder("a1", "INBOX").unwrap().messages.is_empty());
    drop(s);
    assert!(!on_disk(&t, b"BACK-ON-DISK"));
    let _ = std::fs::remove_dir_all(t);
}
