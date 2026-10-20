//! The CLI's arguments as the system hands them over.

/// An argument that isn't Unicode (a file name in another encoding) panicked `std::env::args()`
/// before anything ran (WordCraft found); it's read lossily now.
#[test]
fn an_argument_that_isnt_unicode_doesnt_crash() {
    #[cfg(windows)]
    let odd = {
        use std::os::windows::ffi::OsStringExt;
        std::ffi::OsString::from_wide(&[0x66, 0xD800, 0x67])
    };
    #[cfg(unix)]
    let odd = {
        use std::os::unix::ffi::OsStringExt;
        std::ffi::OsString::from_vec(vec![b'f', 0xff, b'g'])
    };
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_mailcraft-cli")).arg("--version").arg(odd).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stdout).starts_with("mailcraft"));
}
