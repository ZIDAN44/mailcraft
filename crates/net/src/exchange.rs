//! Exchange: on-premises through EWS (signed in with NTLM, Kerberos or Basic), Exchange Online through
//! Microsoft Graph; mail, and calendars and contacts.

pub mod ews;
pub(crate) mod ews_pim;
pub mod graph;
pub mod graph_pim;
pub mod kerberos;
pub mod ntlm;
pub(crate) mod window;

/// An automatic reply's text as the HTML both transports send it as: the text's own `&`, `<`
/// and `>` escaped (a reply saying `Back <soon>` or `R&D` isn't taken for markup), its lines
/// parted by `<br>`.
pub(crate) fn reply_html(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('\n', "<br>")
}
