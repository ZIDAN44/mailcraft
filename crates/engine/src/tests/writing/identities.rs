//! Engine tests: an account's other addresses (issue #81). One mailbox, read once; a message is
//! written from any of its addresses, with that address's name, signature and place for
//! replies, and the server is told that address sends it.

use mailcraft_model::{Flags, Folder, MessageKey};

use super::*;

/// A session with one account on a server (`zoe@example.org`), its Inbox and Drafts in, two
/// signatures ("Full" for new mail and replies, "Shop"), and no other address yet.
fn one_account(name: &str) -> (Session, String) {
    let mut s = Session::open(Some(temp(name)), mailcraft_model::dates::Clock::new(NOW, 0));
    let id = s.run("account.add", &json!({"email": "zoe@example.org", "password": "pw", "name": "Zoe Park"})).unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    let folder =
        |path: &str, role| Folder { account: id.clone(), path: path.into(), delimiter: Some('/'), role, selectable: true, ..Default::default() };
    s.apply(Event::Folders { account: id.clone(), folders: vec![folder("INBOX", FolderRole::Inbox), folder("Drafts", FolderRole::Drafts)] });
    s.run("account.setSignature", &json!({"account": id, "name": "Full", "signature": "Zoe Park\nStudio"})).unwrap();
    s.run("account.setSignature", &json!({"account": id, "name": "Shop", "signature": "The shop's desk"})).unwrap();
    s.run("account.signatureDefaults", &json!({"account": id, "new": "Full", "reply": "Full"})).unwrap();
    s.take_jobs();
    (s, id)
}

/// Put a message in a folder of the account, its source here, and show the folder.
fn put(s: &mut Session, account: &str, folder: &str, uid: u32, raw: &[u8]) -> MessageKey {
    let store = s.store.clone().unwrap();
    let mut st = store.load_folder(account, folder).unwrap_or_default();
    st.validity = 7;
    let key = MessageKey { account: account.into(), folder: folder.into(), validity: 7, uid };
    st.messages.insert(0, mailcraft_mime::summary_from_full(raw, key.clone(), Flags::default(), 0, NOW).into());
    store.save_folder(account, folder, &st).unwrap();
    store.write_body(&key, raw).unwrap();
    s.apply(Event::FolderChanged { account: account.into(), folder: folder.into() });
    s.run("folder.open", &json!({"account": account, "folder": folder})).unwrap();
    key
}

fn body(s: &Session) -> String {
    s.compose.as_ref().map(|c| c.body.clone()).unwrap_or_default()
}

/// The message handed to the network layer: who the server is told sends it, and its source.
fn sent(s: &mut Session) -> (String, mailcraft_mime::Parsed, Vec<u8>) {
    let jobs = s.take_jobs();
    let Some(Job::Send { from, raw, .. }) = jobs.into_iter().find(|j| matches!(j, Job::Send { .. })) else { panic!("nothing sent") };
    (from, mailcraft_mime::parse(&raw).unwrap(), raw)
}

