//! Exchange: on-premises through EWS (signed in with NTLM or Basic), Exchange Online through
//! Microsoft Graph; mail, and calendars and contacts.

pub mod ews;
pub(crate) mod ews_pim;
pub mod graph;
pub mod graph_pim;
pub mod ntlm;
