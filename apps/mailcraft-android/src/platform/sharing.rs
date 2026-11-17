//! Share MailCraft items and files with Android apps.

use super::super::activity;
use super::{create_download_uri, delete_uri, publish_download_uri, write_bytes_to_uri};

const MAX_SHARED_TEXT_BYTES: usize = 1 << 20;
const MAX_SHARE_TITLE_BYTES: usize = 4096;
const MAX_CALENDAR_EVENT_TEXT_BYTES: usize = 128 << 10;
const MAX_SYSTEM_CALENDAR_RECURRENCE_DATES: usize = 512;
const MAX_SYSTEM_CALENDAR_ATTENDEES: usize = 512;
const MAX_SYSTEM_CONTACT_TEXT_BYTES: usize = 128 << 10;
const MAX_SYSTEM_CONTACT_ROWS: usize = 128;
const MAX_SHARED_FILE_BYTES: usize = 25 << 20;
const MAX_SHARED_MESSAGE_BYTES: usize = mailcraft_engine::MAX_MAIL_FILE_BYTES as usize;

fn calendar_recurrence_dates(dates: &[i64], all_day: bool) -> Result<Option<String>, String> {
    if dates.is_empty() {
        return Ok(None);
    }
    let mut formatted = Vec::with_capacity(dates.len());
    for timestamp in dates {
        let Some(date) = chrono::DateTime::<chrono::Utc>::from_timestamp(*timestamp, 0) else {
            return Err(mailcraft_ui_egui::i18n::t("This event has an invalid date for Android calendar").to_owned());
        };
        use chrono::Datelike;
        if !(0..=9999).contains(&date.year()) {
            return Err(mailcraft_ui_egui::i18n::t("This event has an invalid date for Android calendar").to_owned());
        }
        formatted.push(if all_day { date.format("%Y%m%d").to_string() } else { date.format("%Y%m%dT%H%M%SZ").to_string() });
    }
    Ok(Some(formatted.join(",")))
}

