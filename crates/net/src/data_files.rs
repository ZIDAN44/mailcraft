//! Outlook data files (`.pst`): read into an account on this computer, and written out; and the
//! room left on the drive MailCraft's own are written to.

pub mod msg;
pub mod pst;
#[cfg(test)]
mod pst_bench;
pub mod pst_export;
pub(crate) mod pst_pim;
pub mod space;