/// Addresses are added to an account, changed and taken out; what can't be one is refused, and
/// the account doesn't connect again for it.
#[test]
fn an_account_has_other_addresses() {
    let (mut s, id) = one_account("identities-settings");
    let r = s
        .run(
            "account.setAddress",
            &json!({"account": id, "address": " Shop@example.org ", "name": "The Shop", "replyTo": "help@example.org", "signature": "shop"}),
        )
        .unwrap();
    assert_eq!((r["address"].as_str(), r["added"].as_bool()), (Some("Shop@example.org"), Some(true)));
    assert!(s.take_jobs().is_empty(), "the same mailbox: nothing to connect again for");
    // Changed: what isn't given stays.
    let r = s.run("account.setAddress", &json!({"account": id, "address": "shop@example.org", "name": "Shop desk"})).unwrap();
    assert_eq!(r["added"].as_bool(), Some(false));
    let listed = s.run("account.list", &json!({})).unwrap();
    assert_eq!(
        listed[0]["addresses"],
        json!([{"address": "Shop@example.org", "name": "Shop desk", "replyTo": "help@example.org", "signature": "Shop"}])
    );
    s.run("account.setAddress", &json!({"account": id, "address": "shop@example.org", "signature": null, "replyTo": ""})).unwrap();
    let kept = s.account(&id).unwrap().config.identities[0].clone();
    assert_eq!((kept.signature, kept.reply_to.as_str()), (None, ""));
    // Kept with the account's settings.
    let saved = s.store.as_ref().unwrap().load_accounts().unwrap();
    assert_eq!(saved[0].identities.len(), 1);

    // Refused: no address, the account's own, another account's, a signature it hasn't.
    let other = s.run("account.add", &json!({"email": "desk@example.org", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    s.run("account.setAddress", &json!({"account": other, "address": "front@example.org"})).unwrap();
    for bad in ["nobody", "ZOE@example.org", "desk@example.org", "Front@example.org"] {
        assert!(matches!(s.run("account.setAddress", &json!({"account": id, "address": bad})), Err(CmdError::Params(_))), "{bad}");
    }
    assert!(s.run("account.setAddress", &json!({"account": id, "address": "x@example.org", "signature": "Missing"})).is_err());
    assert!(s.run("account.setAddress", &json!({"account": "nobody", "address": "x@example.org"})).is_err());
    assert_eq!(s.account(&id).unwrap().config.identities.len(), 1);
    // Taken out; one that isn't there is said.
    s.run("account.removeAddress", &json!({"account": id, "address": "SHOP@example.org"})).unwrap();
    assert!(s.account(&id).unwrap().config.identities.is_empty());
    assert!(matches!(s.run("account.removeAddress", &json!({"account": id, "address": "shop@example.org"})), Err(CmdError::Params(_))));
    done(s);
}

/// A message written from another of the account's addresses: that address and its name in
/// From, the server told it sends it, replies going where the address says, its signature in
/// place of the account's; the account's own again when From goes back.
#[test]
fn a_message_goes_from_the_address_picked() {
    let (mut s, id) = one_account("identities-send");
    s.run(
        "account.setAddress",
        &json!({"account": id, "address": "shop@example.org", "name": "The Shop", "replyTo": "help@example.org", "signature": "Shop"}),
    )
    .unwrap();
    s.run("account.setAddress", &json!({"account": id, "address": "plain@example.org"})).unwrap();
    s.run("mail.new", &json!({})).unwrap();
    s.compose.as_mut().unwrap().body.insert_str(0, "Hi Ann,");
    assert_eq!(body(&s), "Hi Ann,\n\n-- \nZoe Park\nStudio");
    s.run("compose.update", &json!({"from": "SHOP@example.org", "to": "ann@x.example", "subject": "Your order"})).unwrap();
    assert_eq!(s.compose.as_ref().unwrap().from, "shop@example.org");
    assert_eq!(body(&s), "Hi Ann,\n\n-- \nThe shop's desk", "its own signature");
    // One of no account's, or of another's, isn't picked.
    assert!(matches!(s.run("compose.update", &json!({"from": "else@where.example"})), Err(CmdError::Params(_))));
    assert_eq!(s.compose.as_ref().unwrap().from, "shop@example.org");
    s.run("compose.send", &json!({})).unwrap();
    let (from, p, raw) = sent(&mut s);
    assert_eq!(from, "shop@example.org", "who the server is told sends it");
    assert_eq!((p.from.name.as_str(), p.from.email.as_str()), ("The Shop", "shop@example.org"));
    assert_eq!(p.reply_to.iter().map(|a| a.email.as_str()).collect::<Vec<_>>(), ["help@example.org"]);
    assert!(p.message_id.ends_with("@example.org>") || p.message_id.ends_with("@example.org"), "{}", p.message_id);
    assert!(!String::from_utf8_lossy(&raw).contains("zoe@example.org"), "the account's own address isn't in it");
    let writing = s.compose.as_ref().map_or(0, |c| c.id);
    s.apply(Event::Sent { account: id.clone(), id: writing });

    // An address with no name or signature of its own: the account's; where replies go said
    // in the message wins over the address's.
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"from": "plain@example.org", "to": "ann@x.example", "subject": "Plain"})).unwrap();
    assert_eq!(body(&s), "\n\n-- \nZoe Park\nStudio");
    s.run("compose.update", &json!({"from": "shop@example.org", "replyTo": "me@x.example"})).unwrap();
    assert_eq!(body(&s), "\n\n-- \nThe shop's desk");
    // Back to the account's own: by its address, or by none.
    s.run("compose.update", &json!({"from": "Zoe@example.org"})).unwrap();
    assert_eq!((s.compose.as_ref().unwrap().from.as_str(), body(&s).as_str()), ("", "\n\n-- \nZoe Park\nStudio"));
    s.run("compose.update", &json!({"from": "shop@example.org"})).unwrap();
    s.run("compose.send", &json!({})).unwrap();
    let (from, p, _) = sent(&mut s);
    assert_eq!((from.as_str(), p.reply_to[0].email.as_str()), ("shop@example.org", "me@x.example"));
    done(s);
}