/// Open Android's system contact editor without requesting contacts permissions.
pub(crate) fn add_to_system_contacts(
    app: &winit::platform::android::activity::AndroidApp,
    contact: &mailcraft_engine::model::vcard::Card,
) -> Result<(), String> {
    let websites: Vec<&str> = contact
        .extra
        .iter()
        .filter_map(|line| vcard_property_value(line, "URL"))
        .filter_map(android_website)
        .take(MAX_SYSTEM_CONTACT_ROWS.saturating_add(1))
        .collect();
    let nickname = contact.extra.iter().find_map(|line| vcard_property_value(line, "NICKNAME")).map(str::trim).filter(|value| !value.is_empty());
    let anniversary = contact
        .extra
        .iter()
        .find_map(|line| vcard_property_value(line, "ANNIVERSARY"))
        .and_then(mailcraft_engine::model::vcard::birthday_of)
        .filter(|date| !date.starts_with("--"));
    let text_bytes = contact
        .emails
        .iter()
        .map(|value| value.value.len().saturating_add(value.kind.len()))
        .chain(contact.phones.iter().map(|value| value.value.len().saturating_add(value.kind.len())))
        .chain(contact.addresses.iter().map(|address| {
            [address.kind.len(), address.street.len(), address.city.len(), address.region.len(), address.postcode.len(), address.country.len()]
                .into_iter()
                .fold(0usize, usize::saturating_add)
        }))
        .chain([
            contact.name.len(),
            contact.given.len(),
            contact.family.len(),
            contact.org.len(),
            contact.title.len(),
            contact.note.len(),
            contact.birthday.len(),
        ])
        .chain(websites.iter().map(|website| website.len()))
        .chain(nickname.into_iter().map(str::len))
        .chain(anniversary.as_ref().map(String::len))
        .fold(0usize, usize::saturating_add);
    let data_rows = 1usize
        .saturating_add(if !contact.org.trim().is_empty() || !contact.title.trim().is_empty() { 1 } else { 0 })
        .saturating_add(if !contact.note.trim().is_empty() { 1 } else { 0 })
        .saturating_add(if !contact.birthday.trim().is_empty() { 1 } else { 0 })
        .saturating_add(contact.emails.len())
        .saturating_add(contact.phones.len())
        .saturating_add(contact.addresses.iter().filter(|address| !address.is_empty()).count())
        .saturating_add(websites.len())
        .saturating_add(if anniversary.is_some() { 1 } else { 0 })
        .saturating_add(if nickname.is_some() { 1 } else { 0 });
    if text_bytes > MAX_SYSTEM_CONTACT_TEXT_BYTES || data_rows > MAX_SYSTEM_CONTACT_ROWS {
        return Err(mailcraft_ui_egui::i18n::t("This contact is too large to add to Android contacts").to_owned());
    }
    use jni::objects::JValue;

    let result = activity::with_activity(app, |env, activity| {
        let operation = (|| -> robius_android_env::Result<()> {
            let action = env.get_static_field("android/content/Intent", "ACTION_INSERT", "Ljava/lang/String;")?.l()?;
            let contacts_uri = env.get_static_field("android/provider/ContactsContract$Contacts", "CONTENT_URI", "Landroid/net/Uri;")?.l()?;
            let intent = env.new_object(
                "android/content/Intent",
                "(Ljava/lang/String;Landroid/net/Uri;)V",
                &[JValue::Object(&action), JValue::Object(&contacts_uri)],
            )?;
            let contact_type = env.get_static_field("android/provider/ContactsContract$Contacts", "CONTENT_TYPE", "Ljava/lang/String;")?.l()?;
            env.call_method(
                &intent,
                "setDataAndType",
                "(Landroid/net/Uri;Ljava/lang/String;)Landroid/content/Intent;",
                &[JValue::Object(&contacts_uri), JValue::Object(&contact_type)],
            )?;

            let rows = env.new_object("java/util/ArrayList", "()V", &[])?;
            let name = contact.display();
            if !name.trim().is_empty() {
                let row = contact_row(env, "StructuredName")?;
                put_contact_text(env, &row, "StructuredName", "DISPLAY_NAME", &name)?;
                put_optional_contact_text(env, &row, "StructuredName", "GIVEN_NAME", &contact.given)?;
                put_optional_contact_text(env, &row, "StructuredName", "FAMILY_NAME", &contact.family)?;
                add_contact_row(env, &rows, &row)?;
            }
            if !contact.org.trim().is_empty() || !contact.title.trim().is_empty() {
                let row = contact_row(env, "Organization")?;
                put_optional_contact_text(env, &row, "Organization", "COMPANY", &contact.org)?;
                put_optional_contact_text(env, &row, "Organization", "TITLE", &contact.title)?;
                add_contact_row(env, &rows, &row)?;
            }
            for email in &contact.emails {
                if email.value.trim().is_empty() {
                    continue;
                }
                let row = contact_row(env, "Email")?;
                put_contact_text(env, &row, "Email", "ADDRESS", &email.value)?;
                put_contact_type(env, &row, "Email", email_type(&email.kind))?;
                add_contact_row(env, &rows, &row)?;
            }
            for phone in &contact.phones {
                if phone.value.trim().is_empty() {
                    continue;
                }
                let row = contact_row(env, "Phone")?;
                put_contact_text(env, &row, "Phone", "NUMBER", &phone.value)?;
                put_contact_type(env, &row, "Phone", phone_type(&phone.kind))?;
                add_contact_row(env, &rows, &row)?;
            }
            for address in contact.addresses.iter().filter(|address| !address.is_empty()) {
                let row = contact_row(env, "StructuredPostal")?;
                put_optional_contact_text(env, &row, "StructuredPostal", "STREET", &address.street)?;
                put_optional_contact_text(env, &row, "StructuredPostal", "CITY", &address.city)?;
                put_optional_contact_text(env, &row, "StructuredPostal", "REGION", &address.region)?;
                put_optional_contact_text(env, &row, "StructuredPostal", "POSTCODE", &address.postcode)?;
                put_optional_contact_text(env, &row, "StructuredPostal", "COUNTRY", &address.country)?;
                put_contact_type(env, &row, "StructuredPostal", postal_type(&address.kind))?;
                add_contact_row(env, &rows, &row)?;
            }
            for website in &websites {
                let row = contact_row(env, "Website")?;
                put_contact_text(env, &row, "Website", "URL", website)?;
                add_contact_row(env, &rows, &row)?;
            }
            if let Some(nickname) = nickname {
                let row = contact_row(env, "Nickname")?;
                put_contact_text(env, &row, "Nickname", "NAME", nickname)?;
                add_contact_row(env, &rows, &row)?;
            }
            if !contact.note.trim().is_empty() {
                let row = contact_row(env, "Note")?;
                put_contact_text(env, &row, "Note", "NOTE", &contact.note)?;
                add_contact_row(env, &rows, &row)?;
            }
            if let Some(birthday) = mailcraft_engine::model::vcard::birthday_of(&contact.birthday).filter(|date| !date.starts_with("--")) {
                let row = contact_row(env, "Event")?;
                put_contact_text(env, &row, "Event", "START_DATE", &birthday)?;
                put_contact_integer(env, &row, "Event", "TYPE", 3)?;
                add_contact_row(env, &rows, &row)?;
            }
            if let Some(anniversary) = anniversary {
                let row = contact_row(env, "Event")?;
                put_contact_text(env, &row, "Event", "START_DATE", &anniversary)?;
                put_contact_integer(env, &row, "Event", "TYPE", 1)?;
                add_contact_row(env, &rows, &row)?;
            }
            if !env.call_method(&rows, "isEmpty", "()Z", &[])?.z()? {
                let key = env.get_static_field("android/provider/ContactsContract$Intents$Insert", "DATA", "Ljava/lang/String;")?.l()?;
                env.call_method(
                    &intent,
                    "putParcelableArrayListExtra",
                    "(Ljava/lang/String;Ljava/util/ArrayList;)Landroid/content/Intent;",
                    &[JValue::Object(&key), JValue::Object(&rows)],
                )?;
            }
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
        Ok(Err(robius_android_env::Error::JavaException)) => Err(mailcraft_ui_egui::i18n::t("No Android contacts app can add contacts").to_owned()),
        Ok(Err(error)) | Err(error) => Err(error.to_string()),
    }
}

fn contact_row<'local>(env: &mut jni::JNIEnv<'local>, kind: &str) -> jni::errors::Result<jni::objects::JObject<'local>> {
    use jni::objects::JValue;

    let row = env.new_object("android/content/ContentValues", "()V", &[])?;
    let key = env.get_static_field("android/provider/ContactsContract$Data", "MIMETYPE", "Ljava/lang/String;")?.l()?;
    let value =
        env.get_static_field(format!("android/provider/ContactsContract$CommonDataKinds${kind}"), "CONTENT_ITEM_TYPE", "Ljava/lang/String;")?.l()?;
    env.call_method(&row, "put", "(Ljava/lang/String;Ljava/lang/String;)V", &[JValue::Object(&key), JValue::Object(&value)])?;
    Ok(row)
}

fn vcard_property_value<'a>(line: &'a str, expected: &str) -> Option<&'a str> {
    let (mut quoted, mut escaped, mut colon) = (false, false, None);
    for (index, character) in line.char_indices() {
        if escaped {
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else if character == '"' {
            quoted = !quoted;
        } else if character == ':' && !quoted {
            colon = Some(index);
            break;
        }
    }
    let colon = colon?;
    let header = line.get(..colon)?;
    let name = header.split(';').next()?.rsplit('.').next()?;
    if !name.eq_ignore_ascii_case(expected) {
        return None;
    }
    line.get(colon.saturating_add(1)..)
}

