//! Outlook data files (`.pst`): read into an account on this computer, and written out; and the
//! room left on the drive MailCraft's own are written to.

pub mod msg;
pub(crate) mod outlook;
pub mod pst;
#[cfg(test)]
mod pst_bench;
#[cfg(test)]
mod pst_bytes;
pub mod pst_export;
pub(crate) mod pst_pim;
#[cfg(test)]
pub(crate) mod scratch;
pub mod space;
#[cfg(test)]
pub(crate) mod test_data;
