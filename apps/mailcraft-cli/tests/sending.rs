//! `mailcraft-cli send` sends only from an account that can: with none, or with only an Outlook
//! data file, it fails and says why (it printed `{"sent": …, "local": true}` for a data file,
//! and nothing had gone).

use std::process::{Command, Output};

fn cli(data: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_mailcraft-cli")).arg("--data").arg(data).args(args).env_remove("MAILCRAFT_DATA").output().unwrap()
}

#[test]
fn send_fails_with_no_account_that_can_send() {
    let data = std::env::temp_dir().join(format!("mailcraft-cli-send-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&data);
    std::fs::create_dir_all(&data).unwrap();
    let send = ["send", "--to", "ann@example.org", "--subject", "Plans", "--body", "See you"];
    let out = cli(&data, &send);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("no accounts yet"), "{}", String::from_utf8_lossy(&out.stderr));
    // A data file opened (its import runs in the app; the account is there at once).
    let out = cli(&data, &["run", "account.importPst", r#"{"path": "Archive.pst"}"#]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let out = cli(&data, &send);
    let (stdout, stderr) = (String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(!out.status.success(), "{stdout}");
    assert!(stderr.contains("data file") && !stdout.contains("sent"), "{stdout} / {stderr}");
    let _ = std::fs::remove_dir_all(&data);
}
