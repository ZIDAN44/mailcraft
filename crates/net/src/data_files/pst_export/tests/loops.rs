//! A data file added to whose folders go round: a damaged or crafted file's folder listed under
//! itself, twice.

use super::*;
use crate::data_files::pst_bytes::{block_list, seal};
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

/// `bytes` with the four at `at` put to `new`, inside one of the file's `blocks` (`block_list`),
/// whose CRC is put right; `None` when `at` isn't inside a block's data.
fn patched(bytes: &[u8], blocks: &[(u64, (usize, usize))], at: usize, new: [u8; 4]) -> Option<Vec<u8>> {
    let &(_, block) = blocks.iter().find(|(_, (start, size))| *start <= at && at + 4 <= start + size)?;
    let mut out = bytes.to_vec();
    out[at..at + 4].copy_from_slice(&new);
    seal(&mut out, block);
    Some(out)
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
        let blocks = block_list(&bytes);
        let found = places.into_iter().any(|at| {
            let Some(tried) = patched(&bytes, &blocks, at, lp.to_le_bytes()) else { return false };
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
