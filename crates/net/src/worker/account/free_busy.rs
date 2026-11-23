//! An account's worker: colleagues' free/busy times (issue #140), for their calendars shared
//! with it for those alone: Exchange's availability service, on premises or Online; a CalDAV
//! server's that schedules (issue #242, RFC 6638).

use mailcraft_model::pim::free_busy::FreeBusyAnswer;

use super::*;

/// People asked about in one job at most (a job asks for the calendars shown, a handful).
const PEOPLE_MAX: usize = 200;

impl Worker {
    /// When `people`'s calendars are taken between `from` and `to`: said in
    /// [`Event::FreeBusy`], an answer each, why there's none on each when it failed.
    pub(crate) fn free_busy(&mut self, mut people: Vec<String>, from: i64, to: i64) {
        people.truncate(PEOPLE_MAX);
        // Exchange answers over the mail connection; any other account's calendar server is asked
        // straight away (issue #242), with no mail connection to make or wait for.
        let exchange = self.acc.graph.is_some() || self.acc.ews.is_some();
        let ready = if exchange { self.connected_for_job().map_err(NetError::No) } else { Ok(()) };
        let r = ready.and_then(|()| match &self.conn {
            Some(Conn::Ews(c)) => c.availability(&people, from, to),
            Some(Conn::Graph(c)) => c.availability(&people, from, to),
            _ => self.dav_free_busy(&people, from, to),
        });
        let answers = r.unwrap_or_else(|e| {
            log::info!("{}: free/busy times: {e}", self.acc.id);
            let why = e.to_string();
            people.into_iter().map(|email| FreeBusyAnswer { email, error: Some(why.clone()), ..Default::default() }).collect()
        });
        self.sink.emit(Event::FreeBusy { account: self.id(), from, to, answers });
    }

    /// `people`'s free/busy times from the account's calendar server (issue #242): asked of the
    /// user's scheduling outbox, found from where their calendars are (the engine asks only once
    /// a sync found that).
    fn dav_free_busy(&mut self, people: &[String], from: i64, to: i64) -> Result<Vec<FreeBusyAnswer>> {
        let pim: mailcraft_model::pim::Pim = self.store.load_account_doc(&self.acc.id, mailcraft_model::pim::PIM_DOC).unwrap_or_default();
        let Some(home) = pim.caldav_home.filter(|h| !h.is_empty()) else {
            return Err(NetError::No("this account's calendar server isn't known: its colleagues' free/busy time can't be asked".into()));
        };
        let dav = self.dav_client()?;
        let r = dav.outbox(&home).and_then(|outbox| match outbox {
            Some(outbox) => dav.free_busy(&outbox, &self.acc.email, people, from, to, now()),
            None => Err(NetError::No("this calendar server doesn't give colleagues' free/busy time".into())),
        });
        self.dav_digest = dav.digest();
        r
    }
}
