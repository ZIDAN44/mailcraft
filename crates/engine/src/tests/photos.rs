//! Issue #223: people's photos, a contact's own (its card's `PHOTO`) and their organisation's,
//! asked of an on-premises Exchange account's server as they're shown.

use mailcraft_model::pim::StoredCard;
use mailcraft_model::vcard::{Card, Typed};

use super::*;

/// A tiny picture's bytes, and as base64.
const PNG: &[u8] = b"\x89PNG\r\n\x1a\nphoto";
const PNG_B64: &str = "iVBORw0KGgpwaG90bw==";

fn card(uid: &str, email: &str, photo_line: &str) -> StoredCard {
    let card = Card {
        uid: uid.into(),
        name: uid.into(),
        emails: vec![Typed { value: email.into(), ..Default::default() }],
        extra: vec![photo_line.to_string()],
        ..Default::default()
    };
    StoredCard { book: "local".into(), card, ..Default::default() }
}

#[test]
fn a_contacts_own_photo_is_read_from_its_card() {
    let mut s = sample_session("photos-cards");
    let p = s.pim_mut("sample");
    p.cards.push(card("v3", "pat@x.example", &format!("PHOTO;ENCODING=b;TYPE=PNG:{PNG_B64}")));
    p.cards.push(card("v4", "lee@x.example", &format!("item1.PHOTO:data:image/png;base64,{PNG_B64}")));
    // At a web address: not fetched. Not a picture, or not base64: none.
    p.cards.push(card("web", "web@x.example", "PHOTO;VALUE=uri:https://pics.example/web.jpg"));
    p.cards.push(card("text", "text@x.example", "PHOTO:data:text/plain;base64,aGk="));
    p.cards.push(card("junk", "junk@x.example", "PHOTO;ENCODING=b:@@@"));
    s.pim_revision += 1;
    s.want_photos(&[]);
    assert_eq!(s.photo("PAT@x.example").as_deref(), Some(PNG));
    assert_eq!(s.photo(" lee@x.example ").as_deref(), Some(PNG));
    for none in ["web@x.example", "text@x.example", "junk@x.example", "nobody@x.example"] {
        assert!(s.photo(none).is_none(), "{none}");
    }
    assert!(!s.take_jobs().iter().any(|j| matches!(j, Job::FetchPhoto { .. })), "no Exchange account: nothing asked");
    // A card's photo taken away: gone once the address book changed.
    s.pim_mut("sample").cards.retain(|c| c.card.uid != "v3");
    s.pim_revision += 1;
    s.want_photos(&[]);
    assert!(s.photo("pat@x.example").is_none());
}

#[test]
fn the_organisations_photo_is_asked_of_exchange_once() {
    let mut s = sample_session("photos-ews");
    let p = json!({"email": "riley@contoso.example", "password": "pw", "username": "CONTOSO\\riley", "ews": "mail.contoso.example"});
    let account = s.run("account.add", &p).unwrap()["account"].as_str().unwrap().to_string();
    s.take_jobs();
    let asked = |s: &mut Session| -> Vec<String> {
        s.take_jobs().into_iter().filter_map(|j| if let Job::FetchPhoto { email, .. } = j { Some(email) } else { None }).collect()
    };
    s.want_photos(&["Ana@Contoso.example".into(), "sam@contoso.example".into(), "not an address".into()]);
    assert_eq!(asked(&mut s), ["ana@contoso.example", "sam@contoso.example"]);
    // Shown again while asked: not asked twice.
    s.want_photos(&["ana@contoso.example".into()]);
    assert!(asked(&mut s).is_empty());
    let rev = s.photos.rev;
    s.apply(Event::Photo { account: account.clone(), email: "ana@contoso.example".into(), photo: Some(PNG.to_vec()), error: None });
    s.apply(Event::Photo { account: account.clone(), email: "sam@contoso.example".into(), photo: None, error: None });
    assert!(s.photos.rev != rev, "the window draws again");
    assert_eq!(s.photo("ana@contoso.example").as_deref(), Some(PNG));
    assert!(s.photo("sam@contoso.example").is_none());
    // Sam has none: not asked again for hours.
    s.want_photos(&["sam@contoso.example".into(), "ana@contoso.example".into()]);
    assert!(asked(&mut s).is_empty());
    s.clock.now += 7 * 3600;
    s.want_photos(&["sam@contoso.example".into()]);
    assert_eq!(asked(&mut s), ["sam@contoso.example"]);
    // A photo nobody asked for isn't kept.
    s.apply(Event::Photo { account: account.clone(), email: "eve@contoso.example".into(), photo: Some(PNG.to_vec()), error: None });
    assert!(s.photo("eve@contoso.example").is_none());
    // Many shown at once: a few asked at a time.
    let many: Vec<String> = (0..50).map(|i| format!("p{i}@contoso.example")).collect();
    s.want_photos(&many);
    assert!(asked(&mut s).len() <= 8);
    // A contact's own photo comes first: the server isn't asked for them.
    s.pim_mut("sample").cards.push(card("zoe", "zoe@contoso.example", &format!("PHOTO;ENCODING=b:{PNG_B64}")));
    s.pim_revision += 1;
    s.apply(Event::Photo { account, email: "p0@contoso.example".into(), photo: None, error: Some("offline".into()) });
    s.want_photos(&["zoe@contoso.example".into()]);
    assert!(!asked(&mut s).contains(&"zoe@contoso.example".to_string()));
    assert_eq!(s.photo("zoe@contoso.example").as_deref(), Some(PNG));
}