fn android_website(value: &str) -> Option<&str> {
    let value = value.trim();
    let has_supported_scheme = value.get(..8).is_some_and(|scheme| scheme.eq_ignore_ascii_case("https://"))
        || value.get(..7).is_some_and(|scheme| scheme.eq_ignore_ascii_case("http://"));
    if !has_supported_scheme {
        return None;
    }
    (!value.chars().any(|character| character.is_whitespace() || character.is_control()) && value.len() <= 4096).then_some(value)
}

fn put_contact_text(env: &mut jni::JNIEnv<'_>, row: &jni::objects::JObject<'_>, kind: &str, field: &str, value: &str) -> jni::errors::Result<()> {
    use jni::objects::JValue;

    let key = env.get_static_field(format!("android/provider/ContactsContract$CommonDataKinds${kind}"), field, "Ljava/lang/String;")?.l()?;
    let value = env.new_string(value)?;
    env.call_method(row, "put", "(Ljava/lang/String;Ljava/lang/String;)V", &[JValue::Object(&key), JValue::Object(value.as_ref())])?;
    Ok(())
}

fn put_optional_contact_text(
    env: &mut jni::JNIEnv<'_>,
    row: &jni::objects::JObject<'_>,
    kind: &str,
    field: &str,
    value: &str,
) -> jni::errors::Result<()> {
    if !value.trim().is_empty() {
        put_contact_text(env, row, kind, field, value)?;
    }
    Ok(())
}

