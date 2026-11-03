//! Where each folder goes in the data file: by its name, as Outlook shows names, or, for the
//! account's Sent, Trash, Drafts and Junk folders (and the folders inside them), in the file's
//! own special folders.

use super::*;
use outlook_pst::UnicodePstSpecialFolder as Special;

/// The roles that have a special folder in a data file. MailCraft has no Outbox folder.
pub(super) const SPECIAL: [(FolderRole, Special); 4] = [
    (FolderRole::Sent, Special::SentItems),
    (FolderRole::Trash, Special::DeletedItems),
    (FolderRole::Drafts, Special::Drafts),
    (FolderRole::Junk, Special::JunkEmail),
];

/// The special folder for `role`, when it has one.
pub(super) fn special(role: FolderRole) -> Option<Special> {
    SPECIAL.iter().find(|(r, _)| *r == role).map(|(_, s)| *s)
}

/// A folder's path as names, the way Outlook shows them.
pub(super) fn names(path: &str, delimiter: Option<char>, role: FolderRole) -> Vec<String> {
    let parts: Vec<&str> = match delimiter {
        Some(d) => path.split(d).collect(),
        None => vec![path],
    };
    let mut out: Vec<String> = parts.iter().map(|p| utf7::decode(p).trim().to_string()).collect();
    if let Some(first) = out.first_mut()
        && (role == FolderRole::Inbox || first.eq_ignore_ascii_case("INBOX"))
    {
        *first = "Inbox".into();
    }
    for n in &mut out {
        if n.is_empty() {
            *n = "Folder".into();
        }
    }
    out
}

/// The folders the file has for each special role: the ones an existing file records, else
/// the names a new file gets.
pub(super) struct Targets(Vec<(Special, Vec<String>)>);

impl Targets {
    /// The file at `path`'s (none when there's no file yet). The file is one being added to, so
    /// anything wrong in it, a panic in the writer's reader too, costs only its record: that
    /// folder gets the name a new file gives it.
    pub(super) fn of(path: Option<&Path>, format: Format) -> Targets {
        Targets::found(|s| match path {
            Some(p) => format.special_folder(p, s),
            None => Ok(None),
        })
    }

    /// The ones `lookup` finds.
    fn found(lookup: impl Fn(Special) -> io::Result<Option<Vec<String>>> + std::panic::RefUnwindSafe) -> Targets {
        let found = |s: Special| std::panic::catch_unwind(|| lookup(s)).ok()?.ok().flatten();
        Targets(SPECIAL.iter().map(|(_, s)| (*s, found(*s).unwrap_or_else(|| vec![s.default_name().to_string()]))).collect())
    }

    pub(super) fn get(&self, s: Special) -> Vec<String> {
        self.0.iter().find(|(t, _)| *t == s).map(|(_, names)| names.clone()).unwrap_or_else(|| vec![s.default_name().to_string()])
    }
}

/// Where `f` (one of `folders`) goes: its special folder when its role has one, inside it when
/// a folder above it has such a role, else its own names.
pub(super) fn place(f: &Folder, folders: &[Folder], targets: &Targets) -> Vec<String> {
    let own = names(&f.path, f.delimiter, f.role);
    if let Some(s) = special(f.role) {
        return targets.get(s);
    }
    let above = folders
        .iter()
        .filter_map(|a| Some((a, special(a.role)?)))
        .find(|(a, _)| a.path != f.path && a.delimiter.is_some_and(|d| f.path.strip_prefix(a.path.as_str()).is_some_and(|rest| rest.starts_with(d))));
    match above {
        Some((a, s)) => {
            let depth = names(&a.path, a.delimiter, a.role).len();
            let mut out = targets.get(s);
            out.extend(own.into_iter().skip(depth));
            out
        }
        None => own,
    }
}

/// Folder names as an ANSI file keeps them (Windows-1252, a `?` for what it lacks), so folders
/// whose names it would write alike stay apart: `placed[i]` is where `folders[i]` goes, and each
/// name of it that is written as another folder's beside it gets a number, "Отчёты (2)", as a
/// new file's folder taken by the account's mail does. The writer finds a folder by its name as
/// written, so without one `Письма` and `Отчёты` would be one `??????` folder. The same folder gets
/// the same name each export: names are given in order of where the folders go, the file's
/// special folders first.
pub(super) fn apart(placed: &[Vec<String>], targets: &Targets) -> Vec<Vec<String>> {
    let mut names = Apart::default();
    for (_, special) in &targets.0 {
        names.give(special);
    }
    let mut order: Vec<usize> = (0..placed.len()).collect();
    order.sort_by(|&a, &b| placed.get(a).cmp(&placed.get(b)));
    let mut out = vec![Vec::new(); placed.len()];
    for i in order {
        if let (Some(slot), Some(path)) = (out.get_mut(i), placed.get(i)) {
            *slot = names.give(path);
        }
    }
    out
}

/// The names [`apart`] gave.
#[derive(Default)]
struct Apart {
    /// Who took each name as written, beside the folders above it as given: the name asked for.
    written: std::collections::HashMap<(Vec<Vec<u8>>, Vec<u8>), String>,
    /// The name each name asked for was given, beside the folders above it as given.
    given: std::collections::HashMap<(Vec<String>, String), String>,
}

