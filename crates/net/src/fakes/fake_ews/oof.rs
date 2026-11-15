//! The fake server's automatic replies: `GetUserOofSettings` and `SetUserOofSettings`, answered
//! outside EWS's response messages, as Exchange does.

use super::*;

/// The automatic replies, as last set.
pub struct Oof {
    /// `Disabled`, `Enabled` or `Scheduled`.
    pub state: String,
    pub start: String,
    pub end: String,
    /// The reply to people inside the organisation, as given (HTML).
    pub inside: String,
    /// The reply to people outside it.
    pub outside: String,
    /// Who outside is answered: `None`, `Known` or `All`.
    pub audience: String,
}

impl Default for Oof {
    fn default() -> Oof {
        Oof {
            state: "Disabled".into(),
            start: String::new(),
            end: String::new(),
            inside: String::new(),
            outside: String::new(),
            audience: "All".into(),
        }
    }
}

impl Data {
    pub(super) fn oof_answer(&self) -> String {
        let Oof { state, start, end, inside, outside, audience } = &self.oof;
        format!(
            "<ResponseMessage ResponseClass=\"Success\"><ResponseCode>NoError</ResponseCode></ResponseMessage>\
             <OofSettings><OofState>{state}</OofState><ExternalAudience>{audience}</ExternalAudience>\
             <Duration><StartTime>{start}</StartTime><EndTime>{end}</EndTime></Duration>\
             <InternalReply><Message>{}</Message></InternalReply><ExternalReply><Message>{}</Message></ExternalReply></OofSettings>",
            esc(inside),
            esc(outside)
        )
    }

    /// The automatic replies set; refused when the reply inside says `refuse`.
    pub(super) fn set_oof(&mut self, op: &El) -> String {
        let s = op.child("UserOofSettings");
        let get = |n: &str| s.and_then(|s| s.find(n)).map(|e| e.text.clone()).unwrap_or_default();
        let msg = s.and_then(|s| s.find("InternalReply")).and_then(|r| r.child("Message")).map(|m| m.text.clone()).unwrap_or_default();
        if msg == "refuse" {
            return "<ResponseMessage ResponseClass=\"Error\"><MessageText>refused</MessageText><ResponseCode>ErrorInvalidParameter</ResponseCode></ResponseMessage>".into();
        }
        let outside = s.and_then(|s| s.find("ExternalReply")).and_then(|r| r.child("Message")).map(|m| m.text.clone()).unwrap_or_default();
        self.oof =
            Oof { state: get("OofState"), start: get("StartTime"), end: get("EndTime"), inside: msg, outside, audience: get("ExternalAudience") };
        "<ResponseMessage ResponseClass=\"Success\"><ResponseCode>NoError</ResponseCode></ResponseMessage>".into()
    }
}
