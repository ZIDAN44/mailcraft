pub(crate) fn picker_fallback() -> String {
    mailcraft_ui_egui::i18n::t("The Android file picker couldn't open. In Files, choose Share → MailCraft to use the file.").to_owned()
}

fn selected_file_too_large(max_bytes: u64) -> String {
    let size = (max_bytes >> 20).to_string();
    mailcraft_ui_egui::i18n::tfmt("The selected file is larger than {size} MiB", &[("size", &size)])
}

pub(crate) fn picker_file_limit(purpose: &str, name: &str, mime_type: &str) -> Result<u64, String> {
    let extension = std::path::Path::new(name).extension().and_then(std::ffi::OsStr::to_str).map(str::to_ascii_lowercase).unwrap_or_default();
    let mime = mime_type.split(';').next().unwrap_or_default().trim();
    let data_container = matches!(extension.as_str(), "pst" | "mailcraft") || mime.eq_ignore_ascii_case("application/vnd.storytold.mailcraft");
    let pim_file = matches!(extension.as_str(), "ics" | "ical" | "vcf" | "vcard")
        || matches!(
            mime.to_ascii_lowercase().as_str(),
            "text/calendar" | "text/vcalendar" | "text/x-vcalendar" | "application/ics" | "text/vcard" | "text/x-vcard"
        );
    if purpose == "account-data-file" && !data_container {
        return Err(mailcraft_ui_egui::i18n::t("Choose a .pst or .mailcraft data file for this account").to_owned());
    }
    Ok(if matches!(purpose, "data-file" | "account-data-file") {
        if data_container {
            mailcraft_engine::MAX_DATA_ARCHIVE_BYTES
        } else if pim_file {
            mailcraft_engine::MAX_PIM_FILE_BYTES
        } else {
            mailcraft_engine::MAX_MAIL_FILE_BYTES
        }
    } else {
        match purpose {
            "rules" => 4 << 20,
            "categories" => 1 << 20,
            "attach" | "picture" => mailcraft_engine::MAX_ATTACHMENT_BYTES as u64,
            purpose if purpose.starts_with("pgp-secret:") => 1 << 20,
            purpose if purpose.starts_with("pgp-public:") => 8 << 20,
            purpose if purpose.starts_with("certificate:") => 1 << 20,
            "signature-picture" => mailcraft_engine::mime::SIGNATURE_PICTURE_MAX as u64,
            "signature" => mailcraft_engine::mime::SIGNATURE_CODE_MAX as u64,
            _ => mailcraft_engine::MAX_MAIL_FILE_BYTES,
        }
    })
}

fn image_file_name(name: &str, bytes: &[u8]) -> String {
    let extension = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("png")
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("jpg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("gif")
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12).is_some_and(|kind| kind == b"WEBP") {
        Some("webp")
    } else if bytes.starts_with(b"BM") {
        Some("bmp")
    } else {
        None
    };
    let Some(extension) = extension else { return name.to_owned() };
    let name = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let stem = std::path::Path::new(name).file_stem().and_then(std::ffi::OsStr::to_str).filter(|stem| !stem.is_empty()).unwrap_or("picture");
    format!("{stem}.{extension}")
}

pub(crate) fn read_picked_file(
    activity: &winit::platform::android::activity::AndroidApp,
    directory: Option<&std::path::Path>,
    purpose: &str,
    pick_image: bool,
    uri: &str,
    picked_name: Option<&str>,
    picked_mime: Option<&str>,
    picked_size: Option<u64>,
) -> Result<(String, Vec<u8>), String> {
    const MAX_PICKED_NAME_CHARS: usize = 512;
    const MAX_PICKED_MIME_CHARS: usize = 256;
    const MAX_PICKED_URI_BYTES: usize = 8192;

    let name = picked_name.filter(|name| !name.is_empty()).unwrap_or("attachment").chars().take(MAX_PICKED_NAME_CHARS).collect::<String>();
    let mime_type = picked_mime.unwrap_or("application/octet-stream").chars().take(MAX_PICKED_MIME_CHARS).collect::<String>();
    let content_authority = uri.strip_prefix("content://").and_then(|remainder| remainder.split('/').next());
    if uri.len() > MAX_PICKED_URI_BYTES {
        return Err(mailcraft_ui_egui::i18n::t("The selected document address is too long").to_owned());
    }
    if content_authority.is_none_or(str::is_empty) || uri.chars().any(char::is_control) {
        return Err(picker_fallback());
    }
    let data_file = matches!(purpose, "data-file" | "account-data-file");
    let max_bytes = picker_file_limit(purpose, &name, &mime_type)?;
    if picked_size.is_some_and(|size| size > max_bytes) {
        return Err(selected_file_too_large(max_bytes));
    }
    let directory = directory.ok_or_else(|| "MailCraft's private data folder is unavailable".to_string())?;
    let staged = super::platform::stage_android_document(activity, uri, &mime_type, &name, directory, max_bytes, !data_file)?;
    if data_file {
        return Ok((staged.to_string_lossy().into_owned(), Vec::new()));
    }
    if purpose == "attach" {
        return Ok((staged.to_string_lossy().into_owned(), Vec::new()));
    }
    let read = (|| -> Result<Vec<u8>, String> {
        let file = std::fs::File::open(&staged).map_err(|error| error.to_string())?;
        let mut reader = std::io::Read::take(file, max_bytes.saturating_add(1));
        let mut data = Vec::new();
        std::io::Read::read_to_end(&mut reader, &mut data).map_err(|error| error.to_string())?;
        if data.len() as u64 > max_bytes {
            return Err(selected_file_too_large(max_bytes));
        }
        Ok(data)
    })();
    super::handoff::discard_staged_android_file(&staged);
    let data = read?;
    let name = staged.file_name().and_then(std::ffi::OsStr::to_str).unwrap_or("attachment");
    let name = if pick_image { image_file_name(name, &data) } else { name.to_owned() };
    Ok((name, data))
}
