//! Outlook data files (`.pst`): read into an account on this computer, and written out.

pub mod pst;
#[cfg(test)]
mod pst_bench;
pub mod pst_export;
pub(crate) mod pst_pim;
