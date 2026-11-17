//! Streaming writes to Android's public Downloads collection.
use super::super::activity;

/// Remove Downloads rows this app left pending if Android stopped it during a previous write.
pub(crate) fn cleanup_pending_downloads(app: &winit::platform::android::activity::AndroidApp) {
    use jni::objects::{JObject, JValue};

    let result = activity::with_activity(app, |env, activity| {
        let operation = (|| -> robius_android_env::Result<()> {
            let sdk = env.get_static_field("android/os/Build$VERSION", "SDK_INT", "I")?.i()?;
            if sdk < 29 {
                return Ok(());
            }
            let collection = env.get_static_field("android/provider/MediaStore$Downloads", "EXTERNAL_CONTENT_URI", "Landroid/net/Uri;")?.l()?;
            let resolver = env.call_method(activity, "getContentResolver", "()Landroid/content/ContentResolver;", &[])?.l()?;
            let selection = env.new_string("is_pending = 1 AND relative_path = ? AND owner_package_name = ?")?;
            let selection_args = env.new_object_array(2, "java/lang/String", JObject::null())?;
            let relative_path = env.new_string("Download/MailCraft/")?;
            let owner_package = env.call_method(activity, "getPackageName", "()Ljava/lang/String;", &[])?.l()?;
            env.set_object_array_element(&selection_args, 0, &relative_path)?;
            env.set_object_array_element(&selection_args, 1, &owner_package)?;
            env.call_method(
                &resolver,
                "delete",
                "(Landroid/net/Uri;Ljava/lang/String;[Ljava/lang/String;)I",
                &[JValue::Object(&collection), JValue::Object(selection.as_ref()), JValue::Object(&selection_args)],
            )?;
            Ok(())
        })();
        if operation.is_err() && env.exception_check().unwrap_or(false) {
            let _ = env.exception_clear();
        }
        operation
    });
    match result {
        Ok(Ok(())) => {}
        Ok(Err(error)) | Err(error) => log::debug!("MailCraft could not clean up interrupted Android downloads: {error}"),
    }
}

pub(crate) fn create_download_uri(app: &winit::platform::android::activity::AndroidApp, name: &str, mime_type: &str) -> Result<String, String> {
    use jni::objects::{JObject, JString, JValue};

    let result = activity::with_activity(app, |env, activity| {
        let operation = (|| -> Result<String, Box<dyn std::error::Error>> {
            let sdk = env.get_static_field("android/os/Build$VERSION", "SDK_INT", "I")?.i()?;
            if sdk < 29 {
                return Err(std::io::Error::other("The Android Downloads folder needs Android 10 or newer").into());
            }
            let values = env.new_object("android/content/ContentValues", "()V", &[])?;
            for (key, value) in [("_display_name", name), ("mime_type", mime_type), ("relative_path", "Download/MailCraft")] {
                let key = env.new_string(key)?;
                let value = env.new_string(value)?;
                env.call_method(
                    &values,
                    "put",
                    "(Ljava/lang/String;Ljava/lang/String;)V",
                    &[JValue::Object(key.as_ref()), JValue::Object(value.as_ref())],
                )?;
            }
            let pending_key = env.new_string("is_pending")?;
            let pending_value = env.call_static_method("java/lang/Integer", "valueOf", "(I)Ljava/lang/Integer;", &[JValue::Int(1)])?.l()?;
            env.call_method(
                &values,
                "put",
                "(Ljava/lang/String;Ljava/lang/Integer;)V",
                &[JValue::Object(pending_key.as_ref()), JValue::Object(&pending_value)],
            )?;
            let collection = env.get_static_field("android/provider/MediaStore$Downloads", "EXTERNAL_CONTENT_URI", "Landroid/net/Uri;")?.l()?;
            let resolver = env.call_method(activity, "getContentResolver", "()Landroid/content/ContentResolver;", &[])?.l()?;
            let uri = env
                .call_method(
                    &resolver,
                    "insert",
                    "(Landroid/net/Uri;Landroid/content/ContentValues;)Landroid/net/Uri;",
                    &[JValue::Object(&collection), JValue::Object(&values)],
                )?
                .l()?;
            if uri.is_null() {
                return Err(std::io::Error::other("Android did not create a Downloads entry").into());
            }
            let uri_text = env.call_method(&uri, "toString", "()Ljava/lang/String;", &[])?.l()?;
            let uri_text = env.get_string(&JString::from(uri_text))?.to_string_lossy().into_owned();
            if uri_text.len() > 8192 {
                let null = JObject::null();
                let _ = env.call_method(
                    &resolver,
                    "delete",
                    "(Landroid/net/Uri;Ljava/lang/String;[Ljava/lang/String;)I",
                    &[JValue::Object(&uri), JValue::Object(&null), JValue::Object(&null)],
                );
                return Err(std::io::Error::other("Android returned an invalid Downloads URI").into());
            }
            Ok(uri_text)
        })();
        if operation.is_err() && env.exception_check().unwrap_or(false) {
            let _ = env.exception_clear();
        }
        operation
    });
    match result {
        Ok(Ok(uri)) => Ok(uri),
        Ok(Err(error)) => Err(error.to_string()),
        Err(error) => Err(error.to_string()),
    }
}

