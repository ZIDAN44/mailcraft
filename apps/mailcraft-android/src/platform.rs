//! Android services that call the platform through JNI.
use super::activity;
use std::sync::atomic::{AtomicU64, Ordering};

mod display;
mod downloads;
mod links;
mod sharing;
pub(super) use display::{SystemInsets, dark_mode, font_scale, language_tag, phone_form_factor, system_bar_appearance, system_insets};
pub(super) use downloads::*;
pub(super) use links::{dial_phone, open_file, open_file_with_mime, open_map, open_url};
pub(super) use sharing::{add_to_system_calendar, add_to_system_contacts};
pub(super) use sharing::{share_calendar_event, share_contact, share_file, share_files, share_message};

/// Close a duplicate NativeActivity after its launch intent has been routed to the active window.
pub(super) fn finish_activity(app: &winit::platform::android::activity::AndroidApp) {
    let result = activity::with_activity(app, |env, activity| {
        let finished = env.call_method(activity, "finish", "()V", &[]);
        if finished.is_err() && env.exception_check().unwrap_or(false) {
            let _ = env.exception_clear();
        }
        finished.map(|_| ())
    });
    if let Err(error) = result {
        eprintln!("MailCraft could not close the duplicate Android window: {error}");
    }
}

/// Android's private directory excluded from automatic cloud backup.
pub(super) fn no_backup_data_path(app: &winit::platform::android::activity::AndroidApp) -> Option<std::path::PathBuf> {
    use jni::objects::JString;

    let result = activity::with_activity(app, |env, activity| {
        let operation = (|| -> robius_android_env::Result<Option<std::path::PathBuf>> {
            let directory = env.call_method(activity, "getNoBackupFilesDir", "()Ljava/io/File;", &[])?.l()?;
            if directory.is_null() {
                return Ok(None);
            }
            let path = env.call_method(&directory, "getAbsolutePath", "()Ljava/lang/String;", &[])?.l()?;
            if path.is_null() {
                return Ok(None);
            }
            let path = env.get_string(&JString::from(path))?.to_string_lossy().into_owned();
            Ok(Some(std::path::PathBuf::from(path)))
        })();
        if operation.is_err() && env.exception_check().unwrap_or(false) {
            let _ = env.exception_clear();
        }
        operation
    });
    match result {
        Ok(Ok(path)) => path,
        Ok(Err(error)) => {
            log::debug!("MailCraft could not read the Android clipboard picture: {error}");
            None
        }
        Err(error) => {
            eprintln!("MailCraft could not locate Android's no-backup directory: {error}");
            None
        }
    }
}

/// Whether Android currently has a network that can be used to reach mail servers.
pub(super) fn network_available(app: &winit::platform::android::activity::AndroidApp) -> Option<bool> {
    use jni::objects::JValue;

    let result = activity::with_activity(app, |env, activity| {
        let operation = (|| -> robius_android_env::Result<Option<bool>> {
            let service_name = env.new_string("connectivity")?;
            let manager = env
                .call_method(activity, "getSystemService", "(Ljava/lang/String;)Ljava/lang/Object;", &[JValue::Object(service_name.as_ref())])?
                .l()?;
            if manager.is_null() {
                return Ok(None);
            }
            let network = env.call_method(&manager, "getActiveNetwork", "()Landroid/net/Network;", &[])?.l()?;
            if network.is_null() {
                return Ok(Some(false));
            }
            let capabilities = env
                .call_method(
                    &manager,
                    "getNetworkCapabilities",
                    "(Landroid/net/Network;)Landroid/net/NetworkCapabilities;",
                    &[JValue::Object(&network)],
                )?
                .l()?;
            if capabilities.is_null() {
                return Ok(Some(false));
            }
            // The active network may reach a mail server on the local network without carrying
            // NET_CAPABILITY_INTERNET. Only wait when Android explicitly reports a captive portal.
            let captive_portal = env.call_method(&capabilities, "hasCapability", "(I)Z", &[JValue::Int(17)])?.z()?;
            let validated = env.call_method(&capabilities, "hasCapability", "(I)Z", &[JValue::Int(16)])?.z()?;
            Ok(Some(!captive_portal || validated))
        })();
        if operation.is_err() && env.exception_check().unwrap_or(false) {
            let _ = env.exception_clear();
        }
        operation
    });
    match result {
        Ok(Ok(available)) => available,
        Ok(Err(error)) | Err(error) => {
            log::debug!("MailCraft could not read Android network state: {error}");
            None
        }
    }
}

