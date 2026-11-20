//! Colleagues' free/busy times (issue #140) from Exchange's availability service: each person
//! answered apart, their details when shared, why when there are none.

use mailcraft_model::ical::ShowAs;
use mailcraft_model::pim::free_busy::FreeBusyAnswer;

use super::*;

fn at(s: &str) -> i64 {
    mailcraft_jmap::utc_date(s).unwrap()
}

/// Ana shares her details, Ben his times alone; Cleo shares nothing.
fn colleagues() -> Data {
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.busy("ana@contoso.example", "2026-11-03T09:00:00", "2026-11-03T10:30:00", "Busy", Some("Budget review"));
    d.busy("ana@contoso.example", "2026-11-03T13:00:00", "2026-11-03T14:00:00", "Free", Some("Gym"));
    d.busy("ana@contoso.example", "2026-11-04T00:00:00", "2026-11-05T00:00:00", "OOF", Some("Leave"));
    d.busy("ben@contoso.example", "2026-11-03T15:00:00", "2026-11-03T16:00:00", "Tentative", None);
    d.busy("ben@contoso.example", "2026-12-30T15:00:00", "2026-12-30T16:00:00", "Busy", None);
    d.busy("cleo@contoso.example", "2026-11-03T15:00:00", "2026-11-03T16:00:00", "Busy", None);
    d.free_busy_refused.push("cleo@contoso.example".into());
    d
}

fn client(server: &Server) -> crate::ews::Client {
    crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap()
}

#[test]
fn each_persons_times_come_with_what_they_share() {
    let server = serve(colleagues());
    let people: Vec<String> =
        ["ana@contoso.example", "ben@contoso.example", "cleo@contoso.example", "nobody@contoso.example"].map(String::from).into();
    let (from, to) = (at("2026-11-02T00:00:00Z"), at("2026-11-09T00:00:00Z"));
    let answers = client(&server).availability(&people, from, to).unwrap();
    let emails: Vec<&str> = answers.iter().map(|a| a.email.as_str()).collect();
    assert_eq!(emails, ["ana@contoso.example", "ben@contoso.example", "cleo@contoso.example", "nobody@contoso.example"]);

    // Ana: her meeting with its subject, her leave; her free time isn't a time taken.
    let ana = &answers[0];
    assert_eq!(ana.error, None);
    assert_eq!(ana.busy.len(), 2, "{:?}", ana.busy);
    let review = &ana.busy[0];
    assert_eq!((review.start, review.end, review.show_as), (at("2026-11-03T09:00:00Z"), at("2026-11-03T10:30:00Z"), ShowAs::Busy));
    assert_eq!(review.subject, "Budget review");
    assert_eq!(ana.busy[1].show_as, ShowAs::OutOfOffice);
    // Ben: his times alone, none past the window.
    let ben = &answers[1];
    assert_eq!(ben.busy.len(), 1);
    assert_eq!((ben.busy[0].show_as, ben.busy[0].subject.as_str()), (ShowAs::Tentative, ""));
    // Cleo and someone unknown: why, and nothing taken.
    assert_eq!(
        answers[2],
        FreeBusyAnswer { email: people[2].clone(), error: Some("doesn't share their free/busy time".into()), ..Default::default() }
    );
    assert_eq!(answers[3].error.as_deref(), Some("not found in the organisation"));

    // Asked in UTC, its window as given.
    let d = server.data.lock().unwrap();
    let ask = d.availability_asks.last().unwrap();
    assert_eq!((ask.start.as_str(), ask.end.as_str(), ask.bias.as_str()), ("2026-11-02T00:00:00", "2026-11-09T00:00:00", "0"));
}

