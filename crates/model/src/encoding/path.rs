//! Paths as text: a file's name at the end of one, whichever system wrote it.

/// The last part of a path, after its last `/` or `\` (the whole of it when it has neither): a
/// file's name in `C:\Mail\Archive.pst` or `/home/zoe/notes.eml`, a picture's in
/// `http://studio.example/a/chart.png`; empty after a trailing separator.
pub fn file_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_last_part_of_a_path() {
        assert_eq!(file_name(r"C:\Mail\Archive.pst"), "Archive.pst");
        assert_eq!(file_name("/home/zoe/notes.eml"), "notes.eml");
        assert_eq!(file_name("http://studio.example/a/chart.png"), "chart.png");
        assert_eq!(file_name(r"C:\Mail/mixed\name.msg"), "name.msg");
        assert_eq!(file_name("plain.txt"), "plain.txt");
        assert_eq!(file_name("folder/"), "");
        assert_eq!(file_name(""), "");
    }

    proptest::proptest! {
        /// What follows the last separator, by another way of finding it.
        #[test]
        fn any_path(path in r"[a-z/\\:. é]{0,24}") {
            let after = path.rfind(['/', '\\']).map_or(path.as_str(), |at| path.get(at + 1..).unwrap_or_default());
            proptest::prop_assert_eq!(file_name(&path), after);
        }
    }
}
