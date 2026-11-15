//! What the `.pst` targets (`pst`, `pst_append`) share: their scratch files and how an input
//! becomes a data file. Not a target of its own: each includes it as a module.

use std::path::PathBuf;
use std::sync::OnceLock;

/// A file in the temp folder, this process's own.
pub fn scratch(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("mailcraft-fuzz-{}-{name}", std::process::id()))
}

/// The good file, made once per process.
static GOOD: OnceLock<Vec<u8>> = OnceLock::new();

/// The data file an input stands for, written to the scratch file `name`: the input itself when
/// it starts like a data file (`!BDN`), otherwise the good file `make` gives with the input read
/// as five-byte patches (a little-endian offset, then the byte to put there). `None` when it
/// can't be written.
pub fn write_input(data: &[u8], make: fn() -> Vec<u8>, name: &str) -> Option<PathBuf> {
    let file = if data.starts_with(b"!BDN") {
        data.to_vec()
    } else {
        let mut file = GOOD.get_or_init(make).clone();
        for p in data.chunks_exact(5) {
            let at = u32::from_le_bytes([p[0], p[1], p[2], p[3]]) as usize % file.len();
            file[at] = p[4];
        }
        file
    };
    let path = scratch(name);
    std::fs::write(&path, &file).ok()?;
    Some(path)
}
