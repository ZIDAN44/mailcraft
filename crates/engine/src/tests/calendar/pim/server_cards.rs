//! Engine tests: a contact kept on a server, edited here.

use mailcraft_model::pim::{LOCAL, StoredCard};
use mailcraft_model::vcard;

use super::*;

const SERVER_CARD: &str = "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:zoe-server\r\nFN:Dr. Zoe Marie Park Jr.\r\nN:Park;Zoe;Marie;Dr.;Jr.\r\n\
item1.TEL;TYPE=WORK,FAX;PREF=1:+1 555 0100\r\nitem1.X-ABLabel:Studio fax\r\n\
EMAIL;TYPE=INTERNET,WORK;PREF=1:zoe@studio.example\r\nTEL;VALUE=uri;TYPE=cell:tel:+49-151-234\r\nEND:VCARD\r\n";

/// A server's contact saved from the contact form with only its title changed goes back with
/// all of its name and its addresses' and numbers' lines as the server wrote them. The form
/// sends the name shown (which cleared the family and given names) and addresses as text (which
/// dropped their kinds), and the lines' other types, preference and group were never kept.
#[test]
fn a_server_contact_saved_from_the_form_keeps_what_it_had() {
    let mut s = sample_session("server-card");
    let card = vcard::parse(SERVER_CARD).remove(0);
    s.pim.get_mut("sample").unwrap().cards.push(StoredCard { book: LOCAL.into(), card, ..Default::default() });
    let form = json!({
        "uid": "zoe-server",
        "name": "Dr. Zoe Marie Park Jr.",
        "emails": ["zoe@studio.example"],
        "phones": [{"value": "+1 555 0100", "kind": "work"}, {"value": "+49-151-234", "kind": "cell"}],
        "title": "Director",
    });
    s.run("contact.update", &form).unwrap();
    let c = &s.pim["sample"].cards.iter().find(|c| c.card.uid == "zoe-server").unwrap().card;
    assert_eq!(c.title, "Director");
    let out = vcard::write(c);
    for line in [
        "FN:Dr. Zoe Marie Park Jr.",
        "N:Park;Zoe;Marie;Dr.;Jr.",
        "item1.TEL;TYPE=WORK,FAX;PREF=1:+1 555 0100",
        "item1.X-ABLabel:Studio fax",
        "EMAIL;TYPE=INTERNET,WORK;PREF=1:zoe@studio.example",
        "TEL;VALUE=uri;TYPE=cell:tel:+49-151-234",
    ] {
        assert!(out.lines().any(|l| l == line), "{line}:\n{out}");
    }
    // Renamed, the old name's parts go with it.
    s.run("contact.update", &json!({"uid": "zoe-server", "name": "Zoe Park"})).unwrap();
    let c = &s.pim["sample"].cards.iter().find(|c| c.card.uid == "zoe-server").unwrap().card;
    assert!(vcard::write(c).lines().any(|l| l == "N:;;;;"), "{}", vcard::write(c));
}
