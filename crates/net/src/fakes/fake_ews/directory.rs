//! The fake server's organisation directory (issue #9): Exchange's ambiguous name resolution,
//! with the full contact when it's asked for.

use super::*;

impl Data {
    /// Every name or address with a word starting with the search, up to 100; several are a
    /// warning, none an error.
    pub(super) fn directory_answer(&self, op: &El) -> String {
        let ok = |inner: String| format!("<m:R ResponseClass=\"Success\"><m:ResponseCode>NoError</m:ResponseCode>{inner}</m:R>");
        let err =
            |code: &str| format!("<m:R ResponseClass=\"Error\"><m:MessageText>{code}</m:MessageText><m:ResponseCode>{code}</m:ResponseCode></m:R>");
        let q = op.text_of("UnresolvedEntry").unwrap_or("").trim().to_lowercase();
        let hit = |n: &str, a: &str| {
            let (n, a) = (n.to_lowercase(), a.to_lowercase());
            !q.is_empty() && (a.starts_with(&q) || n.starts_with(&q) || n.split(' ').any(|w| w.starts_with(&q)))
        };
        let full = op.attr("ReturnFullContactData") == Some("true");
        let found: Vec<String> = self
            .directory
            .iter()
            .filter(|(n, a)| hit(n, a))
            .take(100)
            .map(|(n, a)| {
                let shown = if self.directory_aliases { a.split('@').next().unwrap_or(a) } else { n.as_str() };
                let contact = if full {
                    let (given, surname) = n.split_once(' ').unwrap_or((n.as_str(), ""));
                    format!(
                        "<t:Contact><t:DisplayName>{}</t:DisplayName><t:GivenName>{}</t:GivenName><t:Surname>{}</t:Surname>\
                         <t:EmailAddresses><t:Entry Key=\"EmailAddress1\">SMTP:{}</t:Entry></t:EmailAddresses>\
                         <t:ContactSource>ActiveDirectory</t:ContactSource></t:Contact>",
                        esc(n),
                        esc(given),
                        esc(surname),
                        esc(a)
                    )
                } else {
                    String::new()
                };
                format!(
                    "<t:Resolution><t:Mailbox><t:Name>{}</t:Name><t:EmailAddress>{}</t:EmailAddress><t:RoutingType>SMTP</t:RoutingType>\
                     <t:MailboxType>Mailbox</t:MailboxType></t:Mailbox>{contact}</t:Resolution>",
                    esc(shown),
                    esc(a)
                )
            })
            .collect();
        let set =
            format!("<m:ResolutionSet TotalItemsInView=\"{}\" IncludesLastItemInRange=\"true\">{}</m:ResolutionSet>", found.len(), found.concat());
        match found.len() {
            0 => err("ErrorNameResolutionNoResults"),
            1 => ok(set),
            _ => format!(
                "<m:R ResponseClass=\"Warning\"><m:MessageText>Multiple results were found.</m:MessageText>\
                 <m:ResponseCode>ErrorNameResolutionMultipleResults</m:ResponseCode>{set}</m:R>"
            ),
        }
    }
}
