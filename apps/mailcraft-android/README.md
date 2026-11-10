# MailCraft for Android

This is the first native Android app shell for the shared MailCraft engine, network worker, and
egui interface. It packages a Rust `cdylib` as an Android `NativeActivity` through `eframe` and
`cargo-apk`.

The app stores the MailCraft cache in its private Android data directory and uses the production
network worker for account setup and synchronization. Cached mail is available offline. Sign-in
credentials are stored in Android's encrypted SharedPreferences backed by the Android Keystore.
Android document pickers and platform URL/file handling remain to be added.

## Build and run

Install the Android SDK and NDK, add the Rust targets for the device ABIs you want to build, and
install `cargo-apk`. For an arm64 device:

```sh
rustup target add aarch64-linux-android
cargo apk build -p mailcraft-android --target aarch64-linux-android
cargo apk run -p mailcraft-android --target aarch64-linux-android
```

`cargo-apk` reads the application id, SDK levels, and network permissions from this crate's
`Cargo.toml`. Android uses the NDK toolchain supplied by the installed SDK.
