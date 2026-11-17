//! Android clipboard image and file access.
use super::super::activity;
use super::*;

pub(crate) enum ClipboardText {
    Empty,
    Image,
    Text(String),
    TooLarge,
}

/// Whether Android's clipboard currently contains an image, without decoding or copying it.
pub(crate) fn clipboard_has_image(app: &winit::platform::android::activity::AndroidApp) -> Result<bool, String> {
    use jni::objects::JValue;

    let result = activity::with_activity(app, |env, activity| {
        let operation = (|| -> Result<bool, Box<dyn std::error::Error>> {
            let service = env.new_string("clipboard")?;
            let manager =
                env.call_method(activity, "getSystemService", "(Ljava/lang/String;)Ljava/lang/Object;", &[JValue::Object(service.as_ref())])?.l()?;
            if manager.is_null() {
                return Ok(false);
            }
            let clip = env.call_method(&manager, "getPrimaryClip", "()Landroid/content/ClipData;", &[])?.l()?;
            if clip.is_null() {
                return Ok(false);
            }
            let description = env.call_method(&clip, "getDescription", "()Landroid/content/ClipDescription;", &[])?.l()?;
            if description.is_null() {
                return Ok(false);
            }
            let image_mime = env.new_string("image/*")?;
            Ok(env.call_method(&description, "hasMimeType", "(Ljava/lang/String;)Z", &[JValue::Object(image_mime.as_ref())])?.z()?)
        })();
        if operation.is_err() && env.exception_check().unwrap_or(false) {
            let _ = env.exception_clear();
        }
        operation
    });
    match result {
        Ok(Ok(has_image)) => Ok(has_image),
        Ok(Err(error)) => Err(error.to_string()),
        Err(error) => Err(error.to_string()),
    }
}

/// Read plain text from Android's clipboard for an egui paste event.
pub(crate) fn clipboard_text(app: &winit::platform::android::activity::AndroidApp) -> Result<ClipboardText, String> {
    const MAX_CLIPBOARD_BYTES: usize = 4 << 20;
    // Bound the Java String conversion before allocating it, then apply the exact byte limit.
    const MAX_CLIPBOARD_UTF16_UNITS: usize = MAX_CLIPBOARD_BYTES;
    use jni::objects::{JString, JValue};

    let result = activity::with_activity(app, |env, activity| {
        let operation = (|| -> Result<ClipboardText, Box<dyn std::error::Error>> {
            let service = env.new_string("clipboard")?;
            let manager =
                env.call_method(activity, "getSystemService", "(Ljava/lang/String;)Ljava/lang/Object;", &[JValue::Object(service.as_ref())])?.l()?;
            if manager.is_null() {
                return Ok(ClipboardText::Empty);
            }
            let clip = env.call_method(&manager, "getPrimaryClip", "()Landroid/content/ClipData;", &[])?.l()?;
            if clip.is_null() {
                return Ok(ClipboardText::Empty);
            }
            let description = env.call_method(&clip, "getDescription", "()Landroid/content/ClipDescription;", &[])?.l()?;
            if description.is_null() {
                return Ok(ClipboardText::Empty);
            }
            let mime = env.new_string("text/plain")?;
            let has_text = env.call_method(&description, "hasMimeType", "(Ljava/lang/String;)Z", &[JValue::Object(mime.as_ref())])?.z()?;
            if has_text {
                let items = env.call_method(&clip, "getItemCount", "()I", &[])?.i()?.clamp(0, 16);
                for index in 0..items {
                    let item = env.call_method(&clip, "getItemAt", "(I)Landroid/content/ClipData$Item;", &[JValue::Int(index)])?.l()?;
                    if item.is_null() {
                        continue;
                    }
                    let text = env.call_method(&item, "getText", "()Ljava/lang/CharSequence;", &[])?.l()?;
                    if text.is_null() {
                        continue;
                    }
                    let length = env.call_method(&text, "length", "()I", &[])?.i()?;
                    if length < 0 || usize::try_from(length)? > MAX_CLIPBOARD_UTF16_UNITS {
                        return Ok(ClipboardText::TooLarge);
                    }
                    let value = env.call_method(&text, "toString", "()Ljava/lang/String;", &[])?.l()?;
                    if value.is_null() {
                        continue;
                    }
                    let value = env.get_string(&JString::from(value))?.to_string_lossy().into_owned();
                    if value.len() > MAX_CLIPBOARD_BYTES {
                        return Ok(ClipboardText::TooLarge);
                    }
                    return Ok(if value.is_empty() { ClipboardText::Empty } else { ClipboardText::Text(value) });
                }
            }
            let image_mime = env.new_string("image/*")?;
            if env.call_method(&description, "hasMimeType", "(Ljava/lang/String;)Z", &[JValue::Object(image_mime.as_ref())])?.z()? {
                return Ok(ClipboardText::Image);
            }
            Ok(ClipboardText::Empty)
        })();
        if operation.is_err() && env.exception_check().unwrap_or(false) {
            let _ = env.exception_clear();
        }
        operation
    });
    match result {
        Ok(Ok(text)) => Ok(text),
        Ok(Err(error)) => Err(error.to_string()),
        Err(error) => Err(error.to_string()),
    }
}