/// A window past what the service gives is cut, not refused; many people go in requests of 100
/// at most, answered in the order asked.
#[test]
fn a_long_window_and_many_people_are_asked_as_the_server_takes_them() {
    let server = serve(colleagues());
    let mut people: Vec<String> = (0..150).map(|n| format!("p{n}@elsewhere.example")).collect();
    people.insert(120, "ben@contoso.example".into());
    let from = at("2026-11-02T00:00:00Z");
    let answers = client(&server).availability(&people, from, from + 365 * 86_400).unwrap();
    assert_eq!(answers.len(), 151);
    assert!(answers.iter().zip(&people).all(|(a, p)| a.email == *p));
    assert_eq!(answers[120].error, None, "{:?}", answers[120]);
    assert_eq!(answers[120].busy.len(), 1);
    assert!(answers.iter().enumerate().filter(|(i, _)| *i != 120).all(|(_, a)| a.error.is_some()));
    let d = server.data.lock().unwrap();
    let sizes: Vec<usize> = d.availability_asks.iter().map(|a| a.people.len()).collect();
    assert_eq!(sizes, [100, 51]);
    let end = at(&format!("{}Z", d.availability_asks[0].end));
    assert_eq!(end - from, crate::ews::AVAILABILITY_WINDOW_MAX);
}

/// Through the account's worker: the times asked for, said in an event.
#[test]
fn the_worker_says_the_times_asked_for() {
    let server = serve(colleagues());
    let store = Store::open(crate::data_files::scratch::scratch("ews-free-busy")).unwrap();
    let mut pool = crate::worker::Pool::new(store, Arc::new(crate::worker::MemoryCredentials::default()));
    pool.submit(Job::Start { account: account("a1", &server.url), password: Some(Secret(PASSWORD.into())) });
    until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    let (from, to) = (at("2026-11-02T00:00:00Z"), at("2026-11-09T00:00:00Z"));
    pool.submit(Job::FreeBusy { account: "a1".into(), people: vec!["ben@contoso.example".into()], from, to });
    let seen = until(&pool, "the times", |e| matches!(e, Event::FreeBusy { .. }));
    let Some(Event::FreeBusy { account, from: f, to: t, answers }) = seen.into_iter().rev().find(|e| matches!(e, Event::FreeBusy { .. })) else {
        panic!("no answer")
    };
    assert_eq!((account.as_str(), f, t), ("a1", from, to));
    assert_eq!(answers.len(), 1);
    assert_eq!(answers[0].busy.len(), 1);
}

/// What a broken or hostile answer holds is taken as far as it makes sense: never a panic.
#[test]
fn a_broken_answer_is_read_safely() {
    let people = vec!["a@x.example".to_string(), "b@x.example".to_string()];
    for doc in [
        "",
        "<FreeBusyResponseArray/>",
        "<FreeBusyResponseArray><FreeBusyResponse><FreeBusyView><CalendarEventArray><CalendarEvent><StartTime>2026-13-40T99:00:00</StartTime>\
         <EndTime>x</EndTime><BusyType>Busy</BusyType></CalendarEvent></CalendarEventArray></FreeBusyView></FreeBusyResponse></FreeBusyResponseArray>",
        "<FreeBusyResponseArray><FreeBusyResponse><FreeBusyView><CalendarEventArray><CalendarEvent><StartTime>2026-11-03T10:00:00</StartTime>\
         <EndTime>2026-11-03T09:00:00</EndTime><BusyType>Busy</BusyType></CalendarEvent></CalendarEventArray></FreeBusyView></FreeBusyResponse></FreeBusyResponseArray>",
    ] {
        let answers = crate::ews::availability_answers(&parse(doc), &people);
        assert_eq!(answers.len(), 2);
        assert!(answers[1].error.is_some(), "{doc}");
        for b in answers.iter().flat_map(|a| &a.busy) {
            assert!(b.end >= b.start);
        }
    }
}

