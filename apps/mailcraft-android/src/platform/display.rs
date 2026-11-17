//! Android display, locale, and system appearance configuration.
use super::activity;

#[derive(Clone, Copy, Default)]
pub(crate) struct SystemInsets {
    pub(crate) left: i32,
    pub(crate) top: i32,
    pub(crate) right: i32,
    pub(crate) bottom: i32,
    pub(crate) ime_visible: bool,
    pub(crate) ime_supported: bool,
}

/// The locale the Android Activity is currently configured to use.
pub(crate) fn language_tag(app: &winit::platform::android::activity::AndroidApp) -> Option<String> {
    let result = activity::with_activity(app, |env, context| activity::system_language_from_context(env, context));
    match result {
        Ok(Ok(tag)) => tag,
        Ok(Err(error)) | Err(error) => {
            log::debug!("MailCraft could not read Android's interface language: {error}");
            None
        }
    }
}

/// Current system bar, display cutout and keyboard insets, in physical pixels.
pub(crate) fn system_insets(app: &winit::platform::android::activity::AndroidApp) -> Option<SystemInsets> {
    use jni::objects::JValue;

    let result = activity::with_activity(app, |env, activity| {
        let operation = (|| -> robius_android_env::Result<Option<SystemInsets>> {
            let window = env.call_method(activity, "getWindow", "()Landroid/view/Window;", &[])?.l()?;
            if window.is_null() {
                return Ok(None);
            }
            let decor = env.call_method(&window, "getDecorView", "()Landroid/view/View;", &[])?.l()?;
            if decor.is_null() {
                return Ok(None);
            }
            let insets = env.call_method(&decor, "getRootWindowInsets", "()Landroid/view/WindowInsets;", &[])?.l()?;
            if insets.is_null() {
                return Ok(None);
            }
            let sdk = env.get_static_field("android/os/Build$VERSION", "SDK_INT", "I")?.i()?;
            if sdk < 30 {
                return Ok(None);
            }
            let system_bars = env.call_static_method("android/view/WindowInsets$Type", "systemBars", "()I", &[])?.i()?;
            let cutout = env.call_static_method("android/view/WindowInsets$Type", "displayCutout", "()I", &[])?.i()?;
            // Keep content clear of the status/navigation bars even while Android briefly
            // hides them during an edge-to-edge transition. The app does not use immersive mode.
            let values =
                env.call_method(&insets, "getInsetsIgnoringVisibility", "(I)Landroid/graphics/Insets;", &[JValue::Int(system_bars | cutout)])?.l()?;
            if values.is_null() {
                return Ok(None);
            }
            // Some edge-to-edge window configurations report a smaller stable top inset than
            // the visible status bar. Respect Android's configured height as the minimum.
            let resources = env.call_method(activity, "getResources", "()Landroid/content/res/Resources;", &[])?.l()?;
            let status_bar_name = env.new_string("status_bar_height")?;
            let dimension_type = env.new_string("dimen")?;
            let android_package = env.new_string("android")?;
            let status_bar_resource = env
                .call_method(
                    &resources,
                    "getIdentifier",
                    "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;)I",
                    &[JValue::Object(status_bar_name.as_ref()), JValue::Object(dimension_type.as_ref()), JValue::Object(android_package.as_ref())],
                )?
                .i()?;
            let status_bar_height = if status_bar_resource > 0 {
                env.call_method(&resources, "getDimensionPixelSize", "(I)I", &[JValue::Int(status_bar_resource)])?.i()?.max(0)
            } else {
                0
            };
            let ime = env.call_static_method("android/view/WindowInsets$Type", "ime", "()I", &[])?.i()?;
            let keyboard = env.call_method(&insets, "getInsets", "(I)Landroid/graphics/Insets;", &[JValue::Int(ime)])?.l()?;
            let system_bottom = env.get_field(&values, "bottom", "I")?.i()?;
            let keyboard_bottom = if keyboard.is_null() { 0 } else { env.get_field(&keyboard, "bottom", "I")?.i()? };
            Ok(Some(SystemInsets {
                left: env.get_field(&values, "left", "I")?.i()?,
                top: env.get_field(&values, "top", "I")?.i()?.max(status_bar_height),
                right: env.get_field(&values, "right", "I")?.i()?,
                // Android resizes this Activity for the IME (`adjustResize`). The egui layout
                // should reserve only the navigation bar here; reserving the keyboard as well
                // would shrink the already-resized viewport a second time.
                bottom: system_bottom,
                ime_visible: keyboard_bottom > 0,
                ime_supported: true,
            }))
        })();
        if operation.is_err() && env.exception_check().unwrap_or(false) {
            let _ = env.exception_clear();
        }
        operation
    });
    match result {
        Ok(Ok(insets)) => insets,
        Ok(Err(error)) | Err(error) => {
            eprintln!("MailCraft could not read Android's screen insets: {error}");
            None
        }
    }
}