pub(crate) fn publish_download_uri(app: &winit::platform::android::activity::AndroidApp, uri: &str) -> Result<(), String> {
    use jni::objects::{JObject, JValue};

    let result = activity::with_activity(app, |env, activity| {
        let operation = (|| -> robius_android_env::Result<()> {
            let uri_string = env.new_string(uri)?;
            let android_uri = env
                .call_static_method("android/net/Uri", "parse", "(Ljava/lang/String;)Landroid/net/Uri;", &[JValue::Object(uri_string.as_ref())])?
                .l()?;
            let values = env.new_object("android/content/ContentValues", "()V", &[])?;
            let key = env.new_string("is_pending")?;
            let zero = env.call_static_method("java/lang/Integer", "valueOf", "(I)Ljava/lang/Integer;", &[JValue::Int(0)])?.l()?;
            env.call_method(&values, "put", "(Ljava/lang/String;Ljava/lang/Integer;)V", &[JValue::Object(key.as_ref()), JValue::Object(&zero)])?;
            let resolver = env.call_method(activity, "getContentResolver", "()Landroid/content/ContentResolver;", &[])?.l()?;
            let null = JObject::null();
            let updated = env
                .call_method(
                    &resolver,
                    "update",
                    "(Landroid/net/Uri;Landroid/content/ContentValues;Ljava/lang/String;[Ljava/lang/String;)I",
                    &[JValue::Object(&android_uri), JValue::Object(&values), JValue::Object(&null), JValue::Object(&null)],
                )?
                .i()?;
            if updated == 0 {
                return Err(jni::errors::Error::NullPtr("Android Downloads entry"));
            }
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

pub(crate) fn delete_uri(app: &winit::platform::android::activity::AndroidApp, uri: &str) {
    use jni::objects::{JObject, JValue};

    let _ = activity::with_activity(app, |env, activity| {
        let operation = (|| -> robius_android_env::Result<()> {
            let uri = env.new_string(uri)?;
            let uri =
                env.call_static_method("android/net/Uri", "parse", "(Ljava/lang/String;)Landroid/net/Uri;", &[JValue::Object(uri.as_ref())])?.l()?;
            let resolver = env.call_method(activity, "getContentResolver", "()Landroid/content/ContentResolver;", &[])?.l()?;
            let null = JObject::null();
            env.call_method(
                &resolver,
                "delete",
                "(Landroid/net/Uri;Ljava/lang/String;[Ljava/lang/String;)I",
                &[JValue::Object(&uri), JValue::Object(&null), JValue::Object(&null)],
            )?;
            Ok(())
        })();
        if operation.is_err() && env.exception_check().unwrap_or(false) {
            let _ = env.exception_clear();
        }
        operation
    });
}

pub(crate) fn write_file_to_uri(
    app: &winit::platform::android::activity::AndroidApp,
    uri: &str,
    source_path: &std::path::Path,
) -> Result<(), String> {
    let mut source = std::fs::File::open(source_path).map_err(|error| error.to_string())?;
    write_reader_to_uri(app, uri, &mut source)
}

pub(crate) fn write_bytes_to_uri(app: &winit::platform::android::activity::AndroidApp, uri: &str, bytes: &[u8]) -> Result<(), String> {
    let mut source = std::io::Cursor::new(bytes);
    write_reader_to_uri(app, uri, &mut source)
}

fn write_reader_to_uri(app: &winit::platform::android::activity::AndroidApp, uri: &str, source: &mut dyn std::io::Read) -> Result<(), String> {
    use jni::objects::JValue;

    let result = activity::with_activity(app, |env, activity| {
        let operation = (|| -> Result<(), Box<dyn std::error::Error>> {
            let uri_string = env.new_string(uri)?;
            let android_uri = env
                .call_static_method("android/net/Uri", "parse", "(Ljava/lang/String;)Landroid/net/Uri;", &[JValue::Object(uri_string.as_ref())])?
                .l()?;
            let resolver = env.call_method(activity, "getContentResolver", "()Landroid/content/ContentResolver;", &[])?.l()?;
            let mode = env.new_string("w")?;
            let output = env
                .call_method(
                    &resolver,
                    "openOutputStream",
                    "(Landroid/net/Uri;Ljava/lang/String;)Ljava/io/OutputStream;",
                    &[JValue::Object(&android_uri), JValue::Object(mode.as_ref())],
                )?
                .l()?;
            if output.is_null() {
                return Err(std::io::Error::other("Android returned no stream for the selected export file").into());
            }
            let copy_result = (|| -> Result<(), Box<dyn std::error::Error>> {
                let buffer = env.new_byte_array(32 * 1024)?;
                let mut bytes = vec![0_u8; 32 * 1024];
                let mut signed = vec![0_i8; 32 * 1024];
                loop {
                    let count = std::io::Read::read(source, &mut bytes)?;
                    if count == 0 {
                        break;
                    }
                    let unsigned = bytes.get(..count).ok_or_else(|| std::io::Error::other("export file returned an invalid chunk"))?;
                    let signed_chunk = signed.get_mut(..count).ok_or_else(|| std::io::Error::other("export buffer returned an invalid chunk"))?;
                    for (target, source) in signed_chunk.iter_mut().zip(unsigned) {
                        *target = *source as i8;
                    }
                    env.set_byte_array_region(&buffer, 0, signed_chunk)?;
                    env.call_method(
                        &output,
                        "write",
                        "([BII)V",
                        &[JValue::Object(buffer.as_ref()), JValue::Int(0), JValue::Int(i32::try_from(count)?)],
                    )?;
                }
                env.call_method(&output, "flush", "()V", &[])?;
                Ok(())
            })();
            if copy_result.is_err() && env.exception_check().unwrap_or(false) {
                let _ = env.exception_clear();
            }
            let close_result = env.call_method(&output, "close", "()V", &[]);
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
}
