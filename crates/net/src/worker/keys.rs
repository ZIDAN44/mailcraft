//! The pool's work with an account's own keys, each job on a thread of its own: OpenPGP and
//! S/MIME.

use super::*;

pub(super) mod pgp;
pub(super) mod smime;
