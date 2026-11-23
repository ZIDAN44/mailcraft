//! `opengl32.dll` isn't a load-time import of `mailcraft.exe`. wgpu's OpenGL backend names it, so
//! every binary that links wgpu loaded it as it started; MailCraft draws with DirectX 12 and never
//! calls it, and a process that has loaded it takes about 60 ms longer to end (as PhotoCraft
//! measured): every start and close of MailCraft, and every test process. `.cargo/config.toml`
//! delay-loads it (it loads on the first OpenGL call, if one ever comes); this reads the built
//! exe's PE import tables to prove it.
#![cfg(all(target_os = "windows", target_env = "msvc", target_pointer_width = "64"))]

/// The built exe names `opengl32.dll` in its delay-load table, never its import table.
#[test]
fn opengl_is_delay_loaded() {
    // The run-time variable first: a run from a nextest archive points it at the extracted exe,
    // while the compile-time path names the folder the archive was built in.
    let path = std::env::var("CARGO_BIN_EXE_mailcraft").unwrap_or_else(|_| env!("CARGO_BIN_EXE_mailcraft").to_string());
    let exe = std::fs::read(path).unwrap();
    let pe = Pe::parse(&exe);
    let imports = pe.dll_names(1, 20, 12);
    let delayed = pe.dll_names(13, 32, 4);
    assert!(imports.iter().any(|d| d == "kernel32.dll"), "the import table parses: {imports:?}");
    assert!(!imports.iter().any(|d| d == "opengl32.dll"), "opengl32.dll is a load-time import: {imports:?}");
    assert!(delayed.iter().any(|d| d == "opengl32.dll"), "opengl32.dll isn't delay-loaded: {delayed:?}");
}

/// Just enough of a PE32+ image to list the DLLs its import directories name.
struct Pe<'a> {
    bytes: &'a [u8],
    /// Virtual address, virtual size and file offset of each section.
    sections: Vec<(u32, u32, u32)>,
    /// Where the optional header's data directories start.
    directories: usize,
}

impl<'a> Pe<'a> {
    /// The PE headers and section table of a PE32+ image (a test: it panics on any other).
    fn parse(bytes: &'a [u8]) -> Self {
        let pe = u32_at(bytes, 0x3c) as usize;
        assert_eq!(&bytes[pe..pe + 4], b"PE\0\0");
        let coff = pe + 4;
        let sections = u16_at(bytes, coff + 2) as usize;
        let optional = coff + 20;
        let optional_size = u16_at(bytes, coff + 16) as usize;
        assert_eq!(u16_at(bytes, optional), 0x20b, "a PE32+ image");
        let table = optional + optional_size;
        let sections = (0..sections)
            .map(|i| {
                let s = table + i * 40;
                (u32_at(bytes, s + 12), u32_at(bytes, s + 8), u32_at(bytes, s + 20))
            })
            .collect();
        Pe { bytes, sections, directories: optional + 112 }
    }

    /// The file offset of relative virtual address `rva`, through the section that holds it.
    fn offset(&self, rva: u32) -> usize {
        let &(va, _, raw) = self.sections.iter().find(|&&(va, size, _)| rva >= va && rva < va + size).unwrap();
        (rva - va + raw) as usize
    }

    /// The DLL names of data directory `index`: an array of `entry`-byte descriptors, ended by an
    /// all-zero one, with the name's RVA at `name_at`.
    fn dll_names(&self, index: usize, entry: usize, name_at: usize) -> Vec<String> {
        let rva = u32_at(self.bytes, self.directories + index * 8);
        if rva == 0 {
            return Vec::new();
        }
        let start = self.offset(rva);
        (0..)
            .map(|i| start + i * entry)
            .take_while(|&d| self.bytes[d..d + entry].iter().any(|&b| b != 0))
            .map(|d| {
                let name = self.offset(u32_at(self.bytes, d + name_at));
                let end = name + self.bytes[name..].iter().position(|&b| b == 0).unwrap();
                String::from_utf8_lossy(&self.bytes[name..end]).to_ascii_lowercase()
            })
            .collect()
    }
}

/// The little-endian `u16` at byte offset `at`.
fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

/// The little-endian `u32` at byte offset `at`.
fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}
