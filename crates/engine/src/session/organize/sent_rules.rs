//! Rules for mail the user sends (issue #202), as Outlook's rules can be for messages sent: they
//! run as each message's copy reaches Sent. That's mail new in Sent past what was there when it
//! was last looked at, and the copy of each message sent from here, wherever it's first seen: a
//! Sent folder its first copy made (a new mailbox) has no mark yet, so nothing in it is new.

use super::*;

impl Session {
    /// A message about to go from a server account (`raw`, as sent): its copy in Sent is looked
    /// for, when there are rules for mail sent.
    pub(crate) fn note_sending(&mut self, account: &str, raw: &[u8]) {
        if !self.rules.iter().any(|r| r.sent && r.enabled) {
            return;
        }
        let id = mailcraft_mime::written_message_id(raw);
        if !id.is_empty() {
            self.sent_here.insert((account.to_string(), String::new(), id));
        }
    }

    /// Mail new in Sent (`arrived`, past its mark) and the copies there of mail sent from here:
    /// the rules for mail sent run on them, and nothing is announced.
    pub(in crate::session) fn sent_arrived(&mut self, account: &str, folder: &str, arrived: Vec<Summary>) {
        // No rule for mail sent: nothing to look for (a Sent of 50,000 was read through at each
        // of its changes).
        if !self.rules.iter().any(|r| r.sent && r.enabled) {
            return;
        }
        let mut mail = arrived;
        // Copies looked for only while mail sent from here waits for them.
        let waiting = self.sent_here.iter().any(|(a, _, _)| a == account);
        let here: Vec<Summary> = if !waiting {
            Vec::new()
        } else {
            self.folder_state(account, folder)
                .messages
                .iter()
                .filter(|m| !m.message_id.is_empty() && !mail.iter().any(|a| a.key == m.key))
                .map(|m| Summary::clone(m))
                .collect()
        };
        for m in here {
            if self.sent_here.remove(&(account.to_string(), String::new(), m.message_id.clone())) {
                mail.push(m);
            }
        }
        // Those above the mark were sent from here too, or elsewhere: either way not looked for.
        for m in &mail {
            self.sent_here.remove(&(account.to_string(), String::new(), m.message_id.clone()));
        }
        self.run_rules(&mail);
    }
}