/// Decode a small video preview frame with Android's own media decoder.
pub(super) fn video_frame(app: &winit::platform::android::activity::AndroidApp, path: &std::path::Path) -> mailcraft_ui_egui::VideoFrame {
    const MAX_VIDEO_BYTES: u64 = 64 << 20;
    const MAX_DIMENSION: u32 = 480;
    const MAX_PIXELS: u64 = 16_000_000;
    // API 26 can't ask MediaMetadataRetriever for a scaled bitmap, so it must decode the whole
    // frame before we can scale it. Keep that older path small enough for a memory-constrained app.
    const MAX_UNSCALED_PIXELS: u64 = 4_000_000;
    const MAX_PNG_BYTES: usize = 8 << 20;
    use jni::objects::{JObject, JValue};
    use mailcraft_ui_egui::VideoFrame;

    let length = match std::fs::metadata(path) {
        Ok(metadata) => metadata.len(),
        Err(_) => return VideoFrame::Unavailable,
    };
    if length > MAX_VIDEO_BYTES {
        return VideoFrame::Unavailable;
    }
    let Some(path) = path.to_str() else { return VideoFrame::Unavailable };
    let result = activity::with_activity(app, |env, _activity| {
        let operation = (|| -> Result<Option<Vec<u8>>, Box<dyn std::error::Error>> {
            let retriever = env.new_object("android/media/MediaMetadataRetriever", "()V", &[])?;
            let frame_result = (|| -> Result<Option<Vec<u8>>, Box<dyn std::error::Error>> {
                let path = env.new_string(path)?;
                env.call_method(&retriever, "setDataSource", "(Ljava/lang/String;)V", &[JValue::Object(path.as_ref())])?;
                let sdk = env.get_static_field("android/os/Build$VERSION", "SDK_INT", "I")?.i()?;
                let width = video_dimension(env, &retriever, 18).unwrap_or_default();
                let height = video_dimension(env, &retriever, 19).unwrap_or_default();
                // Without dimensions the legacy decoder could allocate an arbitrary full-size
                // frame before encode_video_bitmap gets a chance to reject it.
                if width == 0 || height == 0 {
                    return Ok(None);
                }
                let can_scale = sdk >= 27;
                let pixel_limit = if can_scale { MAX_PIXELS } else { MAX_UNSCALED_PIXELS };
                if u64::from(width).saturating_mul(u64::from(height)) > pixel_limit {
                    return Ok(None);
                }
                let mut found = None;
                for time in [1_000_000_i64, 0] {
                    let allow_full_frame = !can_scale;
                    let mut frame = if can_scale {
                        let longest = width.max(height);
                        let scaled_width = u32::try_from((u64::from(width) * u64::from(MAX_DIMENSION) / u64::from(longest)).max(1))?;
                        let scaled_height = u32::try_from((u64::from(height) * u64::from(MAX_DIMENSION) / u64::from(longest)).max(1))?;
                        match env.call_method(
                            &retriever,
                            "getScaledFrameAtTime",
                            "(JIII)Landroid/graphics/Bitmap;",
                            &[JValue::Long(time), JValue::Int(2), JValue::Int(scaled_width as i32), JValue::Int(scaled_height as i32)],
                        ) {
                            Ok(frame) => frame.l()?,
                            Err(_) => {
                                if env.exception_check().unwrap_or(false) {
                                    let _ = env.exception_clear();
                                }
                                JObject::null()
                            }
                        }
                    } else {
                        JObject::null()
                    };
                    if frame.is_null() && allow_full_frame {
                        frame = match env.call_method(
                            &retriever,
                            "getFrameAtTime",
                            "(JI)Landroid/graphics/Bitmap;",
                            &[JValue::Long(time), JValue::Int(2)],
                        ) {
                            Ok(frame) => frame.l()?,
                            Err(_) => {
                                if env.exception_check().unwrap_or(false) {
                                    let _ = env.exception_clear();
                                }
                                JObject::null()
                            }
                        };
                    }
                    if frame.is_null() {
                        continue;
                    }
                    found = encode_video_bitmap(env, &frame, MAX_DIMENSION, pixel_limit, MAX_PNG_BYTES)?;
                    if found.is_some() {
                        break;
                    }
                }
                Ok(found)
            })();
            if frame_result.is_err() && env.exception_check().unwrap_or(false) {
                let _ = env.exception_clear();
            }
            let release = env.call_method(&retriever, "release", "()V", &[]);
            match (frame_result, release) {
                (Err(error), _) => Err(error),
                (Ok(_), Err(error)) => Err(error.into()),
                (Ok(frame), Ok(_)) => Ok(frame),
            }
        })();
        if operation.is_err() && env.exception_check().unwrap_or(false) {
            let _ = env.exception_clear();
        }
        operation
    });
    match result {
        Ok(Ok(Some(bytes))) if bytes.starts_with(b"\x89PNG\r\n\x1a\n") => VideoFrame::Ready(bytes),
        Ok(Ok(_)) => VideoFrame::Unavailable,
        Ok(Err(error)) => {
            eprintln!("MailCraft couldn't decode an Android video thumbnail: {error}");
            VideoFrame::Unavailable
        }
        Err(error) => {
            eprintln!("MailCraft couldn't decode an Android video thumbnail: {error}");
            VideoFrame::Unavailable
        }
    }
}

