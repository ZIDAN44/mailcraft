//! Servers of the tests' own, in memory: IMAP, CalDAV/CardDAV, EWS, Graph and JMAP; a proxy to
//! put in front of one, for a link that misbehaves; the HTTP they share; the tests' one way of
//! waiting on them; and their random numbers and measurements.

mod exchange_e2e;
pub(crate) mod fake;
pub(crate) mod fake_dav;
pub(crate) mod fake_ews;
pub(crate) mod fake_graph;
pub(crate) mod fake_jmap;
mod http;
pub(crate) mod measure;
pub(crate) mod proxy;
pub(crate) mod random;
mod real;
pub(crate) mod waiting;