/// Issue #185: a colleague's work hours come with their times, in their zone as the service
/// writes it (Central Europe: an hour east, two in summer), each period's days and minutes.
#[test]
fn a_colleagues_work_hours_come_with_their_times() {
    let mut d = colleagues();
    d.working_hours.push((
        "ana@contoso.example".into(),
        "<t:WorkingHours><t:TimeZone><t:Bias>-60</t:Bias>\
         <t:StandardTime><t:Bias>0</t:Bias><t:Time>03:00:00</t:Time><t:DayOrder>5</t:DayOrder><t:Month>10</t:Month><t:DayOfWeek>Sunday</t:DayOfWeek></t:StandardTime>\
         <t:DaylightTime><t:Bias>-60</t:Bias><t:Time>02:00:00</t:Time><t:DayOrder>5</t:DayOrder><t:Month>3</t:Month><t:DayOfWeek>Sunday</t:DayOfWeek></t:DaylightTime>\
         </t:TimeZone><t:WorkingPeriodArray>\
         <t:WorkingPeriod><t:DayOfWeek>Monday Tuesday Wednesday Thursday</t:DayOfWeek><t:StartTimeInMinutes>480</t:StartTimeInMinutes><t:EndTimeInMinutes>1020</t:EndTimeInMinutes></t:WorkingPeriod>\
         <t:WorkingPeriod><t:DayOfWeek>Friday</t:DayOfWeek><t:StartTimeInMinutes>480</t:StartTimeInMinutes><t:EndTimeInMinutes>720</t:EndTimeInMinutes></t:WorkingPeriod>\
         </t:WorkingPeriodArray></t:WorkingHours>"
            .into(),
    ));
    // Ben's: the schema's word for Monday to Friday, and a clock change at an hour past any.
    d.working_hours.push((
        "ben@contoso.example".into(),
        "<t:WorkingHours><t:TimeZone><t:Bias>0</t:Bias>\
         <t:StandardTime><t:Bias>0</t:Bias><t:Time>4294967295:99:00</t:Time><t:DayOrder>1</t:DayOrder><t:Month>10</t:Month><t:DayOfWeek>Sunday</t:DayOfWeek></t:StandardTime>\
         <t:DaylightTime><t:Bias>-60</t:Bias><t:Time>02:00:00</t:Time><t:DayOrder>1</t:DayOrder><t:Month>3</t:Month><t:DayOfWeek>Sunday</t:DayOfWeek></t:DaylightTime>\
         </t:TimeZone><t:WorkingPeriodArray>\
         <t:WorkingPeriod><t:DayOfWeek>Weekday</t:DayOfWeek><t:StartTimeInMinutes>540</t:StartTimeInMinutes><t:EndTimeInMinutes>1020</t:EndTimeInMinutes></t:WorkingPeriod>\
         </t:WorkingPeriodArray></t:WorkingHours>"
            .into(),
    ));
    let server = serve(d);
    let people: Vec<String> = ["ana@contoso.example", "ben@contoso.example", "cleo@contoso.example"].map(String::from).into();
    let (from, to) = (at("2026-11-02T00:00:00Z"), at("2026-11-09T00:00:00Z"));
    let answers = client(&server).availability(&people, from, to).unwrap();
    let work = answers[0].work.as_ref().expect("Ana's work hours");
    assert_eq!(work.periods.len(), 2);
    assert_eq!((work.periods[0].days, work.periods[0].start, work.periods[0].end), (0b1111, 480, 1020));
    assert_eq!((work.periods[1].days, work.periods[1].end), (0b1_0000, 720), "Friday mornings");
    // Tuesday 3 November: 08:00 to 17:00 there is 07:00 to 16:00 UTC; Friday ends at 11:00.
    let spans = work.spans(from, to);
    assert!(spans.contains(&(at("2026-11-03T07:00:00Z"), at("2026-11-03T16:00:00Z"))), "{spans:?}");
    assert!(spans.contains(&(at("2026-11-06T07:00:00Z"), at("2026-11-06T11:00:00Z"))), "{spans:?}");
    assert_eq!(spans.len(), 5, "no weekend: {spans:?}");
    let ben = answers[1].work.as_ref().expect("Ben's work hours");
    assert_eq!((ben.periods[0].days, ben.periods[0].start), (0b1_1111, 540), "Weekday: Monday to Friday");
    assert_eq!(ben.spans(from, to).len(), 5);
    assert_eq!(answers[2].work, None, "Cleo's server says none");
}