/// Match Android's system bars and their icon contrast to the surfaces MailCraft draws beneath them.
pub(crate) fn system_bar_appearance(
    app: &winit::platform::android::activity::AndroidApp,
    status_color: egui::Color32,
    navigation_color: egui::Color32,
) -> Result<(), String> {
    use jni::objects::JValue;

    let result = activity::with_activity(app, |env, activity| {
        let operation = (|| -> robius_android_env::Result<()> {
            let status_argb = i32::from_be_bytes([status_color.a(), status_color.r(), status_color.g(), status_color.b()]);
            let navigation_argb = i32::from_be_bytes([navigation_color.a(), navigation_color.r(), navigation_color.g(), navigation_color.b()]);
            let status_dark_icons = bar_color_is_light(status_color);
            let navigation_dark_icons = bar_color_is_light(navigation_color);
            env.call_method(
                activity,
                "setMailCraftSystemBars",
                "(IIZZ)V",
                &[
                    JValue::Int(status_argb),
                    JValue::Int(navigation_argb),
                    JValue::Bool(u8::from(status_dark_icons)),
                    JValue::Bool(u8::from(navigation_dark_icons)),
                ],
            )?;
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

fn bar_color_is_light(color: egui::Color32) -> bool {
    let linear = |channel: u8| {
        let channel = f32::from(channel) / 255.0;
        if channel <= 0.04045 { channel / 12.92 } else { ((channel + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * linear(color.r()) + 0.7152 * linear(color.g()) + 0.0722 * linear(color.b()) > 0.5
}

pub(crate) fn dark_mode(app: &winit::platform::android::activity::AndroidApp) -> Option<bool> {
    let result = activity::with_activity(app, |env, activity| {
        let operation = (|| -> robius_android_env::Result<Option<bool>> {
            let resources = env.call_method(activity, "getResources", "()Landroid/content/res/Resources;", &[])?.l()?;
            if resources.is_null() {
                return Ok(None);
            }
            let configuration = env.call_method(&resources, "getConfiguration", "()Landroid/content/res/Configuration;", &[])?.l()?;
            if configuration.is_null() {
                return Ok(None);
            }
            let ui_mode = env.get_field(&configuration, "uiMode", "I")?.i()? & 0x30;
            Ok(match ui_mode {
                0x20 => Some(true),
                0x10 => Some(false),
                _ => None,
            })
        })();
        if operation.is_err() && env.exception_check().unwrap_or(false) {
            let _ = env.exception_clear();
        }
        operation
    });
    match result {
        Ok(Ok(dark)) => dark,
        Ok(Err(error)) | Err(error) => {
            eprintln!("MailCraft could not read Android's appearance setting: {error}");
            None
        }
    }
}

/// Whether Android classifies this device as phone-sized; unlike the window's height, this does
/// not change when the software keyboard resizes the app.
pub(crate) fn phone_form_factor(app: &winit::platform::android::activity::AndroidApp) -> Option<bool> {
    let result = activity::with_activity(app, |env, activity| {
        let operation = (|| -> robius_android_env::Result<Option<bool>> {
            let resources = env.call_method(activity, "getResources", "()Landroid/content/res/Resources;", &[])?.l()?;
            if resources.is_null() {
                return Ok(None);
            }
            let configuration = env.call_method(&resources, "getConfiguration", "()Landroid/content/res/Configuration;", &[])?.l()?;
            if configuration.is_null() {
                return Ok(None);
            }
            let smallest_width = env.get_field(&configuration, "smallestScreenWidthDp", "I")?.i()?;
            Ok((smallest_width > 0).then_some(smallest_width < 600))
        })();
        if operation.is_err() && env.exception_check().unwrap_or(false) {
            let _ = env.exception_clear();
        }
        operation
    });
    match result {
        Ok(Ok(phone)) => phone,
        Ok(Err(error)) | Err(error) => {
            log::debug!("MailCraft could not read Android's smallest screen width: {error}");
            None
        }
    }
}

/// Android's text-size preference (`Configuration.fontScale`), applied to the whole UI so
/// controls grow with the text and scroll when they no longer fit.
pub(crate) fn font_scale(app: &winit::platform::android::activity::AndroidApp) -> Option<f32> {
    let result = activity::with_activity(app, |env, activity| {
        let operation = (|| -> robius_android_env::Result<Option<f32>> {
            let resources = env.call_method(activity, "getResources", "()Landroid/content/res/Resources;", &[])?.l()?;
            if resources.is_null() {
                return Ok(None);
            }
            let configuration = env.call_method(&resources, "getConfiguration", "()Landroid/content/res/Configuration;", &[])?.l()?;
            if configuration.is_null() {
                return Ok(None);
            }
            let scale = env.get_field(&configuration, "fontScale", "F")?.f()?;
            Ok(scale.is_finite().then_some(scale.clamp(0.8, 2.0)))
        })();
        if operation.is_err() && env.exception_check().unwrap_or(false) {
            let _ = env.exception_clear();
        }
        operation
    });
    match result {
        Ok(Ok(scale)) => scale,
        Ok(Err(error)) | Err(error) => {
            log::debug!("MailCraft could not read Android's text size: {error}");
            None
        }
    }
}
