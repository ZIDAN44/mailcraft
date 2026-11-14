//! A data file added to whose folders go round: a damaged or crafted file's folder listed under
//! itself, twice.

use super::*;
use crate::data_files::scratch::scratch;

/// The node of the folder at `names` below the top of a data file's folders.
fn node_of(path: &Path, names: &[&str]) -> u32 {
    let any = store_of(path).unwrap();
    let mut node = any.ipm_subtree().unwrap();
    for name in names {
        let folder = any.open_folder(node).unwrap();
        node = folder
            .hierarchy_table()
            .unwrap()
            .rows_matrix()
            .map(|row| row.id())
            .find(|id| any.open_folder(*id).is_ok_and(|f| f.properties().display_name().is_ok_and(|n| n == *name)))
            .unwrap();
    }
    node
}

/// The nodes a folder's hierarchy table lists (`None` when the file can't say).
fn listed(path: &Path, node: u32) -> Option<Vec<u32>> {
    let any = store_of(path).ok()?;
    let folder = any.open_folder(node).ok()?;
    Some(folder.hierarchy_table()?.rows_matrix().map(|row| row.id()).collect())
}

/// MS-PST's CRC (CRC-32 from 0, no final XOR).
fn crc(data: &[u8]) -> u32 {
    let mut c = 0u32;
    for b in data {
        c ^= u32::from(*b);
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
        }
    }
    c
}

/// `bytes` with the four at `at` put to `new`, inside a block of a Unicode file whose trailer
/// (its size, then its CRC, in the last 16 of its 64-byte steps) is put right; `None` when `at`
/// isn't inside a block's data.
fn patched(bytes: &[u8], at: usize, new: [u8; 4]) -> Option<Vec<u8>> {
    let u16_at = |i: usize| usize::from(u16::from_le_bytes([bytes[i], bytes[i + 1]]));
    let u32_at = |b: &[u8], i: usize| u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
    let first = (at / 64 + 1) * 64;
    (first..=(at + 8192 + 64).min(bytes.len())).step_by(64).find_map(|end| {
        let trailer = end.checked_sub(16)?;
        let size = (u16_at(trailer) + 16).div_ceil(64) * 64;
        let start = end.checked_sub(size)?;
        let data = start..start + u16_at(trailer);
        let inside = data.start <= at && at + 4 <= data.end && crc(&bytes[data.clone()]) == u32_at(bytes, trailer + 4);
        inside.then(|| {
            let mut out = bytes.to_vec();
            out[at..at + 4].copy_from_slice(&new);
            let sum = crc(&out[data]);
            out[trailer + 4..trailer + 8].copy_from_slice(&sum.to_le_bytes());
            out
        })
    })
}

/// A file whose folder `Loop` lists itself twice, made from one where it held `A` and `B`: of the
/// places each one's node is written, the one that is its row in `Loop`'s list (found by trying
/// them: the writer doesn't encode its files) names `Loop` instead, its block's CRC put right.
fn looping(path: &Path) {
    write::create_empty(path, mailcraft_pst::Format::Unicode).unwrap();
    for name in ["A", "B"] {
        write::create_folder(path, &["Loop", name], "IPF.Note").unwrap();
    }
    let lp = node_of(path, &["Loop"]);
    for child in [node_of(path, &["Loop", "A"]), node_of(path, &["Loop", "B"])] {
        let bytes = std::fs::read(path).unwrap();
        let needle = child.to_le_bytes();
        let places: Vec<usize> = bytes.windows(4).enumerate().filter(|(_, w)| *w == needle).map(|(i, _)| i).collect();
        let found = places.into_iter().any(|at| {
            let Some(tried) = patched(&bytes, at, lp.to_le_bytes()) else { return false };
            std::fs::write(path, &tried).unwrap();
            let ok = listed(path, lp).is_some_and(|l| l.contains(&lp) && !l.contains(&child));
            if !ok {
                std::fs::write(path, &bytes).unwrap();
            }
            ok
        });
        assert!(found, "no row names {child:#x}");
    }
    assert_eq!(listed(path, lp), Some(vec![lp, lp]), "Loop lists itself twice");
}

/// Adding to a data file whose folders go round reads each folder once for the mail it holds:
/// the scan went down every listing again, so a folder listed twice under itself was read 2^64
/// ways and the export never began.
#[test]
fn a_data_file_whose_folders_go_round_is_read_once() {
    let dir = scratch("pst-export-loops");
    let path = dir.join("loop.pst");
    looping(&path);
    let started = std::time::Instant::now();
    assert!(ids_in(&path).unwrap().is_empty());
    assert!(started.elapsed() < std::time::Duration::from_secs(10), "{:?}", started.elapsed());
}