fn video_dimension(env: &mut jni::JNIEnv<'_>, retriever: &jni::objects::JObject<'_>, key: i32) -> Option<u32> {
    use jni::objects::{JString, JValue};

    let value = match env.call_method(retriever, "extractMetadata", "(I)Ljava/lang/String;", &[JValue::Int(key)]).and_then(|value| value.l()) {
        Ok(value) => value,
        Err(_) => {
            if env.exception_check().unwrap_or(false) {
                let _ = env.exception_clear();
            }
            return None;
        }
    };
    if value.is_null() {
        return None;
    }
    match env.get_string(&JString::from(value)) {
        Ok(value) => value.to_string_lossy().parse().ok(),
        Err(_) => {
            if env.exception_check().unwrap_or(false) {
                let _ = env.exception_clear();
            }
            None
        }
    }
}

fn encode_video_bitmap(
    env: &mut jni::JNIEnv<'_>,
    bitmap: &jni::objects::JObject<'_>,
    max_dimension: u32,
    max_pixels: u64,
    max_bytes: usize,
) -> Result<Option<Vec<u8>>, Box<dyn std::error::Error>> {
    use jni::objects::{JByteArray, JValue};

    let width = env.call_method(bitmap, "getWidth", "()I", &[])?.i()?;
    let height = env.call_method(bitmap, "getHeight", "()I", &[])?.i()?;
    if width <= 0 || height <= 0 || u64::try_from(width)?.saturating_mul(u64::try_from(height)?) > max_pixels {
        let _ = env.call_method(bitmap, "recycle", "()V", &[]);
        if env.exception_check().unwrap_or(false) {
            let _ = env.exception_clear();
        }
        return Ok(None);
    }
    let longest = width.max(height) as u32;
    let scaled = if longest > max_dimension {
        let scaled_width = (u64::try_from(width)? * u64::from(max_dimension) / u64::from(longest)).max(1) as i32;
        let scaled_height = (u64::try_from(height)? * u64::from(max_dimension) / u64::from(longest)).max(1) as i32;
        Some(
            env.call_static_method(
                "android/graphics/Bitmap",
                "createScaledBitmap",
                "(Landroid/graphics/Bitmap;IIZ)Landroid/graphics/Bitmap;",
                &[JValue::Object(bitmap), JValue::Int(scaled_width), JValue::Int(scaled_height), JValue::Bool(1)],
            )?
            .l()?,
        )
    } else {
        None
    };
    let encode_result = (|| -> Result<Option<Vec<u8>>, Box<dyn std::error::Error>> {
        let output = env.new_object("java/io/ByteArrayOutputStream", "()V", &[])?;
        let format = env.get_static_field("android/graphics/Bitmap$CompressFormat", "PNG", "Landroid/graphics/Bitmap$CompressFormat;")?.l()?;
        let picture = scaled.as_ref().filter(|picture| !picture.is_null()).unwrap_or(bitmap);
        let compressed = env
            .call_method(
                picture,
                "compress",
                "(Landroid/graphics/Bitmap$CompressFormat;ILjava/io/OutputStream;)Z",
                &[JValue::Object(&format), JValue::Int(100), JValue::Object(&output)],
            )?
            .z()?;
        if !compressed || usize::try_from(env.call_method(&output, "size", "()I", &[])?.i()?).unwrap_or(usize::MAX) > max_bytes {
            return Ok(None);
        }
        let bytes = env.call_method(&output, "toByteArray", "()[B", &[])?.l()?;
        if bytes.is_null() {
            return Ok(None);
        }
        let bytes: JByteArray<'_> = bytes.into();
        Ok(Some(env.convert_byte_array(&bytes)?))
    })();
    if encode_result.is_err() && env.exception_check().unwrap_or(false) {
        let _ = env.exception_clear();
    }
    if let Some(scaled) = scaled.as_ref().filter(|scaled| !scaled.is_null()) {
        let _ = env.call_method(scaled, "recycle", "()V", &[]);
    }
    let _ = env.call_method(bitmap, "recycle", "()V", &[]);
    if env.exception_check().unwrap_or(false) {
        let _ = env.exception_clear();
    }
    encode_result
}

