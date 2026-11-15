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

/// Output into a pipe closed early (`mailcraft-cli … | head`) is passed over: `println!` panicked
/// there.
#[test]
fn a_closed_pipe_doesnt_crash() {
    use std::process::{Command, Stdio};
    for _ in 0..5 {
        let mut child =
            Command::new(env!("CARGO_BIN_EXE_mailcraft-cli")).arg("--help").stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
        drop(child.stdout.take());
        let out = child.wait_with_output().unwrap();
        assert!(!String::from_utf8_lossy(&out.stderr).contains("panicked"), "{}", String::from_utf8_lossy(&out.stderr));
        // The help was on its way into the pipe, not an error about the arguments.
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    }
}

/// `--help` and `-h` print the help and succeed, without a data folder: they were refused as
/// options given no value.
#[test]
fn help_is_printed_when_asked_for() {
    for help in ["--help", "-h"] {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_mailcraft-cli")).arg(help).output().unwrap();
        assert!(out.status.success(), "{help}: {}", String::from_utf8_lossy(&out.stderr));
        assert!(String::from_utf8_lossy(&out.stdout).contains("mailcraft-cli [--data DIR] <command>"), "{help}");
        assert!(out.stderr.is_empty(), "{help}: {}", String::from_utf8_lossy(&out.stderr));
    }
}