/// Put copied MailCraft text on Android's system clipboard.
pub(crate) fn copy_text(app: &winit::platform::android::activity::AndroidApp, text: &str) -> Result<(), String> {
    const MAX_CLIP_TEXT_BYTES: usize = 4 << 20;
    if text.len() > MAX_CLIP_TEXT_BYTES {
        return Err("The selected text is too large for Android's clipboard".into());
    }
    use jni::objects::JValue;

    let result = activity::with_activity(app, |env, activity| {
        let operation = (|| -> robius_android_env::Result<()> {
            let service = env.new_string("clipboard")?;
            let manager =
                env.call_method(activity, "getSystemService", "(Ljava/lang/String;)Ljava/lang/Object;", &[JValue::Object(service.as_ref())])?.l()?;
            if manager.is_null() {
                return Err(jni::errors::Error::NullPtr("Android clipboard manager"));
            }
            let label = env.new_string("MailCraft")?;
            let value = env.new_string(text)?;
            let clip = env
                .call_static_method(
                    "android/content/ClipData",
                    "newPlainText",
                    "(Ljava/lang/CharSequence;Ljava/lang/CharSequence;)Landroid/content/ClipData;",
                    &[JValue::Object(label.as_ref()), JValue::Object(value.as_ref())],
                )?
                .l()?;
            env.call_method(manager, "setPrimaryClip", "(Landroid/content/ClipData;)V", &[JValue::Object(&clip)])?;
            Ok(())
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
}

/// Read a copied image from Android's clipboard, returning the PNG bytes the editor expects.
pub(crate) fn clipboard_picture(app: &winit::platform::android::activity::AndroidApp) -> Option<Vec<u8>> {
    const MAX_CLIP_BYTES: usize = 16 << 20;
    use jni::objects::JValue;

    let result = activity::with_activity(app, |env, activity| {
        let operation = (|| -> Result<Option<Vec<u8>>, Box<dyn std::error::Error>> {
            let service = env.new_string("clipboard")?;
            let manager =
                env.call_method(activity, "getSystemService", "(Ljava/lang/String;)Ljava/lang/Object;", &[JValue::Object(service.as_ref())])?.l()?;
            if manager.is_null() {
                return Ok(None);
            }
            let clip = env.call_method(&manager, "getPrimaryClip", "()Landroid/content/ClipData;", &[])?.l()?;
            if clip.is_null() {
                return Ok(None);
            }
            let description = env.call_method(&clip, "getDescription", "()Landroid/content/ClipDescription;", &[])?.l()?;
            if description.is_null() {
                return Ok(None);
            }
            let image_type = env.new_string("image/*")?;
            let has_image = env.call_method(&description, "hasMimeType", "(Ljava/lang/String;)Z", &[JValue::Object(image_type.as_ref())])?.z()?;
            if !has_image {
                return Ok(None);
            }
            let item_count = env.call_method(&clip, "getItemCount", "()I", &[])?.i()?.clamp(0, 32);
            let mut image_uri = None;
            for index in 0..item_count {
                let item = env.call_method(&clip, "getItemAt", "(I)Landroid/content/ClipData$Item;", &[JValue::Int(index)])?.l()?;
                if item.is_null() {
                    continue;
                }
                let uri = env.call_method(&item, "getUri", "()Landroid/net/Uri;", &[])?.l()?;
                if !uri.is_null() {
                    image_uri = Some(uri);
                    break;
                }
            }
            let Some(uri) = image_uri else { return Ok(None) };
            let resolver = env.call_method(activity, "getContentResolver", "()Landroid/content/ContentResolver;", &[])?.l()?;
            let input = env.call_method(&resolver, "openInputStream", "(Landroid/net/Uri;)Ljava/io/InputStream;", &[JValue::Object(&uri)])?.l()?;
            if input.is_null() {
                return Ok(None);
            }
            let bytes_result = (|| -> Result<Vec<u8>, Box<dyn std::error::Error>> {
                let buffer = env.new_byte_array(16 * 1024)?;
                let mut bytes = Vec::new();
                loop {
                    let read = env
                        .call_method(&input, "read", "([BII)I", &[JValue::Object(buffer.as_ref()), JValue::Int(0), JValue::Int(16 * 1024)])?
                        .i()?;
                    if read < 0 {
                        break;
                    }
                    if read == 0 {
                        return Err(std::io::Error::other("Android's clipboard provider stopped while reading the image").into());
                    }
                    let next = bytes.len().checked_add(read as usize).ok_or_else(|| std::io::Error::other("clipboard image is too large"))?;
                    if next > MAX_CLIP_BYTES {
                        return Err(std::io::Error::other("clipboard image is larger than 16 MiB").into());
                    }
                    let chunk = env.convert_byte_array(&buffer)?;
                    let used = usize::try_from(read)?;
                    bytes.extend_from_slice(
                        chunk.get(..used).ok_or_else(|| std::io::Error::other("Android returned an invalid clipboard image chunk"))?,
                    );
                }
                Ok(bytes)
            })();
            if bytes_result.is_err() && env.exception_check().unwrap_or(false) {
                let _ = env.exception_clear();
            }
            let close_result = env.call_method(&input, "close", "()V", &[]);
            match (bytes_result, close_result) {
                (Err(error), _) => Err(error),
                (Ok(_), Err(error)) => Err(error.into()),
                (Ok(bytes), Ok(_)) => Ok(Some(bytes)),
            }
        })();
        if operation.is_err() && env.exception_check().unwrap_or(false) {
            let _ = env.exception_clear();
        }
        operation
    });
    let bytes = match result {
        Ok(Ok(Some(bytes))) => bytes,
        Ok(Ok(None)) => return None,
        Ok(Err(_)) | Err(_) => {
            eprintln!("MailCraft could not read an image from Android's clipboard");
            return None;
        }
    };
    decode_clipboard_picture(&bytes)
}

/// Stage copied document URIs in private storage so the shared compose command can read them.
pub(crate) fn clipboard_has_files(app: &winit::platform::android::activity::AndroidApp) -> Result<bool, String> {
    use jni::objects::JValue;

    let result = activity::with_activity(app, |env, activity| {
        let operation = (|| -> Result<bool, Box<dyn std::error::Error>> {
            let service = env.new_string("clipboard")?;
            let manager =
                env.call_method(activity, "getSystemService", "(Ljava/lang/String;)Ljava/lang/Object;", &[JValue::Object(service.as_ref())])?.l()?;
            if manager.is_null() {
                return Ok(false);
            }
            let clip = env.call_method(&manager, "getPrimaryClip", "()Landroid/content/ClipData;", &[])?.l()?;
            if clip.is_null() {
                return Ok(false);
            }
            let description = env.call_method(&clip, "getDescription", "()Landroid/content/ClipDescription;", &[])?.l()?;
            if description.is_null() {
                return Ok(false);
            }
            let mime_count = env.call_method(&description, "getMimeTypeCount", "()I", &[])?.i()?.clamp(0, 16);
            for index in 0..mime_count {
                let mime = env.call_method(&description, "getMimeType", "(I)Ljava/lang/String;", &[JValue::Int(index)])?.l()?;
                if mime.is_null() {
                    continue;
                }
                let mime = super::bounded_java_text(env, &mime, 256)?;
                if !mime.starts_with("text/") || mime.eq_ignore_ascii_case("text/uri-list") {
                    return Ok(true);
                }
            }
            Ok(false)
        })();
        if operation.is_err() && env.exception_check().unwrap_or(false) {
            let _ = env.exception_clear();
        }
        operation
    });
    match result {
        Ok(Ok(has_files)) => Ok(has_files),
        Ok(Err(error)) => Err(error.to_string()),
        Err(error) => Err(error.to_string()),
    }
}

pub(crate) fn clipboard_files(app: &winit::platform::android::activity::AndroidApp, directory: &std::path::Path) -> Vec<std::path::PathBuf> {
    const MAX_FILES: usize = 10;
    const MAX_TOTAL_BYTES: u64 = mailcraft_engine::MAX_ATTACHMENT_BYTES as u64;
    static NEXT_BATCH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    let time = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |duration| duration.as_nanos());
    let batch = NEXT_BATCH.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let batch_dir = directory.join(format!("{time}-{batch}"));
    if std::fs::create_dir_all(directory).is_err() || std::fs::create_dir(&batch_dir).is_err() {
        eprintln!("MailCraft could not prepare copied clipboard files");
        return Vec::new();
    }

    let result = activity::with_activity(app, |env, activity| {
        let operation = (|| -> Result<Vec<std::path::PathBuf>, Box<dyn std::error::Error>> {
            use jni::objects::JValue;

            let service = env.new_string("clipboard")?;
            let manager =
                env.call_method(activity, "getSystemService", "(Ljava/lang/String;)Ljava/lang/Object;", &[JValue::Object(service.as_ref())])?.l()?;
            if manager.is_null() {
                return Ok(Vec::new());
            }
            let clip = env.call_method(&manager, "getPrimaryClip", "()Landroid/content/ClipData;", &[])?.l()?;
            if clip.is_null() {
                return Ok(Vec::new());
            }
            let description = env.call_method(&clip, "getDescription", "()Landroid/content/ClipDescription;", &[])?.l()?;
            if description.is_null() {
                return Ok(Vec::new());
            }
            let mime_count = env.call_method(&description, "getMimeTypeCount", "()I", &[])?.i()?.clamp(0, 16);
            let mut has_file_type = false;
            for mime_index in 0..mime_count {
                let mime = env.call_method(&description, "getMimeType", "(I)Ljava/lang/String;", &[JValue::Int(mime_index)])?.l()?;
                if mime.is_null() {
                    continue;
                }
                let mime = super::bounded_java_text(env, &mime, 256)?;
                // Android's file managers commonly describe copied document URIs as a URI
                // list, which is a text MIME type even though the ClipData items are files.
                if !mime.starts_with("text/") || mime.eq_ignore_ascii_case("text/uri-list") {
                    has_file_type = true;
                    break;
                }
            }
            if !has_file_type {
                return Ok(Vec::new());
            }
            let resolver = env.call_method(activity, "getContentResolver", "()Landroid/content/ContentResolver;", &[])?.l()?;
            let count = env.call_method(&clip, "getItemCount", "()I", &[])?.i()?.clamp(0, MAX_FILES as i32);
            let mut files = Vec::new();
            let mut total = 0_u64;
            for index in 0..count {
                let item = env.call_method(&clip, "getItemAt", "(I)Landroid/content/ClipData$Item;", &[JValue::Int(index)])?.l()?;
                if item.is_null() {
                    continue;
                }
                let uri = env.call_method(&item, "getUri", "()Landroid/net/Uri;", &[])?.l()?;
                if uri.is_null() {
                    continue;
                }
                let scheme = env.call_method(&uri, "getScheme", "()Ljava/lang/String;", &[])?.l()?;
                let scheme = super::bounded_java_text(env, &scheme, 32)?;
                if !["content", "file"].iter().any(|allowed| scheme.eq_ignore_ascii_case(allowed)) {
                    continue;
                }
                let fallback = env.call_method(&uri, "getLastPathSegment", "()Ljava/lang/String;", &[])?.l()?;
                let fallback = super::bounded_java_text(env, &fallback, 512)?;
                let given_name = document_name(env, &resolver, &uri).unwrap_or(fallback);
                let safe_name = mailcraft_engine::mime::sanitize_filename(if given_name.is_empty() { "attachment" } else { &given_name });
                let name = unique_clipboard_name(&safe_name, index as usize);
                let path = batch_dir.join(name);
                let remaining = MAX_TOTAL_BYTES.saturating_sub(total);
                if remaining == 0 {
                    break;
                }
                match copy_clipboard_uri(env, activity, &uri, &path, remaining) {
                    Ok(size) => {
                        total = total.saturating_add(size);
                        files.push(path);
                    }
                    Err(_) => {
                        let _ = std::fs::remove_file(&path);
                        if env.exception_check().unwrap_or(false) {
                            let _ = env.exception_clear();
                        }
                    }
                }
            }
            Ok(files)
        })();
        if operation.is_err() && env.exception_check().unwrap_or(false) {
            let _ = env.exception_clear();
        }
        operation
    });
    match result {
        Ok(Ok(files)) if !files.is_empty() => files,
        Ok(Ok(_)) => {
            let _ = std::fs::remove_dir(&batch_dir);
            Vec::new()
        }
        Ok(Err(_)) | Err(_) => {
            cleanup_clipboard_batch(directory, &batch_dir);
            eprintln!("MailCraft could not read copied clipboard files");
            Vec::new()
        }
    }
}

/// Remove staged clipboard copies when the UI has closed before it could attach them.
pub(crate) fn discard_clipboard_files(root: &std::path::Path, files: &[std::path::PathBuf]) {
    let Some(batch) = files.first().and_then(|file| file.parent()) else { return };
    if files.iter().all(|file| file.parent() == Some(batch)) {
        cleanup_clipboard_batch(root, batch);
    }
}

fn unique_clipboard_name(name: &str, index: usize) -> String {
    use std::path::Path;

    if index == 0 {
        return name.to_owned();
    }
    let path = Path::new(name);
    let stem = path.file_stem().and_then(std::ffi::OsStr::to_str).unwrap_or("attachment");
    match path.extension().and_then(std::ffi::OsStr::to_str) {
        Some(extension) => format!("{stem} ({index}).{extension}"),
        None => format!("{stem} ({index})"),
    }
}

fn copy_clipboard_uri<'env, 'local>(
    env: &mut jni::JNIEnv<'env>,
    activity: &jni::objects::JObject<'local>,
    uri: &jni::objects::JObject<'local>,
    path: &std::path::Path,
    max_bytes: u64,
) -> Result<u64, Box<dyn std::error::Error>> {
    use jni::objects::JValue;
    use std::io::Write as _;

    let resolver = env.call_method(activity, "getContentResolver", "()Landroid/content/ContentResolver;", &[])?.l()?;
    let input = env.call_method(&resolver, "openInputStream", "(Landroid/net/Uri;)Ljava/io/InputStream;", &[JValue::Object(uri)])?.l()?;
    if input.is_null() {
        return Err(std::io::Error::other("clipboard file has no readable stream").into());
    }
    let copy = (|| -> Result<u64, Box<dyn std::error::Error>> {
        let buffer = env.new_byte_array(32 * 1024)?;
        let mut output = std::fs::OpenOptions::new().write(true).create_new(true).open(path)?;
        let mut total = 0_u64;
        loop {
            let read = env.call_method(&input, "read", "([BII)I", &[JValue::Object(buffer.as_ref()), JValue::Int(0), JValue::Int(32 * 1024)])?.i()?;
            if read < 0 {
                break;
            }
            if read == 0 {
                return Err(std::io::Error::other("clipboard provider stopped while reading a file").into());
            }
            let next = total.checked_add(read as u64).ok_or_else(|| std::io::Error::other("clipboard files are too large"))?;
            if next > max_bytes {
                return Err(std::io::Error::other("clipboard files exceed the message attachment limit").into());
            }
            let chunk = env.convert_byte_array(&buffer)?;
            let used = usize::try_from(read)?;
            output.write_all(chunk.get(..used).ok_or_else(|| std::io::Error::other("Android returned an invalid clipboard file chunk"))?)?;
            total = next;
        }
        output.sync_all()?;
        Ok(total)
    })();
    if copy.is_err() && env.exception_check().unwrap_or(false) {
        let _ = env.exception_clear();
    }
    let close = env.call_method(&input, "close", "()V", &[]);
    match (copy, close) {
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(error.into()),
        (Ok(size), Ok(_)) => Ok(size),
    }
}

fn cleanup_clipboard_batch(root: &std::path::Path, batch: &std::path::Path) {
    let (Some(root), Some(batch)) = (root.canonicalize().ok(), batch.canonicalize().ok()) else { return };
    if batch.parent() != Some(root.as_path()) {
        return;
    }
    if let Ok(entries) = std::fs::read_dir(&batch) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                let _ = std::fs::remove_file(path);
            }
        }
    }
    let _ = std::fs::remove_dir(batch);
}

fn decode_clipboard_picture(bytes: &[u8]) -> Option<Vec<u8>> {
    use image::ImageDecoder as _;
    use std::io::Cursor;

    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(64 << 20);
    let mut reader = image::ImageReader::new(Cursor::new(bytes)).with_guessed_format().ok()?;
    reader.limits(limits.clone());
    let mut decoder = reader.into_decoder().ok()?;
    decoder.set_limits(limits).ok()?;
    let (width, height) = decoder.dimensions();
    let pixels = u64::from(width).checked_mul(u64::from(height))?;
    if width == 0 || height == 0 || pixels > 16_000_000 {
        return None;
    }
    let image = image::DynamicImage::from_decoder(decoder).ok()?;
    let image = if width.max(height) > 2048 { image.resize(2048, 2048, image::imageops::FilterType::Triangle) } else { image };
    let mut png = Cursor::new(Vec::new());
    image.write_to(&mut png, image::ImageFormat::Png).ok()?;
    (png.get_ref().len() <= 32 << 20).then(|| png.into_inner())
}
