//! `mailcraft-cli send`.

use super::*;

/// `send`: a new message made, signed as the account signs, sent, and its copy in Sent Items
/// waited for.
pub(crate) fn send(s: &mut Session, store: &Store, args: &Args) -> Result<(), String> {
    let account = pick_account(s, args)?;
    call(s, "mail.new", &json!({"account": account}))?;
    let body = args.opt("--body").unwrap_or_default();
    // The account's signature stays under the text, as in a new message in the app (it
    // was dropped): `--signature NAME` picks another of its signatures, `none` none.
    let signature = args.opt("--signature");
    let below = if signature.as_deref() == Some("none") { String::new() } else { s.compose.as_ref().map(|c| c.body.clone()).unwrap_or_default() };
    let to = args.opt("--to").ok_or("--to is required")?;
    let (cc, subject) = (args.opt("--cc").unwrap_or_default(), args.opt("--subject").unwrap_or_default());
    call(s, "compose.update", &json!({"to": to, "cc": cc, "subject": subject, "body": format!("{body}{below}")}))?;
    match signature.as_deref() {
        Some("none") => {}
        // Put in again: its look and pictures, which the text above doesn't carry.
        None if below.trim().is_empty() => {}
        None => drop(call(s, "compose.insertSignature", &json!({}))?),
        Some(name) => drop(call(s, "compose.insertSignature", &json!({"name": name}))?),
    }
    // Sent from another account (From changed): its signature takes the first one's place.
    // Or from one of an account's addresses (issue #81), given as the address.
    let account = match args.opt("--from") {
        Some(from) if from.contains('@') => {
            let of = s.accounts.iter().find(|a| a.config.has_address(&from)).map(|a| a.config.id.clone());
            let of = of.ok_or_else(|| format!("{from} isn't an address of any account"))?;
            call(s, "compose.update", &json!({"account": of, "from": from}))?;
            of
        }
        Some(from) => {
            call(s, "compose.update", &json!({"account": from}))?;
            from
        }
        None => account,
    };
    if let Some(f) = args.opt("--attach") {
        call(s, "compose.attach", &json!({"path": f}))?;
    }
    let params = json!({"allowEmptySubject": true, "checked": args.flag("--checked")});
    if s.account(&account).is_some_and(|a| a.is_local()) {
        print(&call(s, "compose.send", &params)?);
        return Ok(());
    }
    let mut pool = start(s, store, &account)?;
    let sending = call(s, "compose.send", &params)?;
    // Held to be looked at again (an address that looks like another, many people shown
    // each other's addresses): said at once. It waited out the three minutes unsaid.
    if let Some(checks) = sending.get("check") {
        return Err(format!("not sent: it's held to be checked first ({checks}); send it with --checked once it's been looked at"));
    }
    // Being signed or encrypted first, then sent under the same number: waited for by
    // that (it was sent, and "timed out" said three minutes later, for a script to
    // send it again).
    let me = sending["sending"].as_u64().or_else(|| sending["protecting"].as_u64());
    // This message's word only: one waiting in the Outbox goes out as the account connects,
    // and its word ended the wait before this one was sent (issue #19's testing).
    drive(s, &mut pool, Duration::from_secs(180), |e| match e {
        Event::Sent { id, .. } if Some(*id) == me => Some(Ok(())),
        Event::SendFailed { id, error, .. } if Some(*id) == me => Some(Err(error.clone())),
        Event::Protected { id, error: Some(error), .. } if Some(*id) == me => Some(Err(format!("it couldn't be signed or encrypted: {error}"))),
        // Kept in the Outbox to go later (it waited there unsaid until the time ran out).
        Event::Queued { id, error, .. } if Some(*id) == me => Some(Err(format!("not sent yet, kept in the Outbox: {error}"))),
        _ => None,
    })?;
    outln!("sent");
    // The copy in Sent Items is filed after the message goes (issue #19: the CLI stopped
    // before, and none was kept): wait for it, or for word it couldn't be.
    let mut sent = s.account(&account).and_then(|a| a.folder(mailcraft_engine::model::FolderRole::Sent)).map(|f| f.path.clone());
    let filed = drive(s, &mut pool, Duration::from_secs(60), |e| match e {
        Event::Folders { folders, .. } => {
            if let Some(f) = folders.iter().find(|f| f.role == mailcraft_engine::model::FolderRole::Sent) {
                sent = Some(f.path.clone());
            }
            None
        }
        Event::FolderChanged { folder, .. } if sent.as_deref() == Some(folder.as_str()) => Some(Ok(())),
        Event::SentCopyNotKept { error, .. } => Some(Err(error.clone())),
        _ => None,
    });
    match (filed, s.account(&account).and_then(|a| a.folder(mailcraft_engine::model::FolderRole::Sent))) {
        (Ok(()), Some(f)) => outln!("a copy is in {}", mailcraft_engine::model::utf7::decode(&f.path)),
        (Ok(()), None) => {}
        (Err(e), _) => errln!("note: {e}"),
    }
    Ok(())
}
