//! Servers of the tests' own, in memory: IMAP, CalDAV/CardDAV, EWS, Graph and JMAP.

pub(crate) mod fake;
pub(crate) mod fake_dav;
pub(crate) mod fake_ews;
pub(crate) mod fake_graph;
pub(crate) mod fake_jmap;
mod real_servers;
