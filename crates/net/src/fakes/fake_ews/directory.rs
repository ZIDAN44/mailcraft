//! The fake server's organisation directory (issue #9): Exchange's ambiguous name resolution,
//! with the full contact when it's asked for.

use super::*;

impl Data {
    /// Every name or address with a word starting with the search, up to 100; several are a
    /// warning, none an error.
    pub(super) fn directory_answer(&self, op: &El) -> String {
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
                         <t:ContactSource>ActiveDirectory</t:ContactSource>{}</t:Contact>",
                        esc(n),
                        esc(given),
                        esc(surname),
                        esc(a),
                        self.directory_titles
                            .iter()
                            .find(|(who, ..)| who.eq_ignore_ascii_case(a))
                            .map(|(_, title, department)| format!(
                                "<t:Department>{}</t:Department><t:JobTitle>{}</t:JobTitle>",
                                esc(department),
                                esc(title)
                            ))
                            .unwrap_or_default()
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

    /// Another mailbox on the server, by its address or its Exchange address (LegacyDN, as
    /// Autodiscover names it on older servers).
    pub(super) fn resolve_mailbox(&self, op: &El) -> String {
        let entry = op.text_of("UnresolvedEntry").unwrap_or("").to_ascii_lowercase();
        match self.shared.iter().find(|b| legacy_dn(&b.address).eq_ignore_ascii_case(&entry) || b.address.eq_ignore_ascii_case(&entry)) {
            Some(b) => ok(format!(
                "<m:ResolutionSet TotalItemsInView=\"1\" IncludesLastItemInRange=\"true\"><t:Resolution><t:Mailbox><t:Name>{}</t:Name>\
                 <t:EmailAddress>{}</t:EmailAddress><t:RoutingType>SMTP</t:RoutingType></t:Mailbox></t:Resolution></m:ResolutionSet>",
                esc(&b.name),
                esc(&b.address)
            )),
            None => err("ErrorNameResolutionNoResults"),
        }
    }
}