pub(super) fn java_text_length(env: &mut jni::JNIEnv<'_>, value: &jni::objects::JObject<'_>) -> jni::errors::Result<i32> {
    if value.is_null() {
        return Ok(0);
    }
    env.call_method(value, "length", "()I", &[])?.i().map(|length| length.max(0))
}

pub(super) fn bounded_java_text(env: &mut jni::JNIEnv<'_>, value: &jni::objects::JObject<'_>, max_chars: i32) -> jni::errors::Result<String> {
    use jni::objects::JValue;

    if value.is_null() {
        return Ok(String::new());
    }
    let length = java_text_length(env, value)?.min(max_chars.max(0));
    let prefix = env.call_method(value, "subSequence", "(II)Ljava/lang/CharSequence;", &[JValue::Int(0), JValue::Int(length)])?.l()?;
    if prefix.is_null() {
        return Ok(String::new());
    }
    let text = env.call_method(&prefix, "toString", "()Ljava/lang/String;", &[])?.l()?;
    if text.is_null() {
        return Ok(String::new());
    }
    env.get_string(&jni::objects::JString::from(text)).map(|value| value.to_string_lossy().into_owned())
}

pub(super) fn document_name(env: &mut jni::JNIEnv<'_>, resolver: &jni::objects::JObject<'_>, uri: &jni::objects::JObject<'_>) -> Option<String> {
    use jni::objects::{JObject, JValue};

    let result = (|| -> jni::errors::Result<Option<String>> {
        let projection = env.new_object_array(1, "java/lang/String", JObject::null())?;
        let display_name = env.new_string("_display_name")?;
        env.set_object_array_element(&projection, 0, &display_name)?;
        let cursor = env
            .call_method(
                resolver,
                "query",
                "(Landroid/net/Uri;[Ljava/lang/String;Ljava/lang/String;[Ljava/lang/String;Ljava/lang/String;)Landroid/database/Cursor;",
                &[
                    JValue::Object(uri),
                    JValue::Object(projection.as_ref()),
                    JValue::Object(&JObject::null()),
                    JValue::Object(&JObject::null()),
                    JValue::Object(&JObject::null()),
                ],
            )?
            .l()?;
        if cursor.is_null() {
            return Ok(None);
        }
        let name = (|| -> jni::errors::Result<Option<String>> {
            if !env.call_method(&cursor, "moveToFirst", "()Z", &[])?.z()? {
                return Ok(None);
            }
            let column = env.call_method(&cursor, "getColumnIndex", "(Ljava/lang/String;)I", &[JValue::Object(display_name.as_ref())])?.i()?;
            if column < 0 {
                return Ok(None);
            }
            let value = env.call_method(&cursor, "getString", "(I)Ljava/lang/String;", &[JValue::Int(column)])?.l()?;
            if value.is_null() {
                return Ok(None);
            }
            Ok(Some(bounded_java_text(env, &value, 512)?))
        })();
        let close = env.call_method(&cursor, "close", "()V", &[]);
        match (name, close) {
            (Err(error), _) => Err(error),
            (Ok(_), Err(error)) => Err(error),
            (Ok(name), Ok(_)) => Ok(name),
        }
    })();
    if result.is_err() && env.exception_check().unwrap_or(false) {
        let _ = env.exception_clear();
    }
    result.ok().flatten().filter(|name| !name.trim().is_empty())
}