/// Mail sent to one of the account's other addresses is answered from it, and that address
/// isn't answered as if it were someone else's; a draft written from it is carried on from it;
/// taken out of the account, what's being written from it goes from the account's own.
#[test]
fn replies_and_drafts_keep_the_address() {
    let (mut s, id) = one_account("identities-reply");
    s.run("account.setAddress", &json!({"account": id, "address": "shop@example.org", "name": "The Shop", "signature": "Shop"})).unwrap();
    let to_shop = b"From: Ann <ann@x.example>\r\nTo: Shop <shop@example.org>\r\nCc: Ben <ben@x.example>\r\nSubject: Order\r\nMessage-ID: <1@x.example>\r\n\r\nWhere is it?\r\n";
    let to_both =
        b"From: Ann <ann@x.example>\r\nTo: zoe@example.org, shop@example.org\r\nSubject: Both\r\nMessage-ID: <2@x.example>\r\n\r\nHello.\r\n";
    let first = put(&mut s, &id, "INBOX", 1, to_shop);
    let second = put(&mut s, &id, "INBOX", 2, to_both);
    // Sent to the account's addresses: both are "to me".
    s.run("list.filter", &json!({"value": "toMe"})).unwrap();
    assert_eq!(s.list.len(), 2);
    s.run("list.filter", &json!({"value": "all"})).unwrap();

    s.run("list.open", &json!({"key": first.to_id()})).unwrap();
    s.run("mail.replyAll", &json!({})).unwrap();
    let c = s.compose.clone().unwrap();
    assert_eq!(c.from, "shop@example.org", "answered from where it was sent");
    assert!(c.body.contains("-- \nThe shop's desk"), "{:?}", c.body);
    assert!(
        c.to.contains("ann@x.example") && c.cc.contains("ben@x.example") && !format!("{}{}", c.to, c.cc).contains("shop@example.org"),
        "{} / {}",
        c.to,
        c.cc
    );
    // Kept as a draft: From it; carried on from it.
    // Refused by the server: the address it was written from is named as the likely cause.
    s.run("compose.send", &json!({})).unwrap();
    s.take_jobs();
    s.apply(Event::SendFailed { account: id.clone(), id: c.id, error: "553 Sender address rejected".into() });
    let said = s.compose.as_ref().unwrap().error.clone().unwrap();
    assert!(said.contains("553 Sender address rejected") && said.contains("written from shop@example.org"), "{said}");
    s.compose.as_mut().unwrap().body.insert_str(0, "On its way.");
    s.run("compose.close", &json!({"save": true})).unwrap();
    let jobs = s.take_jobs();
    let draft = jobs.iter().find_map(|j| match j {
        Job::SaveDraft { raw, folder, .. } | Job::KeepDraft { raw, folder, .. } => Some((raw.clone(), folder.clone())),
        _ => None,
    });
    let Some((raw, folder)) = draft else { panic!("no draft: {jobs:?}") };
    assert_eq!(mailcraft_mime::parse(&raw).unwrap().from.email, "shop@example.org");
    let draft = put(&mut s, &id, &folder, 5, &raw);
    s.run("draft.edit", &json!({"key": draft.to_id()})).unwrap();
    assert_eq!(s.compose.as_ref().unwrap().from, "shop@example.org");
    // The address taken out: it goes from the account's own, with the account's signature.
    s.run("account.removeAddress", &json!({"account": id, "address": "shop@example.org"})).unwrap();
    let c = s.compose.clone().unwrap();
    assert_eq!(c.from, "");
    s.run("compose.close", &json!({"save": false})).unwrap();

    // Sent to the account's own address as well: answered from that.
    s.run("account.setAddress", &json!({"account": id, "address": "shop@example.org", "signature": "Shop"})).unwrap();
    // Found in review. Its signature renamed, the address's follows: left with the old name,
    // its mail got the account's own, and the address couldn't be changed any more.
    s.run("account.setSignature", &json!({"account": id, "was": "Shop", "name": "Shop desk", "signature": "The shop's desk"})).unwrap();
    assert_eq!(s.account(&id).unwrap().config.identities[0].signature.as_deref(), Some("Shop desk"));
    s.run("account.setAddress", &json!({"account": id, "address": "shop@example.org", "name": "The Shop"})).unwrap();
    // Everyone put in Bcc from that address: it's the one in To, not the account's main one.
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"from": "shop@example.org", "to": "ann@x.example; ben@x.example"})).unwrap();
    s.run("compose.hideRecipients", &json!({})).unwrap();
    let c = s.compose.clone().unwrap();
    assert!(c.to.contains("shop@example.org") && !c.to.contains("zoe@example.org"), "{}", c.to);
    s.run("compose.close", &json!({"save": false})).unwrap();
    // One's own mail from that address, followed up: from it again.
    let own = b"From: The Shop <shop@example.org>\r\nTo: Ann <ann@x.example>\r\nSubject: Sent\r\nMessage-ID: <3@example.org>\r\n\r\nOn its way.\r\n";
    let third = put(&mut s, &id, "INBOX", 3, own);
    // (The first change heard is taken for the answer to the read mark made above.)
    s.apply(Event::FolderChanged { account: id.clone(), folder: "INBOX".into() });
    s.run("list.open", &json!({"key": third.to_id()})).unwrap();
    s.run("mail.replyAll", &json!({})).unwrap();
    let c = s.compose.clone().unwrap();
    assert_eq!(c.from, "shop@example.org");
    assert!(c.to.contains("ann@x.example"), "{}", c.to);
    s.run("compose.close", &json!({"save": false})).unwrap();
    // The signature removed: the address has none of its own.
    s.run("account.setSignature", &json!({"account": id, "name": "Shop desk", "signature": ""})).unwrap();
    assert_eq!(s.account(&id).unwrap().config.identities[0].signature, None);
    s.run("folder.open", &json!({"account": id, "folder": "INBOX"})).unwrap();
    s.run("list.open", &json!({"key": second.to_id()})).unwrap();
    s.run("mail.reply", &json!({})).unwrap();
    assert_eq!(s.compose.as_ref().unwrap().from, "");
    done(s);
}