fn put_contact_integer(env: &mut jni::JNIEnv<'_>, row: &jni::objects::JObject<'_>, kind: &str, field: &str, value: i32) -> jni::errors::Result<()> {
    use jni::objects::JValue;

    let key = env.get_static_field(format!("android/provider/ContactsContract$CommonDataKinds${kind}"), field, "Ljava/lang/String;")?.l()?;
    let value = env.call_static_method("java/lang/Integer", "valueOf", "(I)Ljava/lang/Integer;", &[JValue::Int(value)])?.l()?;
    env.call_method(row, "put", "(Ljava/lang/String;Ljava/lang/Integer;)V", &[JValue::Object(&key), JValue::Object(&value)])?;
    Ok(())
}

fn put_contact_type(
    env: &mut jni::JNIEnv<'_>,
    row: &jni::objects::JObject<'_>,
    kind: &str,
    (value, label): (i32, Option<&str>),
) -> jni::errors::Result<()> {
    put_contact_integer(env, row, kind, "TYPE", value)?;
    if let Some(label) = label {
        put_optional_contact_text(env, row, kind, "LABEL", label)?;
    }
    Ok(())
}

fn email_type(kind: &str) -> (i32, Option<&str>) {
    let normalized = kind.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "home" => (1, None),
        "work" => (2, None),
        "mobile" | "cell" => (4, None),
        "" | "other" => (3, None),
        _ => (0, Some(kind.trim())),
    }
}

fn phone_type(kind: &str) -> (i32, Option<&str>) {
    let normalized = kind.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "home" => (1, None),
        "mobile" | "cell" => (2, None),
        "work" => (3, None),
        "" | "other" => (7, None),
        _ => (0, Some(kind.trim())),
    }
}

fn postal_type(kind: &str) -> (i32, Option<&str>) {
    let normalized = kind.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "home" => (1, None),
        "work" => (2, None),
        "" | "other" => (3, None),
        _ => (0, Some(kind.trim())),
    }
}

fn add_contact_row(env: &mut jni::JNIEnv<'_>, rows: &jni::objects::JObject<'_>, row: &jni::objects::JObject<'_>) -> jni::errors::Result<()> {
    use jni::objects::JValue;

    env.call_method(rows, "add", "(Ljava/lang/Object;)Z", &[JValue::Object(row)])?;
    Ok(())
}

pub(crate) fn share_contact(app: &winit::platform::android::activity::AndroidApp, title: &str, vcard: &str) -> Result<(), String> {
    share_text(
        app,
        title,
        "text/x-vcard",
        vcard,
        mailcraft_ui_egui::i18n::t("This contact is too large to share"),
        mailcraft_ui_egui::i18n::t("No Android app can share contacts"),
    )
}

pub(crate) fn share_calendar_event(app: &winit::platform::android::activity::AndroidApp, title: &str, ical: &str) -> Result<(), String> {
    share_text(
        app,
        title,
        "text/calendar",
        ical,
        mailcraft_ui_egui::i18n::t("This event is too large to share"),
        mailcraft_ui_egui::i18n::t("No Android app can share calendar events"),
    )
}

/// Hand one attachment to Android's system share sheet using a read-only Downloads URI.
pub(crate) fn share_file(app: &winit::platform::android::activity::AndroidApp, name: &str, mime_type: &str, bytes: &[u8]) -> Result<(), String> {
    share_file_with_limit(app, name, mime_type, bytes, false)
}

/// Share a complete message, which may exceed the per-attachment limit.
pub(crate) fn share_message(app: &winit::platform::android::activity::AndroidApp, name: &str, mime_type: &str, bytes: &[u8]) -> Result<(), String> {
    share_file_with_limit(app, name, mime_type, bytes, true)
}

