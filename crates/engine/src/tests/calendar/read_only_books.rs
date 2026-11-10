//! A calendar server whose address books, or calendars, the user can't change: new contacts and
//! events stay on this computer.

use super::*;

/// Found on a real Nextcloud (issue #61): a user who never opened its web page has no address book
/// of their own yet, only the organisation's accounts and those recently contacted, both
/// read-only. The one on this computer gave way to them, and new contacts had nowhere to go ("no
/// address book to add to").
#[test]
fn read_only_address_books_leave_the_one_on_this_computer() {
    let mut s = Session::open(Some(temp("read-only-books")), mailcraft_model::dates::Clock::new(NOW, 0));
    let id = s.run("account.add", &json!({"email": "zoe@example.org", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    let home = "https://cloud.example.org/remote.php/dav/addressbooks/users/zoe/";
    let book = |id: &str, name: &str| mailcraft_model::pim::BookInfo {
        id: format!("{home}{id}/"),
        name: name.into(),
        read_only: true,
        ..Default::default()
    };
    let shared = mailcraft_model::pim::CalendarInfo {
        id: "https://cloud.example.org/cal/shared/".into(),
        name: "Shared".into(),
        read_only: true,
        ..Default::default()
    };
    let sync = mailcraft_model::pim::DavSync {
        carddav_home: Some(home.into()),
        books: Some(vec![book("z-server-generated--system", "Accounts"), book("z-app-generated--contactsinteraction--recent", "Recently contacted")]),
        calendars: Some(vec![shared]),
        checked: NOW,
        ..Default::default()
    };
    s.apply(Event::DavSynced { account: id.clone(), sync: Box::new(sync) });
    s.take_jobs();
    s.run("contact.create", &json!({"name": "Ana Lee", "account": id})).unwrap();
    assert_eq!(s.pim[&id].cards.iter().map(|c| c.book.as_str()).collect::<Vec<_>>(), [mailcraft_model::pim::LOCAL]);
    s.run("event.create", &json!({"summary": "Dentist", "start": "2026-10-20T10:00", "account": id})).unwrap();
    assert_eq!(s.pim[&id].events.iter().map(|e| e.calendar.as_str()).collect::<Vec<_>>(), [mailcraft_model::pim::LOCAL]);
    assert!(!s.take_jobs().iter().any(|j| matches!(j, Job::SyncDav { .. })), "nothing for the server");
    // The server's books stay listed beside it, to read.
    assert_eq!(s.pim[&id].books.len(), 3);
    done(s);
}