/// The addresses a JMAP server keeps for the account become its other addresses when asked
/// for: not its own, not one it has (kept as it's set here), not another account's, nor what
/// isn't an address. Only a JMAP server is asked.
#[test]
fn a_jmap_servers_identities_become_the_accounts_addresses() {
    let (mut s, id) = one_account("identities-found");
    assert!(matches!(s.run("account.findAddresses", &json!({"account": id})), Err(CmdError::Params(_))), "an IMAP server keeps none");
    let mut config = s.account(&id).unwrap().config.clone();
    config.jmap = Some("https://jmap.example.org/session".into());
    s.update_account(config, None).unwrap();
    s.run("account.setAddress", &json!({"account": id, "address": "shop@example.org", "name": "Kept as set"})).unwrap();
    let other = s.run("account.add", &json!({"email": "desk@example.org", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    s.take_jobs();
    s.run("account.findAddresses", &json!({"account": id})).unwrap();
    assert!(matches!(s.take_jobs().as_slice(), [Job::FindIdentities { account }] if account == &id));
    let identity = |email: &str, name: &str, reply_to: &str| mailcraft_model::Identity {
        email: email.into(),
        name: name.into(),
        reply_to: reply_to.into(),
        signature: None,
    };
    let found = vec![
        identity("zoe@example.org", "Zoe Park", ""),
        identity("SHOP@example.org", "From the server", ""),
        identity("sales@example.org", "Sales", "team@example.org"),
        identity("zoe@alias.example", "Zoe Park", ""),
        identity("desk@example.org", "Desk", ""),
        identity("not an address", "", ""),
    ];
    s.apply(Event::IdentitiesFound { account: id.clone(), found, error: None });
    let kept: Vec<(String, String, String)> =
        s.account(&id).unwrap().config.identities.iter().map(|i| (i.email.clone(), i.name.clone(), i.reply_to.clone())).collect();
    assert_eq!(
        kept,
        [
            ("shop@example.org".to_string(), "Kept as set".to_string(), String::new()),
            ("sales@example.org".to_string(), "Sales".to_string(), "team@example.org".to_string()),
            ("zoe@alias.example".to_string(), String::new(), String::new()),
        ],
        "the account's name is left for the address to follow"
    );
    assert!(s.notices.last().is_some_and(|n| !n.error && n.text.starts_with("2 addresses found")), "{:?}", s.notices.last());
    assert!(s.account(&other).unwrap().config.identities.is_empty());
    // Asked again: nothing new; a server that can't be asked says why.
    s.apply(Event::IdentitiesFound { account: id.clone(), found: vec![identity("sales@example.org", "Sales", "")], error: None });
    assert!(s.notices.last().is_some_and(|n| n.text.contains("no other addresses")));
    s.apply(Event::IdentitiesFound { account: id.clone(), found: Vec::new(), error: Some("the account is offline".into()) });
    assert!(s.notices.last().is_some_and(|n| n.error && n.text.contains("offline")));
    assert_eq!(s.account(&id).unwrap().config.identities.len(), 3);
    done(s);
}

/// Found in review. An account removed took what was being written from it along, unsaid; and
/// a draft saved on a server, opened again from Drafts while it was still being written,
/// opened a second time (sent from one, the other was sent again or kept as a draft of it).
#[test]
fn what_is_being_written_is_neither_lost_nor_opened_twice() {
    let (mut s, id) = one_account("identities-kept");
    // Saved on the server: the copy there is this one, by its Message-ID.
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"to": "ann@x.example", "subject": "Plan", "body": "Half written"})).unwrap();
    let cid = s.compose.as_ref().unwrap().id;
    s.compose.as_mut().unwrap().draft_id = "<plan@example.org>".into();
    let saved = b"From: Zoe Park <zoe@example.org>\r\nTo: ann@x.example\r\nSubject: Plan\r\nMessage-ID: <plan@example.org>\r\n\r\nHalf written\r\n";
    let draft = put(&mut s, &id, "Drafts", 9, saved);
    let again = s.run("draft.edit", &json!({"key": draft.to_id()})).unwrap();
    assert_eq!(again["compose"], json!(cid), "the one being written, brought forward");
    assert_eq!(s.all_composes().count(), 1);

    // The account removed, with another that can send: the message goes From that one.
    let other =
        s.run("account.add", &json!({"email": "ben@example.net", "password": "pw", "name": "Ben"})).unwrap()["account"].as_str().unwrap().to_string();
    assert_eq!(s.compose.as_ref().unwrap().account, id);
    s.run("account.remove", &json!({"account": id})).unwrap();
    let c = s.compose.clone().expect("still being written");
    assert_eq!((c.account.as_str(), c.body.contains("Half written"), c.draft, c.draft_id.as_str()), (other.as_str(), true, None, ""));
    // The last account removed: still there, to be sent once there's one to send from.
    s.run("account.remove", &json!({"account": other})).unwrap();
    assert!(s.compose.as_ref().is_some_and(|c| c.body.contains("Half written")));
    assert!(s.run("compose.send", &json!({})).is_err());
    // MailCraft closed like that: kept all the same, and being written again when it opens
    // (with no Drafts folder to go to, it was let go).
    assert_eq!(s.keep_drafts(), 1);
    let again = Session::open(s.store.clone(), mailcraft_model::dates::Clock::new(NOW, 0));
    assert!(again.all_composes().any(|c| c.body.contains("Half written") && c.window), "back when MailCraft opens");
    assert!(Session::open(s.store.clone(), mailcraft_model::dates::Clock::new(NOW, 0)).all_composes().next().is_none(), "once");
    let third =
        s.run("account.add", &json!({"email": "cy@example.net", "password": "pw", "name": "Cy"})).unwrap()["account"].as_str().unwrap().to_string();
    assert_eq!(s.compose.as_ref().unwrap().account, third);
    done(s);
}
