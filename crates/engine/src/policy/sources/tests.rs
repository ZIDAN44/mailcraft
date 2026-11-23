//! Issue #245: the profile found on this computer.

use super::*;

#[test]
fn a_profile_file_is_json_and_no_more() {
    assert!(parse(b"{\"organisation\": \"x\"}").is_ok());
    assert!(parse(b"\xEF\xBB\xBF{}").is_ok(), "Notepad's byte-order mark");
    assert!(parse(b"[]").unwrap_err().contains("object"));
    assert!(parse(b"{").unwrap_err().contains("JSON"));
    assert!(parse(&vec![b' '; PROFILE_MAX + 1]).unwrap_err().contains("larger"));
}

#[test]
fn a_portable_copys_profile_gives_defaults_and_a_broken_one_is_said() {
    let dir = std::env::temp_dir().join(format!("mailcraft-policy-portable-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(FILE_NAME), r#"{"settings": {"compact": {"value": true, "locked": true}}, "requireEncryption": true}"#).unwrap();
    let o = read(Some(&dir));
    assert!(o.sources.iter().any(|s| s.contains(FILE_NAME)), "{:?}", o.sources);
    assert!(!o.top.settings["compact"].locked && o.top.require_encryption.is_none());
    std::fs::write(dir.join(FILE_NAME), "not json").unwrap();
    let o = read(Some(&dir));
    assert!(o.ignored.iter().any(|i| i.contains("not JSON")), "{:?}", o.ignored);
    let _ = std::fs::remove_dir_all(&dir);
}
