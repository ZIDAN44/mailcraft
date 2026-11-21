//! People's photos (issue #223): Exchange 2013 and later give the photo the organisation has for
//! someone with `GetUserPhoto` (MS-OXWSPHOTO), asked of the account's own server.

use super::*;

/// The largest photo taken (Exchange's 96-pixel ones are a few kilobytes).
pub const MAX_PHOTO: usize = 512 * 1024;

impl Client {
    /// The photo the organisation has for `email`, 96 pixels a side: `None` when it has none.
    /// Exchange answers HTTP 404 then, with no picture; an address it doesn't know is an error
    /// in the answer, taken as none too.
    pub fn user_photo(&self, email: &str) -> Result<Option<Vec<u8>>> {
        let email: String = email.trim().chars().filter(|c| !c.is_control()).take(320).collect();
        if !email.contains('@') {
            return Ok(None);
        }
        let body = format!("<m:GetUserPhoto><m:Email>{}</m:Email><m:SizeRequested>HR96x96</m:SizeRequested></m:GetUserPhoto>", esc(&email));
        let resp = self.post(envelope_as("Exchange2013", &body).into_bytes())?;
        if resp.status == 404 {
            return Ok(None);
        }
        let text = String::from_utf8_lossy(&resp.body);
        let doc = parse(&text);
        let Some(answer) = doc.find("GetUserPhotoResponse") else {
            if let Some(fault) = doc.find("Fault") {
                let why = fault.find("faultstring").map(|f| f.text.clone()).unwrap_or_else(|| "the server refused the request".into());
                return Err(NetError::Protocol(why));
            }
            return Err(NetError::Protocol(format!("Exchange answered HTTP {} without a photo", resp.status)));
        };
        if answer.attr("ResponseClass") == Some("Error") {
            log::info!("photo: {}", answer.text_of("ResponseCode").unwrap_or("refused"));
            return Ok(None);
        }
        let Some(data) = answer.text_of("PictureData").map(str::trim).filter(|d| !d.is_empty()) else { return Ok(None) };
        // Base64 is four characters for three bytes: one too big isn't decoded at all.
        if data.len() / 4 * 3 > MAX_PHOTO {
            return Err(NetError::Protocol("the photo is too big".into()));
        }
        unb64(data).map(Some).ok_or_else(|| NetError::Protocol("the photo isn't base64".into()))
    }
}
