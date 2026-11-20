//! An account's worker: colleagues' free/busy times (issue #140), for their calendars shared
//! with it for those alone: Exchange's availability service, on premises or Online.

use mailcraft_model::pim::free_busy::FreeBusyAnswer;

use super::*;

/// People asked about in one job at most (a job asks for the calendars shown, a handful).
const PEOPLE_MAX: usize = 200;

impl Worker {
    /// When `people`'s calendars are taken between `from` and `to`: said in
    /// [`Event::FreeBusy`], an answer each, why there's none on each when it failed.
    pub(crate) fn free_busy(&mut self, mut people: Vec<String>, from: i64, to: i64) {
        people.truncate(PEOPLE_MAX);
        let r = self.connected_for_job().map_err(NetError::No).and_then(|()| match &self.conn {
            Some(Conn::Ews(c)) => c.availability(&people, from, to),
            Some(Conn::Graph(c)) => c.availability(&people, from, to),
            _ => Err(NetError::No("only an Exchange account can show its colleagues' free/busy time".into())),
        });
        let answers = r.unwrap_or_else(|e| {
            log::info!("{}: free/busy times: {e}", self.acc.id);
            let why = e.to_string();
            people.into_iter().map(|email| FreeBusyAnswer { email, error: Some(why.clone()), ..Default::default() }).collect()
        });
        self.sink.emit(Event::FreeBusy { account: self.id(), from, to, answers });
    }
}