pub(super) fn cjk_font(lang: &str) -> Option<(Vec<u8>, u32)> {
    let index = match lang {
        "ja" => 0,
        "ko" => 1,
        "zh-hant" => 3,
        _ => 2,
    };
    for path in ["/system/fonts/NotoSansCJK-Regular.ttc", "/system/fonts/DroidSansFallbackFull.ttf"] {
        if let Ok(bytes) = std::fs::read(path) {
            let index = if path.ends_with("DroidSansFallbackFull.ttf") { 0 } else { index };
            return Some((bytes, index));
        }
    }
    None
}

pub(super) fn rtl_fonts() -> Vec<Vec<u8>> {
    let mut fonts = Vec::new();
    for candidates in [
        &["/system/fonts/NotoSansArabic-Regular.ttf", "/system/fonts/NotoNaskhArabicUI-Regular.ttf", "/system/fonts/NotoNaskhArabic-Regular.ttf"][..],
        &["/system/fonts/NotoSansHebrew-Regular.ttf"][..],
    ] {
        if let Some(bytes) = candidates.iter().find_map(|path| std::fs::read(path).ok()) {
            fonts.push(bytes);
        }
    }
    fonts
}

#[cfg(target_os = "android")]
pub(super) fn stage_android_document(
    app: &winit::platform::android::activity::AndroidApp,
    uri: &str,
    mime_type: &str,
    given_name: &str,
    directory: &std::path::Path,
    remaining_bytes: u64,
    as_attachment: bool,
) -> Result<std::path::PathBuf, String> {
    use jni::objects::JValue;

    static NEXT_HANDOFF: AtomicU64 = AtomicU64::new(0);
    let segment = given_name.rsplit(':').next().unwrap_or(given_name);
    let given_extension =
        std::path::Path::new(segment).extension().and_then(std::ffi::OsStr::to_str).map(str::to_ascii_lowercase).unwrap_or_default();
    let mime = mime_type.split(';').next().unwrap_or_default().trim().to_ascii_lowercase();
    let openable_extension = if matches!(given_extension.as_str(), "eml" | "msg" | "ics" | "ical" | "vcf" | "vcard" | "pst" | "mailcraft") {
        Some(given_extension.as_str())
    } else {
        match mime.as_str() {
            "message/rfc822" => Some("eml"),
            "application/vnd.ms-outlook" => Some("msg"),
            "application/vnd.storytold.mailcraft" => Some("mailcraft"),
            "text/calendar" | "text/vcalendar" | "text/x-vcalendar" | "application/ics" => Some("ics"),
            "text/vcard" | "text/x-vcard" => Some("vcf"),
            _ => None,
        }
    };
    if !as_attachment && openable_extension.is_none() {
        return Err(mailcraft_ui_egui::i18n::t("MailCraft can open shared .eml, .msg, .ics, .vcf, .pst and .mailcraft files").to_owned());
    }
    let extension = openable_extension.map(str::to_owned).unwrap_or_else(|| {
        if !given_extension.is_empty() {
            given_extension.clone()
        } else if mime.is_empty() || matches!(mime.as_str(), "*/*" | "application/octet-stream" | "binary/octet-stream") {
            String::new()
        } else {
            mime_guess::get_mime_extensions_str(&mime).and_then(|extensions| extensions.first()).copied().unwrap_or_default().to_owned()
        }
    });
    let max_bytes = if as_attachment {
        mailcraft_engine::MAX_ATTACHMENT_BYTES as u64
    } else if matches!(extension.as_str(), "pst" | "mailcraft") {
        mailcraft_engine::MAX_DATA_ARCHIVE_BYTES
    } else if matches!(extension.as_str(), "ics" | "ical" | "vcf" | "vcard") {
        mailcraft_engine::MAX_PIM_FILE_BYTES
    } else {
        mailcraft_engine::MAX_MAIL_FILE_BYTES
    }
    .min(remaining_bytes);
    std::fs::create_dir_all(directory).map_err(|error| error.to_string())?;
    let nonce = NEXT_HANDOFF.fetch_add(1, Ordering::Relaxed);
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |duration| duration.as_nanos());
    let file_dir = directory.join(format!("{now}-{nonce}"));
    std::fs::create_dir(&file_dir).map_err(|error| error.to_string())?;
    let given_name = if segment.trim().is_empty() { "shared".to_string() } else { segment.to_string() };
    let name = mailcraft_engine::mime::sanitize_filename(&given_name);
    let safe_path = std::path::Path::new(&name);
    let current_extension = safe_path.extension().and_then(std::ffi::OsStr::to_str).map(str::to_ascii_lowercase);
    let target = if !as_attachment && let Some(extension) = openable_extension {
        if current_extension.as_deref() == Some(extension) {
            name.clone()
        } else {
            let stem = safe_path.file_stem().and_then(std::ffi::OsStr::to_str).filter(|stem| !stem.is_empty()).unwrap_or("shared");
            format!("{stem}.{extension}")
        }
    } else if safe_path.extension().is_some() || extension.is_empty() {
        name.clone()
    } else {
        format!("{name}.{extension}")
    };
    let path = file_dir.join(target);
    let copy_result = (|| -> Result<(), String> {
        let result = activity::with_activity(app, |env, activity| {
            let operation = (|| -> Result<(), Box<dyn std::error::Error>> {
                let uri_string = env.new_string(uri)?;
                let android_uri = env
                    .call_static_method("android/net/Uri", "parse", "(Ljava/lang/String;)Landroid/net/Uri;", &[JValue::Object(uri_string.as_ref())])?
                    .l()?;
                let resolver = env.call_method(activity, "getContentResolver", "()Landroid/content/ContentResolver;", &[])?.l()?;
                let input = env
                    .call_method(&resolver, "openInputStream", "(Landroid/net/Uri;)Ljava/io/InputStream;", &[JValue::Object(&android_uri)])?
                    .l()?;
                if input.is_null() {
                    return Err(std::io::Error::other("Android returned no stream for the shared document").into());
                }
                let copy_result = (|| -> Result<(), Box<dyn std::error::Error>> {
                    let buffer = env.new_byte_array(32 * 1024)?;
                    let mut output = std::fs::File::create(&path)?;
                    let mut total = 0_u64;
                    loop {
                        let read = env
                            .call_method(&input, "read", "([BII)I", &[JValue::Object(buffer.as_ref()), JValue::Int(0), JValue::Int(32 * 1024)])?
                            .i()?;
                        if read < 0 {
                            break;
                        }
                        if read == 0 {
                            return Err(std::io::Error::other("Android's document provider stopped while reading the shared file").into());
                        }
                        let next = total.checked_add(read as u64).ok_or_else(|| std::io::Error::other("shared document is too large"))?;
                        if next > max_bytes {
                            return Err(std::io::Error::other(format!("shared document is larger than {} MiB", max_bytes >> 20)).into());
                        }
                        let bytes = env.convert_byte_array(&buffer)?;
                        let used = usize::try_from(read)?;
                        use std::io::Write as _;
                        output.write_all(bytes.get(..used).ok_or_else(|| std::io::Error::other("Android returned an invalid document chunk"))?)?;
                        total = next;
                    }
                    output.sync_all()?;
                    Ok(())
                })();
                if copy_result.is_err() && env.exception_check().unwrap_or(false) {
                    let _ = env.exception_clear();
                }
                let close_result = env.call_method(&input, "close", "()V", &[]);
                match (copy_result, close_result) {
                    (Err(error), _) => Err(error),
                    (Ok(()), Err(error)) => Err(error.into()),
                    (Ok(()), Ok(_)) => Ok(()),
                }
            })();
            if operation.is_err() && env.exception_check().unwrap_or(false) {
                let _ = env.exception_clear();
            }
            operation
        });
        match result {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => Err(error.to_string()),
            Err(error) => Err(error.to_string()),
        }
    })();
    if let Err(error) = copy_result {
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&file_dir);
        return Err(error);
    }
    Ok(path)
}

