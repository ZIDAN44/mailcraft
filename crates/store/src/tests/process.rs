//! Tests across processes: this test binary run again as another process, and waiting for the
//! system to let go of what a killed one held.

use std::time::{Duration, Instant};

/// This test binary again, running only `test` (an ignored one that does nothing unless `env`
/// is set): a process that can be killed part-way. What it prints starts on the test harness's
/// own line.
pub(crate) fn child(test: &str, env: &[(&str, String)]) -> std::process::Child {
    command(test, env).spawn().unwrap()
}

/// The command [`child`] starts, for a test to change (where it runs) before it does.
pub(crate) fn command(test: &str, env: &[(&str, String)]) -> std::process::Command {
    let mut cmd = std::process::Command::new(std::env::current_exe().unwrap());
    cmd.args(["--ignored", "--exact", test, "--nocapture", "--test-threads=1"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd
}

/// `open` again while what it's refused is `held` by a process that has ended: the system lets
/// go of a dead process's locks a moment after it ends (Windows), within 10 s.
pub(crate) fn retry_while_held<T, E: std::fmt::Display>(mut open: impl FnMut() -> Result<T, E>, held: impl Fn(&E) -> bool) -> T {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match open() {
            Ok(t) => return t,
            Err(e) if Instant::now() < deadline && held(&e) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => panic!("the dead process's lease wasn't let go: {e}"),
        }
    }
}
