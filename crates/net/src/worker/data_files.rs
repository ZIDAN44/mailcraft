//! The pool's work on data files, each on a thread of its own: read into an account, and made
//! smaller.

use super::*;

pub(super) mod compact;
pub(super) mod imports;
