//! A note's files (from an Outlook data file): listed with the note, saved where asked, never
//! handed to another app when they could run a program, and kept as they are when the note is
//! written on.

use super::*;

/// A note with two files, one of them a program, in the sample's account.
fn with_files(s: &mut Session) -> String {
    let uid = s.run("note.create", &json!({"text": "Paper"})).unwrap()["uid"].as_str().unwrap().to_string();
    let account = s.pim.keys().find(|a| s.pim[*a].notes.iter().any(|n| n.note.uid == uid)).unwrap().clone();
    let note = s.pim.get_mut(&account).unwrap().notes.iter_mut().find(|n| n.note.uid == uid).unwrap();
    note.note.files = vec![
        "ATTACH;FMTTYPE=application/pdf;ENCODING=BASE64;VALUE=BINARY;X-FILENAME=\"swatch.pdf\":JVBERi0xLjc=".into(),
        "ATTACH;FMTTYPE=application/octet-stream;ENCODING=BASE64;VALUE=BINARY;X-FILENAME=\"tool.exe\":TVo=".into(),
        "X-SOMETHING:else".into(),
    ];
    uid
}

#[test]
fn a_note_s_files_are_listed_and_saved() {
    let mut s = sample_session("note-files");
    let uid = with_files(&mut s);
    let listed = s.run("note.list", &json!({"query": "Paper"})).unwrap();
    let note = listed["notes"].as_array().unwrap().iter().find(|n| n["uid"] == uid.as_str()).unwrap();
    assert_eq!(
        note["files"],
        json!([{"name": "swatch.pdf", "type": "application/pdf", "size": 8}, {"name": "tool.exe", "type": "application/octet-stream", "size": 2}])
    );
    let root = s.store.as_ref().unwrap().root().to_path_buf();
    let out = root.join("swatch.pdf");
    s.run("note.saveFile", &json!({"uid": uid, "index": 0, "path": out.to_string_lossy()})).unwrap();
    assert_eq!(std::fs::read(&out).unwrap(), b"%PDF-1.7");
    assert_eq!(s.note_file(&uid, 1).unwrap(), ("tool.exe".to_string(), b"MZ".to_vec()));
    // To be opened by another app: never a program.
    let exe = root.join("tool.exe");
    let refused = s.run("note.saveFile", &json!({"uid": uid, "index": 1, "path": exe.to_string_lossy(), "open": true}));
    assert!(matches!(refused, Err(CmdError::Disabled(_))), "{refused:?}");
    assert!(!exe.exists());
    // Saved to keep, it is written.
    s.run("note.saveFile", &json!({"uid": uid, "index": 1, "path": exe.to_string_lossy()})).unwrap();
    assert!(exe.exists());
    for bad in [json!({"uid": uid, "index": 2, "path": "x"}), json!({"uid": "nope", "index": 0, "path": "x"}), json!({"uid": uid, "path": "x"})] {
        assert!(s.run("note.saveFile", &bad).is_err(), "{bad}");
    }
    // Written on, the note keeps its files.
    s.run("note.update", &json!({"uid": uid, "text": "Paper\n170 gsm", "color": 1})).unwrap();
    let n = s.notes("").into_iter().find(|n| n.note.uid == uid).unwrap();
    assert_eq!(n.note.kept_files().len(), 2);
    done(s);
}
