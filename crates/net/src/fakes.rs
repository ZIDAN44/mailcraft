//! Servers of the tests' own, in memory: IMAP, CalDAV/CardDAV, EWS, Graph and JMAP; a proxy to
//! put in front of one, for a link that misbehaves; and the tests' one way of waiting on them.

mod exchange_e2e;
pub(crate) mod fake;
pub(crate) mod fake_dav;
pub(crate) mod fake_ews;
pub(crate) mod fake_graph;
pub(crate) mod fake_jmap;
pub(crate) mod proxy;
mod real_ews;
mod real_layout;
mod real_pgp;
mod real_pop;
mod real_servers;
pub(crate) mod waiting;
