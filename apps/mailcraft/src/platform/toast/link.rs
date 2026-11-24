//! The `mailcraft:` link a notification carries (issue #273): clicked, in the notification
//! centre too, Windows starts the program registered for the link, which hands it to the
//! MailCraft running on its data folder. A toast's own click event reaches MailCraft only while
//! the toast shows; a program has to be started for one clicked later.
//!
//! The installer registers the link for everyone. Windows' launcher, which starts a toast's link,
//! takes those; one a program registers for its user alone it mostly doesn't (it asks for an app
//! in the Store instead), so MailCraft registers none itself.

use std::path::{Path, PathBuf};

/// The program the link starts, as Windows reads it.
const COMMAND: &str = r"mailcraft\shell\open\command";

/// Whether a `mailcraft:` link reaches the MailCraft running on the data folder `root`: the
/// program registered for it is there, and the folder it starts on (none named: the usual one) is
/// this one. A portable copy's isn't (it writes nothing to the registry), nor one on a folder
/// chosen at start, unless the link was registered to start it so.
pub fn link_reaches(root: &Path) -> bool {
    let Ok(command) = windows_registry::CLASSES_ROOT.open(COMMAND).and_then(|k| k.get_string("")) else { return false };
    let Some((exe, data)) = command_parts(&command) else { return false };
    if !Path::new(&exe).is_file() {
        return false;
    }
    let started_on = data.map(PathBuf::from).or_else(mailcraft_engine::store::default_root);
    started_on.is_some_and(|d| same_folder(&d, root))
}

/// Whether Windows' launcher, which starts a toast's link, takes `mailcraft:` links: the
/// installer's it does. Without them (MailCraft installed before they were registered), a link
/// clicked would only show "You'll need a new app": the toast's own click event reaches MailCraft
/// instead, while the toast shows. Asked of the launcher itself.
pub fn launcher_takes_links() -> bool {
    use windows::System::{LaunchQuerySupportStatus, LaunchQuerySupportType, Launcher};
    let Ok(uri) = windows::Foundation::Uri::CreateUri(&windows::core::HSTRING::from("mailcraft:show")) else { return false };
    Launcher::QueryUriSupportAsync(&uri, LaunchQuerySupportType::Uri)
        .and_then(|asked| asked.join())
        .is_ok_and(|s| s == LaunchQuerySupportStatus::Available)
}

/// The program a registered command starts and the data folder it names (`--data`), when it
/// hands the link over with `--open` (`"C:\…\mailcraft.exe" --open "%1"`).
fn command_parts(command: &str) -> Option<(String, Option<String>)> {
    let words = split_command(command);
    let exe = words.first()?.clone();
    if !words.iter().any(|w| w == "--open") || !words.iter().any(|w| w == "%1") {
        return None;
    }
    let data = words.iter().position(|w| w == "--data").and_then(|i| words.get(i + 1)).cloned();
    Some((exe, data))
}

/// A command line's words, as Windows splits a program's: by spaces, a quoted run one word.
fn split_command(command: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let (mut quoted, mut started) = (false, false);
    for c in command.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                started = true;
            }
            ' ' | '\t' if !quoted => {
                if started {
                    words.push(std::mem::take(&mut word));
                    started = false;
                }
            }
            c => {
                word.push(c);
                started = true;
            }
        }
    }
    if started {
        words.push(word);
    }
    words
}

/// Whether two paths name one folder (Windows' paths don't tell letter case apart).
fn same_folder(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => {
            let plain = |p: &Path| p.to_string_lossy().trim_end_matches(['\\', '/']).to_lowercase();
            plain(a) == plain(b)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_registered_command_names_its_program_and_folder() {
        assert_eq!(
            command_parts(r#""C:\Program Files\MailCraft\mailcraft.exe" --open "%1""#),
            Some((r"C:\Program Files\MailCraft\mailcraft.exe".to_string(), None))
        );
        assert_eq!(
            command_parts(r#""D:\dev\mailcraft.exe" --data "D:\scratch dir\mc" --open "%1""#),
            Some((r"D:\dev\mailcraft.exe".to_string(), Some(r"D:\scratch dir\mc".to_string())))
        );
        // Another program's command, or one that doesn't hand the link over.
        assert_eq!(command_parts(r#""C:\x\other.exe" "%1""#), None);
        assert_eq!(command_parts(r#""C:\x\mailcraft.exe" --open"#), None);
        assert_eq!(command_parts(""), None);
        assert_eq!(split_command(r#"a  "b c"   "" d"#), ["a", "b c", "", "d"]);
    }

    #[test]
    fn a_folder_is_the_same_in_any_letter_case() {
        assert!(same_folder(Path::new(r"C:\Users\Zoe\AppData\Missing\MailCraft\"), Path::new(r"c:\users\zoe\appdata\missing\mailcraft")));
        assert!(!same_folder(Path::new(r"C:\a\MailCraft"), Path::new(r"C:\a\MailCraft2")));
    }
}
