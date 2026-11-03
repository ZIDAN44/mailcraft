//! An Outlook data file whose copy beside it is an account in another MailCraft data folder (the
//! app's, closed): the CLI, on a data folder of its own, is refused it with an error naming that
//! folder, and opens it when told to take it over.

use std::path::Path;
use std::process::Command;

use mailcraft_engine::store::Store;

fn cli(data: &Path, json: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_mailcraft-cli")).arg("--data").arg(data).args(["run", "account.importPst", json]).output().unwrap()
}

#[test]
fn a_copy_another_data_folder_has_is_refused_until_taken_over() {
    let dir = std::env::temp_dir().join(format!("mailcraft-cli-beside-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("Documents")).unwrap();
    let pst = dir.join("Documents").join("Archive.pst");
    std::fs::write(&pst, b"an Outlook file, as far as this test goes").unwrap();
    // The app's data folder opened it (its copy beside it, imported whole), and closed.
    let app = dir.join("app");
    {
        let store = Store::open_packed(&app, dir.join("app files")).unwrap();
        store.place_beside("a1", &pst).unwrap();
        let inbox = mailcraft_engine::model::Folder {
            account: "a1".into(),
            path: "INBOX".into(),
            role: mailcraft_engine::model::FolderRole::Inbox,
            selectable: true,
            ..Default::default()
        };
        store.save_folders("a1", &[inbox]).unwrap();
        let (size, modified) = mailcraft_engine::store::file_stamp(&pst).unwrap();
        let note = serde_json::json!({"state": "done", "source": {"path": pst.to_string_lossy(), "size": size, "modified": modified}});
        store.save_account_doc("a1", "import.json", &note).unwrap();
    }
    let path = serde_json::json!({"path": pst.to_string_lossy()}).to_string();
    let refused = cli(&dir.join("cli"), &path);
    let said = String::from_utf8_lossy(&refused.stderr);
    assert!(!refused.status.success(), "{said}");
    assert!(said.contains("another MailCraft data folder") && said.contains(app.to_string_lossy().as_ref()), "{said}");
    let taken = cli(&dir.join("cli"), &serde_json::json!({"path": pst.to_string_lossy(), "takeOver": true}).to_string());
    let out = String::from_utf8_lossy(&taken.stdout);
    assert!(taken.status.success() && out.contains("\"reopened\": true"), "{out} {}", String::from_utf8_lossy(&taken.stderr));
    let _ = std::fs::remove_dir_all(&dir);
}
