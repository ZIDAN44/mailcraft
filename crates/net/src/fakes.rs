//! Servers of the tests' own, in memory: IMAP, CalDAV/CardDAV, EWS, Graph and JMAP, and a single
//! sign-on service (OpenID Connect).

pub(crate) mod fake;
pub(crate) mod fake_dav;
pub(crate) mod fake_ews;
pub(crate) mod fake_graph;
pub(crate) mod fake_jmap;
pub(crate) mod fake_sso;
mod real_ews;
mod real_layout;
mod real_pgp;
mod real_servers;