fn share_file_with_limit(
    app: &winit::platform::android::activity::AndroidApp,
    name: &str,
    mime_type: &str,
    bytes: &[u8],
    message: bool,
) -> Result<(), String> {
    let max_bytes = if message { MAX_SHARED_MESSAGE_BYTES } else { MAX_SHARED_FILE_BYTES };
    if bytes.len() > max_bytes {
        let error = if message {
            mailcraft_ui_egui::i18n::t("This message is too large to share")
        } else {
            mailcraft_ui_egui::i18n::t("This attachment is too large to share")
        };
        return Err(error.to_owned());
    }
    let name = mailcraft_engine::mime::sanitize_filename(name);
    let mime_type = mime_type.trim();
    let mime_type = if mime_type.len() <= 256
        && !mime_type.chars().any(char::is_control)
        && mime_type.split_once('/').is_some_and(|(major, minor)| {
            let token = |value: &str| {
                !value.is_empty()
                    && value.bytes().all(|byte| {
                        byte.is_ascii_alphanumeric()
                            || matches!(byte, b'!' | b'#' | b'$' | b'%' | b'&' | b'\'' | b'+' | b'-' | b'.' | b'^' | b'_' | b'`' | b'|' | b'~')
                    })
            };
            token(major) && token(minor) && !minor.contains('/')
        }) {
        mime_type.to_owned()
    } else {
        mime_guess::from_path(&name).first_or_octet_stream().to_string()
    };
    let uri = create_download_uri(app, &name, &mime_type)?;
    if let Err(error) = write_bytes_to_uri(app, &uri, bytes).and_then(|()| publish_download_uri(app, &uri)) {
        delete_uri(app, &uri);
        return Err(error);
    }
    use jni::objects::JValue;

    let result = activity::with_activity(app, |env, activity| {
        let operation = (|| -> robius_android_env::Result<()> {
            let action = env.get_static_field("android/content/Intent", "ACTION_SEND", "Ljava/lang/String;")?.l()?;
            let intent = env.new_object("android/content/Intent", "(Ljava/lang/String;)V", &[JValue::Object(&action)])?;
            let mime = env.new_string(&mime_type)?;
            env.call_method(&intent, "setType", "(Ljava/lang/String;)Landroid/content/Intent;", &[JValue::Object(mime.as_ref())])?;
            let uri_string = env.new_string(&uri)?;
            let android_uri = env
                .call_static_method("android/net/Uri", "parse", "(Ljava/lang/String;)Landroid/net/Uri;", &[JValue::Object(uri_string.as_ref())])?
                .l()?;
            let stream = env.get_static_field("android/content/Intent", "EXTRA_STREAM", "Ljava/lang/String;")?.l()?;
            env.call_method(
                &intent,
                "putExtra",
                "(Ljava/lang/String;Landroid/os/Parcelable;)Landroid/content/Intent;",
                &[JValue::Object(&stream), JValue::Object(&android_uri)],
            )?;
            let permission = env.get_static_field("android/content/Intent", "FLAG_GRANT_READ_URI_PERMISSION", "I")?.i()?;
            env.call_method(&intent, "addFlags", "(I)Landroid/content/Intent;", &[JValue::Int(permission)])?;
            let resolver = env.call_method(activity, "getContentResolver", "()Landroid/content/ContentResolver;", &[])?.l()?;
            let title = env.new_string(&name)?;
            let clip = env
                .call_static_method(
                    "android/content/ClipData",
                    "newUri",
                    "(Landroid/content/ContentResolver;Ljava/lang/CharSequence;Landroid/net/Uri;)Landroid/content/ClipData;",
                    &[JValue::Object(&resolver), JValue::Object(title.as_ref()), JValue::Object(&android_uri)],
                )?
                .l()?;
            env.call_method(&intent, "setClipData", "(Landroid/content/ClipData;)Landroid/content/Intent;", &[JValue::Object(&clip)])?;
            let chooser_title =
                env.new_string(if message { mailcraft_ui_egui::i18n::t("Share message") } else { mailcraft_ui_egui::i18n::t("Share attachment") })?;
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
        Ok(Err(robius_android_env::Error::JavaException)) => {
            delete_uri(app, &uri);
            Err(mailcraft_ui_egui::i18n::t("No Android app can share files").to_owned())
        }
        Ok(Err(error)) | Err(error) => {
            delete_uri(app, &uri);
            Err(error.to_string())
        }
    }
}

/// Hand selected attachments to Android's share sheet as one read-only file batch.
pub(crate) fn share_files(app: &winit::platform::android::activity::AndroidApp, files: &[(&str, &str, &[u8])]) -> Result<(), String> {
    const MAX_FILES: usize = 50;
    if files.len() < 2 || files.len() > MAX_FILES {
        return Err(mailcraft_ui_egui::i18n::tfmt("You can share up to {n} attachments at once", &[("n", &MAX_FILES.to_string())]));
    }
    let total = files.iter().fold(0usize, |size, (_, _, bytes)| size.saturating_add(bytes.len()));
    if total > MAX_SHARED_FILE_BYTES {
        return Err(mailcraft_ui_egui::i18n::t("The selected attachments are too large to share").to_owned());
    }
    let mut mime_types = Vec::with_capacity(files.len());
    let mut uris: Vec<String> = Vec::with_capacity(files.len());
    for (name, mime, bytes) in files {
        let name = mailcraft_engine::mime::sanitize_filename(name);
        let mime = mime.trim();
        let valid = mime.len() <= 256
            && !mime.chars().any(char::is_control)
            && mime.split_once('/').is_some_and(|(major, minor)| {
                let token = |value: &str| {
                    !value.is_empty()
                        && value.bytes().all(|byte| {
                            byte.is_ascii_alphanumeric()
                                || matches!(byte, b'!' | b'#' | b'$' | b'%' | b'&' | b'\\' | b'+' | b'-' | b'.' | b'^' | b'_' | b'`' | b'|' | b'~')
                        })
                };
                token(major) && token(minor) && !minor.contains('/')
            });
        let mime = if valid { mime.to_owned() } else { mime_guess::from_path(&name).first_or_octet_stream().to_string() };
        let uri = match create_download_uri(app, &name, &mime) {
            Ok(uri) => uri,
            Err(error) => {
                for uri in &uris {
                    delete_uri(app, uri);
                }
                return Err(error);
            }
        };
        if let Err(error) = write_bytes_to_uri(app, &uri, bytes).and_then(|()| publish_download_uri(app, &uri)) {
            delete_uri(app, &uri);
            for uri in &uris {
                delete_uri(app, uri);
            }
            return Err(error);
        }
        mime_types.push(mime);
        uris.push(uri);
    }
    let share_mime = if mime_types.iter().all(|mime| mime == &mime_types[0]) { mime_types[0].clone() } else { "*/*".to_owned() };
    use jni::objects::JValue;

    let result = activity::with_activity(app, |env, activity| {
        let operation = (|| -> robius_android_env::Result<()> {
            let action = env.get_static_field("android/content/Intent", "ACTION_SEND_MULTIPLE", "Ljava/lang/String;")?.l()?;
            let intent = env.new_object("android/content/Intent", "(Ljava/lang/String;)V", &[JValue::Object(&action)])?;
            let mime = env.new_string(&share_mime)?;
            env.call_method(&intent, "setType", "(Ljava/lang/String;)Landroid/content/Intent;", &[JValue::Object(mime.as_ref())])?;
            let resolver = env.call_method(activity, "getContentResolver", "()Landroid/content/ContentResolver;", &[])?.l()?;
            let title = env.new_string(mailcraft_ui_egui::i18n::t("Share attachment"))?;
            let streams = env.new_object("java/util/ArrayList", "()V", &[])?;
            let mut clip = None;
            for (index, uri) in uris.iter().enumerate() {
                let uri = env.new_string(uri)?;
                let android_uri = env
                    .call_static_method("android/net/Uri", "parse", "(Ljava/lang/String;)Landroid/net/Uri;", &[JValue::Object(uri.as_ref())])?
                    .l()?;
                env.call_method(&streams, "add", "(Ljava/lang/Object;)Z", &[JValue::Object(&android_uri)])?;
                if index == 0 {
                    clip = Some(
                        env.call_static_method(
                            "android/content/ClipData",
                            "newUri",
                            "(Landroid/content/ContentResolver;Ljava/lang/CharSequence;Landroid/net/Uri;)Landroid/content/ClipData;",
                            &[JValue::Object(&resolver), JValue::Object(title.as_ref()), JValue::Object(&android_uri)],
                        )?
                        .l()?,
                    );
                } else if let Some(clip) = clip.as_ref() {
                    let item = env.new_object("android/content/ClipData$Item", "(Landroid/net/Uri;)V", &[JValue::Object(&android_uri)])?;
                    env.call_method(clip, "addItem", "(Landroid/content/ClipData$Item;)V", &[JValue::Object(&item)])?;
                }
            }
            let stream_key = env.get_static_field("android/content/Intent", "EXTRA_STREAM", "Ljava/lang/String;")?.l()?;
            env.call_method(
                &intent,
                "putParcelableArrayListExtra",
                "(Ljava/lang/String;Ljava/util/ArrayList;)Landroid/content/Intent;",
                &[JValue::Object(&stream_key), JValue::Object(&streams)],
            )?;
            if let Some(clip) = clip {
                env.call_method(&intent, "setClipData", "(Landroid/content/ClipData;)Landroid/content/Intent;", &[JValue::Object(&clip)])?;
            }
            let permission = env.get_static_field("android/content/Intent", "FLAG_GRANT_READ_URI_PERMISSION", "I")?.i()?;
            env.call_method(&intent, "addFlags", "(I)Landroid/content/Intent;", &[JValue::Int(permission)])?;
            let chooser = env
                .call_static_method(
                    "android/content/Intent",
                    "createChooser",
                    "(Landroid/content/Intent;Ljava/lang/CharSequence;)Landroid/content/Intent;",
                    &[JValue::Object(&intent), JValue::Object(title.as_ref())],
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
        Ok(Err(robius_android_env::Error::JavaException)) => {
            for uri in &uris {
                delete_uri(app, uri);
            }
            Err(mailcraft_ui_egui::i18n::t("No Android app can share files").to_owned())
        }
        Ok(Err(error)) | Err(error) => {
            for uri in &uris {
                delete_uri(app, uri);
            }
            Err(error.to_string())
        }
    }
}

/// Let the user's calendar app create a new copy without granting MailCraft calendar access.
pub(crate) fn add_to_system_calendar(
    app: &winit::platform::android::activity::AndroidApp,
    title: &str,
    location: &str,
    description: &str,
    start: i64,
    end: i64,
    all_day: bool,
    timezone: Option<&str>,
    rrule: Option<&str>,
    rdates: &[i64],
    exdates: &[i64],
    attendees: &[String],
) -> Result<(), String> {
    if attendees.len() > MAX_SYSTEM_CALENDAR_ATTENDEES || rdates.len().saturating_add(exdates.len()) > MAX_SYSTEM_CALENDAR_RECURRENCE_DATES {
        return Err(mailcraft_ui_egui::i18n::t("This event is too large to add to Android calendar").to_owned());
    }
    let rdates = calendar_recurrence_dates(rdates, all_day)?;
    let exdates = calendar_recurrence_dates(exdates, all_day)?;
    let attendee_bytes = attendees.iter().fold(0usize, |total, email| total.saturating_add(email.len()));
    let text_bytes = title
        .len()
        .saturating_add(location.len())
        .saturating_add(description.len())
        .saturating_add(attendee_bytes)
        .saturating_add(timezone.map_or(0, str::len))
        .saturating_add(rrule.map_or(0, str::len))
        .saturating_add(rdates.as_deref().map_or(0, str::len))
        .saturating_add(exdates.as_deref().map_or(0, str::len));
    if text_bytes > MAX_CALENDAR_EVENT_TEXT_BYTES {
        return Err(mailcraft_ui_egui::i18n::t("This event is too large to add to Android calendar").to_owned());
    }
    let (Some(begin_ms), Some(end_ms)) = (start.checked_mul(1000), end.checked_mul(1000)) else {
        return Err(mailcraft_ui_egui::i18n::t("This event has an invalid date for Android calendar").to_owned());
    };
    if end_ms < begin_ms {
        return Err(mailcraft_ui_egui::i18n::t("This event has an invalid date for Android calendar").to_owned());
    }
    use jni::objects::JValue;

    let result = activity::with_activity(app, |env, activity| {
        let operation = (|| -> robius_android_env::Result<()> {
            let action = env.get_static_field("android/content/Intent", "ACTION_INSERT", "Ljava/lang/String;")?.l()?;
            let intent = env.new_object("android/content/Intent", "(Ljava/lang/String;)V", &[JValue::Object(&action)])?;
            let events_uri = env.get_static_field("android/provider/CalendarContract$Events", "CONTENT_URI", "Landroid/net/Uri;")?.l()?;
            let event_type = env.new_string("vnd.android.cursor.dir/event")?;
            env.call_method(
                &intent,
                "setDataAndType",
                "(Landroid/net/Uri;Ljava/lang/String;)Landroid/content/Intent;",
                &[JValue::Object(&events_uri), JValue::Object(event_type.as_ref())],
            )?;
            for (key, value) in [("title", title), ("eventLocation", location), ("description", description)] {
                let key = env.new_string(key)?;
                let value = env.new_string(value)?;
                env.call_method(
                    &intent,
                    "putExtra",
                    "(Ljava/lang/String;Ljava/lang/String;)Landroid/content/Intent;",
                    &[JValue::Object(key.as_ref()), JValue::Object(value.as_ref())],
                )?;
            }
            let begin_key = env.get_static_field("android/provider/CalendarContract", "EXTRA_EVENT_BEGIN_TIME", "Ljava/lang/String;")?.l()?;
            env.call_method(
                &intent,
                "putExtra",
                "(Ljava/lang/String;J)Landroid/content/Intent;",
                &[JValue::Object(&begin_key), JValue::Long(begin_ms)],
            )?;
            let end_key = env.get_static_field("android/provider/CalendarContract", "EXTRA_EVENT_END_TIME", "Ljava/lang/String;")?.l()?;
            env.call_method(&intent, "putExtra", "(Ljava/lang/String;J)Landroid/content/Intent;", &[JValue::Object(&end_key), JValue::Long(end_ms)])?;
            let all_day_key = env.get_static_field("android/provider/CalendarContract", "EXTRA_EVENT_ALL_DAY", "Ljava/lang/String;")?.l()?;
            env.call_method(
                &intent,
                "putExtra",
                "(Ljava/lang/String;Z)Landroid/content/Intent;",
                &[JValue::Object(&all_day_key), JValue::Bool(u8::from(all_day))],
            )?;
            if let Some(timezone) = timezone.filter(|timezone| !timezone.is_empty()) {
                let key = env.get_static_field("android/provider/CalendarContract$Events", "EVENT_TIMEZONE", "Ljava/lang/String;")?.l()?;
                let value = env.new_string(timezone)?;
                env.call_method(
                    &intent,
                    "putExtra",
                    "(Ljava/lang/String;Ljava/lang/String;)Landroid/content/Intent;",
                    &[JValue::Object(&key), JValue::Object(value.as_ref())],
                )?;
            }
            if let Some(rrule) = rrule {
                let key = env.get_static_field("android/provider/CalendarContract$Events", "RRULE", "Ljava/lang/String;")?.l()?;
                let value = env.new_string(rrule)?;
                env.call_method(
                    &intent,
                    "putExtra",
                    "(Ljava/lang/String;Ljava/lang/String;)Landroid/content/Intent;",
                    &[JValue::Object(&key), JValue::Object(value.as_ref())],
                )?;
            }
            for (field, dates) in [("RDATE", rdates.as_deref()), ("EXDATE", exdates.as_deref())] {
                if let Some(dates) = dates {
                    let key = env.get_static_field("android/provider/CalendarContract$Events", field, "Ljava/lang/String;")?.l()?;
                    let value = env.new_string(dates)?;
                    env.call_method(
                        &intent,
                        "putExtra",
                        "(Ljava/lang/String;Ljava/lang/String;)Landroid/content/Intent;",
                        &[JValue::Object(&key), JValue::Object(value.as_ref())],
                    )?;
                }
            }
            let attendees: Vec<&str> = attendees.iter().map(String::as_str).filter(|email| !email.trim().is_empty()).collect();
            if !attendees.is_empty() {
                let key = env.get_static_field("android/content/Intent", "EXTRA_EMAIL", "Ljava/lang/String;")?.l()?;
                let value = env.new_string(attendees.join(","))?;
                env.call_method(
                    &intent,
                    "putExtra",
                    "(Ljava/lang/String;Ljava/lang/String;)Landroid/content/Intent;",
                    &[JValue::Object(&key), JValue::Object(value.as_ref())],
                )?;
            }
            let chooser_title = env.new_string(mailcraft_ui_egui::i18n::t("Add to Android calendar"))?;
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
        Ok(Err(robius_android_env::Error::JavaException)) => Err(mailcraft_ui_egui::i18n::t("No Android calendar app can add events").to_owned()),
        Ok(Err(error)) | Err(error) => Err(error.to_string()),
    }
}

fn share_text(
    app: &winit::platform::android::activity::AndroidApp,
    title: &str,
    mime_type: &str,
    text: &str,
    too_large: &str,
    no_handler: &str,
) -> Result<(), String> {
    if text.len() > MAX_SHARED_TEXT_BYTES || title.len() > MAX_SHARE_TITLE_BYTES {
        return Err(too_large.to_owned());
    }
    use jni::objects::JValue;

    let result = activity::with_activity(app, |env, activity| {
        let operation = (|| -> robius_android_env::Result<()> {
            let action = env.get_static_field("android/content/Intent", "ACTION_SEND", "Ljava/lang/String;")?.l()?;
            let intent = env.new_object("android/content/Intent", "(Ljava/lang/String;)V", &[JValue::Object(&action)])?;
            let mime = env.new_string(mime_type)?;
            env.call_method(&intent, "setType", "(Ljava/lang/String;)Landroid/content/Intent;", &[JValue::Object(mime.as_ref())])?;
            let extra = env.get_static_field("android/content/Intent", "EXTRA_TEXT", "Ljava/lang/String;")?.l()?;
            let data = env.new_string(text)?;
            env.call_method(
                &intent,
                "putExtra",
                "(Ljava/lang/String;Ljava/lang/CharSequence;)Landroid/content/Intent;",
                &[JValue::Object(&extra), JValue::Object(data.as_ref())],
            )?;
            let chooser_title = env.new_string(title)?;
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
        Ok(Err(robius_android_env::Error::JavaException)) => Err(no_handler.to_owned()),
        Ok(Err(error)) | Err(error) => Err(error.to_string()),
    }
}
