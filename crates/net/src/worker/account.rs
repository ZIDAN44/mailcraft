//! An account's own thread, its `Worker`: connecting and signing in, the jobs it's handed and
//! the changes made offline, syncing, opening and sending mail, the directory, and the server's
//! connection those go over.

use super::*;

pub(super) mod changes;
pub(super) mod connect;
mod directory;
mod opening;
pub(super) mod remote;
mod send;
pub(super) mod sync;
