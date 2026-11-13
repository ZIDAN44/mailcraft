//! A backup file of what the browser keeps (issue #83), read: open, and locked. One read is
//! written again and read back the same.
#![no_main]

use libfuzzer_sys::fuzz_target;
use mailcraft_engine::backup;

fuzz_target!(|data: &[u8]| {
    let _ = backup::locked(data);
    let _ = backup::read(data, Some("pw"));
    // What follows the first line as a file's content, so the reader gets past its name.
    let mut file = b"MAILCRAFT-BACKUP\n\x01\x00".to_vec();
    file.extend_from_slice(data);
    if let Ok(b) = backup::read(&file, None) {
        let again = backup::write(&b, None, [0; 28]).and_then(|f| backup::read(&f, None));
        assert_eq!(again.ok().as_ref(), Some(&b));
    }
});
