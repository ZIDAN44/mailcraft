pub(crate) fn is_message_or_pim_file(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(std::ffi::OsStr::to_str)
        .is_some_and(|extension| matches!(extension.to_ascii_lowercase().as_str(), "eml" | "msg" | "ics" | "ical" | "vcf" | "vcard"))
}

pub(crate) fn is_outlook_message_file(path: &std::path::Path) -> bool {
    path.extension().and_then(std::ffi::OsStr::to_str).is_some_and(|extension| extension.eq_ignore_ascii_case("msg"))
}

pub(crate) fn is_outlook_data_file(path: &std::path::Path) -> bool {
    path.extension().and_then(std::ffi::OsStr::to_str).is_some_and(|extension| matches!(extension.to_ascii_lowercase().as_str(), "pst" | "mailcraft"))
}