/// Open the Android system page for this app's notification permission and channels.
pub(super) fn open_notification_settings(app: &winit::platform::android::activity::AndroidApp) -> Result<(), String> {
    use jni::objects::JValue;

    let result = activity::with_activity(app, |env, activity| {
        let operation = (|| -> robius_android_env::Result<()> {
            let action = env.new_string("android.settings.APP_NOTIFICATION_SETTINGS")?;
            let intent = env.new_object("android/content/Intent", "(Ljava/lang/String;)V", &[JValue::Object(action.as_ref())])?;
            let package_key = env.new_string("android.provider.extra.APP_PACKAGE")?;
            let package = env.call_method(activity, "getPackageName", "()Ljava/lang/String;", &[])?.l()?;
            env.call_method(
                &intent,
                "putExtra",
                "(Ljava/lang/String;Ljava/lang/String;)Landroid/content/Intent;",
                &[JValue::Object(package_key.as_ref()), JValue::Object(&package)],
            )?;
            env.call_method(activity, "startActivity", "(Landroid/content/Intent;)V", &[JValue::Object(&intent)])?;
            Ok(())
        })();
        if operation.is_err() && env.exception_check().unwrap_or(false) {
            let _ = env.exception_clear();
        }
        operation
    });
    match result {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) | Err(error) => Err(error.to_string()),
    }
}

pub(super) fn timezone(app: &winit::platform::android::activity::AndroidApp) -> Option<String> {
    let result = activity::with_activity(app, |env, _activity| activity::system_timezone_from_env(env));
    match result {
        Ok(Ok(zone)) => zone,
        Ok(Err(error)) | Err(error) => {
            eprintln!("MailCraft could not read Android's time zone: {error}");
            None
        }
    }
}

mod clipboard;
pub(super) use clipboard::{
    ClipboardText, clipboard_files, clipboard_has_files, clipboard_has_image, clipboard_picture, clipboard_text, copy_text, discard_clipboard_files,
};
