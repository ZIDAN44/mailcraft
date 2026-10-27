//! Mail and calendar protocols, each over a stream: IMAP, POP, SMTP, ManageSieve, JMAP, and
//! CalDAV/CardDAV.

pub mod dav;
pub mod imap;
pub mod jmap;
pub mod pop;
pub mod sieve;
pub mod smtp;
pub mod web_calendar;
