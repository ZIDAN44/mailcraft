//! An account's own thread, its `Worker`: connecting and signing in, the jobs it's handed and
//! the changes made offline, syncing, opening and sending mail, the directory, and the server's
//! connection those go over.

use mailcraft_model::job::{Transfer, TransferKind};

use super::*;

pub(super) mod changes;
pub(super) mod connect;
mod directory;
mod free_busy;
mod opening;
pub(super) mod remote;
mod send;
mod server_rules;
pub(super) mod sync;
pub(super) mod transfers;
