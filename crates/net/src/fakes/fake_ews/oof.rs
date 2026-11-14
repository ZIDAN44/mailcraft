//! The fake server's automatic replies: `GetUserOofSettings` and `SetUserOofSettings`, answered
//! outside EWS's response messages, as Exchange does.

use super::*;

impl Data {
    pub(super) fn oof_answer(&self) -> String {
        let (state, start, end, msg, outside, audience) = self.oof.clone();
        format!(
            "<ResponseMessage ResponseClass=\"Success\"><ResponseCode>NoError</ResponseCode></ResponseMessage>\
             <OofSettings><OofState>{state}</OofState><ExternalAudience>{audience}</ExternalAudience>\
             <Duration><StartTime>{start}</StartTime><EndTime>{end}</EndTime></Duration>\
             <InternalReply><Message>{}</Message></InternalReply><ExternalReply><Message>{}</Message></ExternalReply></OofSettings>",
            esc(&msg),
            esc(&outside)
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
        self.oof = (get("OofState"), get("StartTime"), get("EndTime"), msg, outside, get("ExternalAudience"));
        "<ResponseMessage ResponseClass=\"Success\"><ResponseCode>NoError</ResponseCode></ResponseMessage>".into()
    }
}
