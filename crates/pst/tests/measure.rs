//! Timings of the writer, run by hand in release:
//! `cargo test --release -p mailcraft-pst --test measure -- --ignored --nocapture`.

use std::time::Instant;

use mailcraft_pst::Format;
use mailcraft_pst::write::{self, SpecialFolder};

fn temp(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("mailcraft-pst-measure-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    let _ = std::fs::remove_file(&path);
    path
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
