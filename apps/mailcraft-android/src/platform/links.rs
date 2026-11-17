//! Android links and contact phone numbers.

use super::activity;

fn android_error(message: &str) -> String {
    mailcraft_ui_egui::i18n::t(message).to_owned()
}

/// Open a web or mail link with the Android app registered for its scheme.
pub(crate) fn open_url(app: &winit::platform::android::activity::AndroidApp, url: &str) -> Result<(), String> {
    const MAX_URL_BYTES: usize = 8192;
    let Some((scheme, _)) = url.split_once(':') else { return Err(android_error("This link has no address scheme")) };
    if url.len() > MAX_URL_BYTES {
        return Err(android_error("This link is too long to open"));
    }
    if !["http", "https", "mailto"].iter().any(|allowed| scheme.eq_ignore_ascii_case(allowed)) {
        return Err(android_error("MailCraft can open web links and mail links on Android"));
    }
    use jni::objects::JValue;

    let result = activity::with_activity(app, |env, activity| {
        let operation = (|| -> robius_android_env::Result<()> {
            let action_name = if scheme.eq_ignore_ascii_case("mailto") { "ACTION_SENDTO" } else { "ACTION_VIEW" };
            let action = env.get_static_field("android/content/Intent", action_name, "Ljava/lang/String;")?.l()?;
            let url = env.new_string(url)?;
            let uri =
                env.call_static_method("android/net/Uri", "parse", "(Ljava/lang/String;)Landroid/net/Uri;", &[JValue::Object(url.as_ref())])?.l()?;
            let intent =
                env.new_object("android/content/Intent", "(Ljava/lang/String;Landroid/net/Uri;)V", &[JValue::Object(&action), JValue::Object(&uri)])?;
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
        Ok(Err(robius_android_env::Error::JavaException)) => Err(android_error("No Android app can open this link")),
        Ok(Err(error)) | Err(error) => Err(error.to_string()),
    }
}

/// Open a MailCraft-owned attachment in another app without exposing a raw filesystem path.
pub(crate) fn open_file(app: &winit::platform::android::activity::AndroidApp, path: &std::path::Path) -> Result<(), String> {
    let mime = mime_guess::from_path(path).first_or_octet_stream().to_string();
    open_file_with_mime(app, path, &mime)
}

pub(crate) fn open_file_with_mime(
    app: &winit::platform::android::activity::AndroidApp,
    path: &std::path::Path,
    mime_type: &str,
) -> Result<(), String> {
    let path = path.canonicalize().map_err(|error| error.to_string())?;
    let metadata = std::fs::metadata(&path).map_err(|error| error.to_string())?;
    let parent = path.parent();
    if !metadata.is_file()
        || !parent.is_some_and(|directory| {
            directory.file_name().and_then(std::ffi::OsStr::to_str) == Some("open")
                && directory.parent().is_some_and(|parent| parent.file_name().and_then(std::ffi::OsStr::to_str) == Some("mailcraft"))
        })
    {
        return Err(android_error("No Android app can open this file"));
    }
    let path = path.to_str().ok_or_else(|| android_error("No Android app can open this file"))?;
    let mime = mime_type.chars().take(256).collect::<String>();
    use jni::objects::JValue;

    let result = activity::with_activity(app, |env, activity| {
        let operation = (|| -> robius_android_env::Result<()> {
            let java_path = env.new_string(path)?;
            let uri = env
                .call_static_method(
                    "ai/storyteller/mailcraft/MailCraftFileProvider",
                    "uriFor",
                    "(Landroid/content/Context;Ljava/lang/String;)Landroid/net/Uri;",
                    &[JValue::Object(activity), JValue::Object(java_path.as_ref())],
                )?
                .l()?;
            let action = env.get_static_field("android/content/Intent", "ACTION_VIEW", "Ljava/lang/String;")?.l()?;
            let intent = env.new_object("android/content/Intent", "(Ljava/lang/String;)V", &[JValue::Object(&action)])?;
            let java_mime = env.new_string(&mime)?;
            env.call_method(
                &intent,
                "setDataAndType",
                "(Landroid/net/Uri;Ljava/lang/String;)Landroid/content/Intent;",
                &[JValue::Object(&uri), JValue::Object(java_mime.as_ref())],
            )?;
            let permission = env.get_static_field("android/content/Intent", "FLAG_GRANT_READ_URI_PERMISSION", "I")?.i()?;
            env.call_method(&intent, "addFlags", "(I)Landroid/content/Intent;", &[JValue::Int(permission)])?;
            let resolver = env.call_method(activity, "getContentResolver", "()Landroid/content/ContentResolver;", &[])?.l()?;
            let file_name = path.rsplit('/').next().unwrap_or("attachment");
            let title = env.new_string(file_name)?;
            let clip = env
                .call_static_method(
                    "android/content/ClipData",
                    "newUri",
                    "(Landroid/content/ContentResolver;Ljava/lang/CharSequence;Landroid/net/Uri;)Landroid/content/ClipData;",
                    &[JValue::Object(&resolver), JValue::Object(title.as_ref()), JValue::Object(&uri)],
                )?
                .l()?;
            env.call_method(&intent, "setClipData", "(Landroid/content/ClipData;)Landroid/content/Intent;", &[JValue::Object(&clip)])?;
            let chooser_title = env.new_string(mailcraft_ui_egui::i18n::t("Open"))?;
            let chooser = env
                .call_static_method(
                    "android/content/Intent",
                    "createChooser",
                    "(Landroid/content/Intent;Ljava/lang/CharSequence;)Landroid/content/Intent;",
                    &[JValue::Object(&intent), JValue::Object(chooser_title.as_ref())],
                )?
                .l()?;
            env.call_method(activity, "startActivity", "(Landroid/content/Intent;)V", &[JValue::Object(&chooser)])?;
            Ok(())
        })();
        if operation.is_err() && env.exception_check().unwrap_or(false) {
            let _ = env.exception_clear();
        }
        operation
    });
    match result {
        Ok(Ok(())) => Ok(()),
        Ok(Err(robius_android_env::Error::JavaException)) => Err(android_error("No Android app can open this file")),
        Ok(Err(error)) | Err(error) => Err(error.to_string()),
    }
}

/// Open Android's dialer with a contact's phone number; the user confirms any call there.
pub(crate) fn dial_phone(app: &winit::platform::android::activity::AndroidApp, number: &str) -> Result<(), String> {
    use jni::objects::JValue;

    if number.chars().count() > 128 {
        return Err(android_error("This phone number is too long to open"));
    }
    let (number, extension) = match number.split_once(';') {
        Some((number, parameter)) if parameter.get(..4).is_some_and(|name| name.eq_ignore_ascii_case("ext=")) => {
            let extension = parameter.get(4..).unwrap_or_default();
            if extension.is_empty() || extension.len() > 20 || !extension.bytes().all(|digit| digit.is_ascii_digit()) {
                return Err(android_error("This phone number contains characters Android cannot dial"));
            }
            (number, Some(extension))
        }
        Some(_) => return Err(android_error("This phone number contains characters Android cannot dial")),
        None => (number, None),
    };
    let mut encoded_number = String::with_capacity(number.len());
    let mut has_digit = false;
    for character in number.chars() {
        match character {
            '0'..='9' => {
                has_digit = true;
                encoded_number.push(character);
            }
            '+' if encoded_number.is_empty() => encoded_number.push('+'),
            '*' => encoded_number.push_str("%2A"),
            '#' => encoded_number.push_str("%23"),
            ' ' | '-' | '(' | ')' | '.' => {}
            _ => return Err(android_error("This phone number contains characters Android cannot dial")),
        }
    }
    if !has_digit {
        return Err(android_error("This contact has no dialable phone number"));
    }
    if let Some(extension) = extension {
        encoded_number.push_str(";ext=");
        encoded_number.push_str(extension);
    }

    let result = activity::with_activity(app, |env, activity| {
        let operation = (|| -> robius_android_env::Result<()> {
            let action = env.get_static_field("android/content/Intent", "ACTION_DIAL", "Ljava/lang/String;")?.l()?;
            let uri = env.new_string(format!("tel:{encoded_number}"))?;
            let uri =
                env.call_static_method("android/net/Uri", "parse", "(Ljava/lang/String;)Landroid/net/Uri;", &[JValue::Object(uri.as_ref())])?.l()?;
            let intent =
                env.new_object("android/content/Intent", "(Ljava/lang/String;Landroid/net/Uri;)V", &[JValue::Object(&action), JValue::Object(&uri)])?;
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
        Ok(Err(robius_android_env::Error::JavaException)) => Err(android_error("No Android dialer app can open this number")),
        Ok(Err(error)) | Err(error) => Err(error.to_string()),
    }
}

/// Open a contact's postal address in the Android map app, without granting location access.
pub(crate) fn open_map(app: &winit::platform::android::activity::AndroidApp, address: &str) -> Result<(), String> {
    use jni::objects::{JString, JValue};

    let address = address.trim();
    if address.is_empty() {
        return Err(android_error("This contact has no address to open"));
    }
    if address.len() > 4096 {
        return Err(android_error("This address is too long to open"));
    }

    let result = activity::with_activity(app, |env, activity| {
        let operation = (|| -> robius_android_env::Result<()> {
            let query = env.new_string(address)?;
            let query = env
                .call_static_method("android/net/Uri", "encode", "(Ljava/lang/String;)Ljava/lang/String;", &[JValue::Object(query.as_ref())])?
                .l()?;
            let query = env.get_string(&JString::from(query))?.to_string_lossy().into_owned();
            let uri = env.new_string(format!("geo:0,0?q={query}"))?;
            let uri =
                env.call_static_method("android/net/Uri", "parse", "(Ljava/lang/String;)Landroid/net/Uri;", &[JValue::Object(uri.as_ref())])?.l()?;
            let action = env.get_static_field("android/content/Intent", "ACTION_VIEW", "Ljava/lang/String;")?.l()?;
            let intent =
                env.new_object("android/content/Intent", "(Ljava/lang/String;Landroid/net/Uri;)V", &[JValue::Object(&action), JValue::Object(&uri)])?;
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
        Ok(Err(robius_android_env::Error::JavaException)) => Err(android_error("No Android map app can open this address")),
        Ok(Err(error)) | Err(error) => Err(error.to_string()),
    }
}
