//! An account's worker: searching the organisation's directory (issue #9) on
//! on-premises Exchange (EWS) and Exchange Online (Graph, with a sign-in of its own).

use super::*;

impl Worker {
    /// Look `query` up in the account's directory and say who was found.
    pub(super) fn search_directory(&mut self, query: String) {
        let mut consent = false;
        let r = match &self.conn {
            Some(Conn::Ews(c)) => c.search_directory(&query),
            Some(Conn::Graph(_)) => match self.directory_token() {
                Ok(token) => {
                    let base = self.acc.graph.clone().unwrap_or_default();
                    let r = crate::graph::search_directory(&base, &token, &query);
                    if r.as_ref().is_err_and(crate::graph::needs_consent) {
                        // Taken away since (an administrator, or the user in their account's
                        // settings): asked for again.
                        self.directory_token = None;
                        consent = true;
                    }
                    r
                }
                Err(e) => {
                    consent = crate::graph::needs_consent(&e);
                    Err(e)
                }
            },
            Some(_) => Err(NetError::No("this account has no organisation directory".into())),
            None => Err(NetError::Closed("not connected".into())),
        };
        let (people, error) = match r {
            Ok(p) => (p, None),
            Err(e) => {
                log::info!("{}: directory: {e}", self.acc.id);
                (Vec::new(), Some(e.to_string()))
            }
        };
        // Only an account signed in with Microsoft can be given the permission in the browser;
        // one signed in with a token as it is just says why.
        let consent = consent && self.acc.oauth.is_some();
        self.sink.emit(Event::Directory { account: self.id(), query, people, error, consent });
    }

    /// The token to search an Exchange Online directory with: from the directory's own sign-in
    /// (kept apart from the mail's), renewed when it runs out. An account signed in with a
    /// token given as it is uses that one.
    fn directory_token(&mut self) -> Result<String> {
        if self.acc.oauth.is_none() {
            return self.secret();
        }
        if let Some(t) = self.directory_token.as_ref().filter(|t| !t.expired()) {
            return Ok(t.access.clone());
        }
        let Some(refresh) = self.creds.as_ref().and_then(|c| c.get_secret(&self.acc, DIRECTORY_SECRET)) else {
            return Err(NetError::Auth("searching your organisation's directory needs your permission first".into()));
        };
        let p = crate::oauth::directory_provider(&self.acc)
            .ok_or_else(|| NetError::No("signing in with Microsoft isn't set up in this copy of MailCraft".into()))?;
        let t = crate::oauth::refresh(&p, &refresh)?;
        // A new refresh token in its place, as the mail's is.
        if let Some(r) = t.refresh.clone().filter(|r| *r != refresh)
            && let Some(c) = &self.creds
            && let Err(e) = c.set_secret(&self.acc, DIRECTORY_SECRET, &r)
        {
            log::warn!("{}: the directory's new sign-in couldn't be kept: {e}", self.acc.id);
        }
        let access = t.access.clone();
        self.directory_token = Some(t);
        Ok(access)
    }
}