impl Apart {
    fn give(&mut self, path: &[String]) -> Vec<String> {
        let mut out: Vec<String> = Vec::with_capacity(path.len());
        for name in path {
            let key = (out.clone(), name.clone());
            let named = match self.given.get(&key) {
                Some(named) => named.clone(),
                None => {
                    let above: Vec<Vec<u8>> = out.iter().map(|n| written(n)).collect();
                    let free = |n: &str| self.written.get(&(above.clone(), written(n))).is_none_or(|by| by.eq_ignore_ascii_case(name));
                    let named = std::iter::once(name.clone())
                        .chain((2..=super::pim::MAX_SUFFIX).map(|n| format!("{name} ({n})")))
                        .find(|n| free(n))
                        .unwrap_or_else(|| name.clone());
                    self.written.entry((above, written(&named))).or_insert_with(|| name.clone());
                    self.given.insert(key, named.clone());
                    named
                }
            };
            out.push(named);
        }
        out
    }
}

/// `name` as an ANSI file's folder name is written, as the writer compares one (ASCII letters in
/// lower case): Windows-1252, a `?` for a character it hasn't, a character from U+0080 to U+009F
/// (as the writer's reader reads a byte back) that byte.
fn written(name: &str) -> Vec<u8> {
    name.chars()
        .map(|c| match u8::try_from(u32::from(c)) {
            Ok(b) => b.to_ascii_lowercase(),
            Err(_) => (0x80..=0x9F).find(|&b: &u8| mime::decode_code_page(&[b], 1252).starts_with(c)).unwrap_or(b'?'),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_names() {
        assert_eq!(names("INBOX/Projects/&AMk-t&AOk-", Some('/'), FolderRole::Other), ["Inbox", "Projects", "Été"]);
        assert_eq!(names("Sent", None, FolderRole::Sent), ["Sent"]);
        assert_eq!(names("a//b", Some('/'), FolderRole::Other), ["a", "Folder", "b"]);
    }

    #[test]
    fn special_folders_and_folders_inside_them() {
        let folder = |path: &str, role| Folder { path: path.into(), delimiter: Some('/'), role, ..Default::default() };
        let folders = [
            folder("[Gmail]/Sent Mail", FolderRole::Sent),
            folder("[Gmail]/Sent Mail/2024", FolderRole::Other),
            folder("Trash", FolderRole::Trash),
            folder("Trashy", FolderRole::Other),
            folder("Projects", FolderRole::Other),
        ];
        let targets = Targets::of(None, Format::Unicode);
        let at = |i: usize| place(&folders[i], &folders, &targets);
        assert_eq!(at(0), ["Sent Items"]);
        assert_eq!(at(1), ["Sent Items", "2024"]);
        assert_eq!(at(2), ["Deleted Items"]);
        assert_eq!(at(3), ["Trashy"]);
        assert_eq!(at(4), ["Projects"]);
        // A file that records its own: those.
        let targets = Targets(vec![(Special::SentItems, vec!["Gesendet".into()])]);
        assert_eq!(place(&folders[1], &folders, &targets), ["Gesendet", "2024"]);
        assert_eq!(place(&folders[2], &folders, &targets), ["Deleted Items"]);
    }

    /// In an ANSI file, folders whose names it would write alike get numbers, the same each
    /// time; the file's special folders keep theirs, and a folder's own name, in any case of its
    /// ASCII letters, is the same folder.
    #[test]
    fn names_an_ansi_file_writes_alike_stay_apart() {
        let p = |s: &[&str]| s.iter().map(|n| n.to_string()).collect::<Vec<_>>();
        let targets = Targets(vec![(Special::SentItems, p(&["Отправлено"]))]);
        let placed = [
            p(&["Письма", "Итоги"]),
            p(&["Отчёты"]),
            p(&["Письма"]),
            p(&["Отчёты", "Итоги"]),
            p(&["Café"]),
            p(&["CAFÉ"]),
            p(&["Получатели"]),
            p(&["Отправлено", "2024"]),
            p(&["Receipts – 2024"]),
            p(&["Rapport"]),
            p(&["RAPPORT"]),
        ];
        let want = [
            p(&["Письма (2)", "Итоги"]),
            p(&["Отчёты"]),
            p(&["Письма (2)"]),
            p(&["Отчёты", "Итоги"]),
            p(&["Café"]),
            p(&["CAFÉ"]),
            p(&["Получатели (2)"]),
            p(&["Отправлено", "2024"]),
            p(&["Receipts – 2024"]),
            p(&["Rapport"]),
            p(&["RAPPORT"]),
        ];
        assert_eq!(apart(&placed, &targets), want);
        let mut reversed = placed.clone();
        reversed.reverse();
        let mut back = apart(&reversed, &targets);
        back.reverse();
        assert_eq!(back, want);
        assert_eq!(written("Receipts – 2024"), b"receipts \x96 2024");
        assert_eq!(written("Wysłane"), b"wys?ane");
    }

    /// A lookup in a damaged file that fails, or panics in the writer's reader, costs only its
    /// record: that folder gets the name a new file gives it, and the export goes on.
    #[test]
    fn a_damaged_record_gives_the_default_name() {
        let targets = Targets::found(|s| match s {
            Special::SentItems => Ok(Some(vec!["Gesendet".into()])),
            Special::DeletedItems => Err(io::Error::other("damaged")),
            _ => panic!("damaged"),
        });
        assert_eq!(targets.get(Special::SentItems), ["Gesendet"]);
        assert_eq!(targets.get(Special::DeletedItems), ["Deleted Items"]);
        assert_eq!(targets.get(Special::Drafts), ["Drafts"]);
        assert_eq!(targets.get(Special::JunkEmail), ["Junk Email"]);
    }
}
