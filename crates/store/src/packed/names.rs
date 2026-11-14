//! Which data file is each account's (`datafiles.json` in the data folder's root): read as the
//! data files open, written whole after each change. A list that can't be read stops the opening
//! (taken for none, every account's file was forgotten and a second one made beside it); a
//! damaged one is set aside, and each account's file found again by the account it says it is,
//! or where [`stem`] would name it ([`found_again`]).

use super::*;

/// The accounts the data folder lists, by their folder's name ([`crate::safe_segment`]), with
/// their addresses; none when it lists none or can't be read.
fn accounts(root: &Path) -> Vec<(String, String)> {
    fs::read(root.join("accounts.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<Vec<mailcraft_model::AccountConfig>>(&b).ok())
        .unwrap_or_default()
        .into_iter()
        .map(|a| (crate::safe_segment(&a.id), a.email))
        .collect()
}

/// What an account's data file is called, before its extension and any number: its address, as
/// Outlook names a `.pst` (an opened data file's own name), else the account's folder name.
pub(super) fn stem(root: &Path, account: &str) -> String {
    let email = accounts(root).into_iter().find(|(a, _)| a == account).map(|(_, e)| e);
    stem_of(email.as_deref(), account)
}

fn stem_of(email: Option<&str>, account: &str) -> String {
    let stem = email.map(str::trim).filter(|e| !e.is_empty()).unwrap_or(account);
    let stem = stem.strip_suffix(".pst").or_else(|| stem.strip_suffix(".ost")).or_else(|| stem.strip_suffix(".mailcraft")).unwrap_or(stem);
    let stem: String = stem
        .chars()
        .map(|c| if c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') { '_' } else { c })
        .take(80)
        .collect();
    let stem = stem.trim_matches(|c: char| c == '.' || c.is_whitespace());
    if stem.is_empty() { account.to_string() } else { stem.to_string() }
}

/// The list as kept: none there is an empty one; one that can't be read is an error; a damaged
/// one is set aside (`.bad`) and made again from the data files there ([`found_again`]).
pub(super) fn read(root: &Path, dir: &Path) -> Result<BTreeMap<String, String>> {
    let path = root.join(NAMES);
    let bytes = match fs::read(&path) {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(e) => return Err(io_err(&path)(e)),
    };
    if let Ok(names) = serde_json::from_slice(&bytes) {
        return Ok(names);
    }
    let _ = fs::rename(&path, path.with_extension("json.bad"));
    let names = found_again(root, dir);
    log::warn!("{}: damaged, set aside; {} accounts' data files found again", path.display(), names.len());
    save(root, &names);
    Ok(names)
}

/// The most data files looked into for the account each says it is.
const FILES_MAX: usize = 4096;

/// Each account's data file in `dir`, found without the list. First by what the files say: a
/// file made by this data folder names its account (`owner.rs`). Then, for an account no file
/// names, by the name its file would have been given, when that file names no account and no
/// other account's would have been given the same name: of two accounts with one address the
/// first was given `<address>` and the second `<address> (2)`, and nothing but the list said
/// which was which, so neither is taken for the other's (each account read the other's mail,
/// by the order the accounts are listed in). An account left without one starts a new file;
/// the old ones stay where they are.
fn found_again(root: &Path, dir: &Path) -> BTreeMap<String, String> {
    let accounts = accounts(root);
    let me = owner::read_folder_id(root).ok().flatten();
    // What each data file in the folder says, by its name.
    let mut says: BTreeMap<String, Option<String>> = BTreeMap::new();
    for e in fs::read_dir(dir).into_iter().flatten().flatten().take(FILES_MAX) {
        let path = e.path();
        let is_one = path.extension().is_some_and(|x| x.eq_ignore_ascii_case(EXTENSION)) && e.file_type().is_ok_and(|t| t.is_file());
        if let (true, Some(name)) = (is_one, e.file_name().to_str()) {
            says.insert(name.to_string(), owner::account_named(&path, root, me.as_deref()));
        }
    }
    let mut names = BTreeMap::new();
    for (account, _) in &accounts {
        let mut theirs = says.iter().filter(|(_, a)| a.as_deref() == Some(account.as_str())).map(|(n, _)| n);
        // Two that say so (a copy of the file put beside it): neither is known to be the one.
        if let (Some(file), None) = (theirs.next(), theirs.next()) {
            names.insert(account.clone(), file.clone());
        }
    }
    let named = |email: &str, account: &str| format!("{}.{EXTENSION}", stem_of(Some(email), account));
    for (account, email) in &accounts {
        // One a file names has it, or (named by two) isn't given another's.
        if says.values().any(|a| a.as_deref() == Some(account.as_str())) {
            continue;
        }
        let file = named(email, account);
        if accounts.iter().any(|(a, e)| a != account && named(e, a).eq_ignore_ascii_case(&file)) {
            log::warn!("{account}: its data file isn't told from another account's of the same name; not guessed");
            continue;
        }
        let unsaid = says.iter().any(|(n, a)| n.eq_ignore_ascii_case(&file) && a.is_none());
        let taken = names.values().any(|n: &String| n.eq_ignore_ascii_case(&file));
        if unsaid && !taken {
            names.insert(account.clone(), file);
        }
    }
    names
}

/// Write the list whole; a failure is logged (the next change writes it whole again).
pub(super) fn save(root: &Path, names: &BTreeMap<String, String>) {
    let path = root.join(NAMES);
    let written = serde_json::to_vec_pretty(names).map_err(|e| e.to_string()).and_then(|json| write_atomic(&path, &json).map_err(|e| e.to_string()));
    if let Err(e) = written {
        log::warn!("{}: not saved: {e}", path.display());
    }
}
