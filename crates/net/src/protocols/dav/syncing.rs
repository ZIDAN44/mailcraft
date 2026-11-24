//! A sync of an account's calendars and address books with its server: what changed here is
//! sent, then what the server has is read.

use super::*;

mod answers;
mod pushing;
mod reading;

/// The longest uid used as a file name as it is.
pub(super) const NAME_KEEP: usize = 100;

/// An event's type, as it's stored.
const CALENDAR_DATA: &str = "text/calendar; charset=utf-8";

/// A file name for a new item (one already on the server keeps the address it has). A short
/// uid of plain characters is the name; any other keeps its readable start and adds `~` and a
/// hash of the whole uid, so two uids alike in what's kept still get names of their own (a plain
/// name never has a `~`).
pub(super) fn resource_name(uid: &str, ext: &str) -> String {
    let plain = |c: char| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.');
    if !uid.is_empty() && uid.len() <= NAME_KEEP && uid.chars().all(plain) {
        return format!("{uid}.{ext}");
    }
    let safe: String = uid.chars().map(|c| if plain(c) { c } else { '-' }).take(NAME_KEEP).collect();
    format!("{safe}~{:016x}.{ext}", mailcraft_store::fnv64(uid))
}

/// What a sync works with: the server, the account, and what the engine last saved.
struct Run<'a> {
    dav: &'a Dav,
    at: Where<'a>,
    pim: &'a Pim,
    now: i64,
    local: mailcraft_model::zone::Zone,
    /// Whether the calendar server schedules meetings itself.
    schedules: bool,
    /// The servers the homes are on: the only ones written to. An item from another one (the
    /// calendar server changed, its old items not yet moved out of the way) isn't sent there
    /// with this server's password.
    origins: Vec<String>,
}

impl Run<'_> {
    fn ours(&self, url: &str) -> bool {
        Url::parse(url).is_ok_and(|u| self.origins.contains(&u.origin()))
    }
}

/// Sync an account's calendars and address books with its server. `pim` is what the engine
/// last saved; nothing is written here: the result is for the engine to merge.
pub fn sync(dav: &Dav, at: &Where, pim: &Pim, now: i64, local: mailcraft_model::zone::Zone) -> Result<DavSync> {
    let mut out = DavSync::default();
    // Where the collections live (looked up once; again a day after finding none, or after
    // signing in again). An address the user gave is followed instead of looking, and what's
    // wrong with it is said.
    let retry = pim.dav_checked.is_none_or(|t| now.saturating_sub(t) > 86_400);
    // A given address that holds nothing is said once and remembered as none, as one that wasn't
    // found is (looked at again a day on, or when it's changed); a refused sign-in stops the sync.
    // Google refusing the access token a permission it lacks (an account signed in before
    // MailCraft asked for calendars) is said the same way: the rest syncs, and signing in again
    // looks again.
    let find = |kind: Kind, given: Option<&str>, errors: &mut Vec<String>| -> Result<Option<String>> {
        let given = given.filter(|g| !g.trim().is_empty());
        let found = match given {
            Some(g) => dav.home_given(kind, g).map(Some),
            None => dav.discover(kind, at.email, at.imap_host),
        };
        match found {
            Err(NetError::No(e)) if given.is_some() => {
                errors.push(e);
                Ok(None)
            }
            Err(e) => match dav.permission_refused(&e) {
                Some(why) => {
                    errors.push(why);
                    Ok(None)
                }
                None => Err(e),
            },
            r => r,
        }
    };
    let mut looked = false;
    out.caldav_home = match &pim.caldav_home {
        Some(h) if !h.is_empty() => Some(h.clone()),
        _ if retry => {
            looked = true;
            find(Kind::Calendars, at.caldav, &mut out.errors)?
        }
        _ => None,
    };
    out.carddav_home = match &pim.carddav_home {
        Some(h) if !h.is_empty() => Some(h.clone()),
        _ if retry => {
            looked = true;
            find(Kind::Contacts, at.carddav, &mut out.errors)?
        }
        _ => None,
    };
    out.checked = now;
    // The day is counted from the last look, not from this sync.
    out.homes_kept = !looked;
    // Whether the calendar server schedules meetings itself (asked at every sync: it's one short
    // request, and a server's settings change). Not known, what was known stays. Google refusing
    // the token the calendars (a later sign-in that granted the contacts alone): said, and they're
    // looked for again as when none were found, the contacts syncing meanwhile; nothing of the
    // calendars is sent or read, as their server isn't the sync's any more.
    if let Some(home) = &out.caldav_home {
        out.auto_schedule = match dav.schedules(home) {
            Ok(b) => Some(b),
            Err(e) => match dav.permission_refused(&e) {
                Some(why) => {
                    out.errors.push(why);
                    out.caldav_home = None;
                    None
                }
                None if matches!(e, NetError::Auth(_)) => return Err(e),
                None => None,
            },
        };
    }
    // Calendars are sent to only with their server known: not after a refusal, though their
    // server may be the contacts' too, nor at the syncs after it, until they're found again.
    let calendars = out.caldav_home.is_some();
    let schedules = out.auto_schedule.unwrap_or(pim.auto_schedule);
    let origins: Vec<String> =
        [&out.caldav_home, &out.carddav_home].into_iter().flatten().filter_map(|h| Url::parse(h).ok()).map(|u| u.origin()).collect();
    let run = Run { dav, at: *at, pim, now, local, schedules, origins };

    // Items whose change the server refused for good: read again below, whatever their etag.
    let mut refetch: Vec<String> = Vec::new();
    run.push_deletions(&mut out, &mut refetch)?;
    if calendars {
        run.push_events(&mut out, &mut refetch)?;
    }
    if let Err(e) = run.push_cards(&mut out) {
        contacts_refused(dav, e, &mut out)?;
    }
    if calendars {
        run.push_tasks(&mut out, &mut refetch)?;
    }
    run.read_calendars(&refetch, &mut out)?;
    if let Err(e) = run.read_books(&mut out) {
        contacts_refused(dav, e, &mut out)?;
    }
    Ok(out)
}

/// A contacts step's error. Google refusing the token its contacts (a sign-in that granted the
/// calendars alone) is said, and the contacts are looked for again as when none were found (a
/// day on, or once signed in again), the calendars syncing meanwhile. Anything else stops the sync.
fn contacts_refused(dav: &Dav, e: NetError, out: &mut DavSync) -> Result<()> {
    let Some(why) = dav.permission_refused(&e) else { return Err(e) };
    if !out.errors.contains(&why) {
        out.errors.push(why);
    }
    // What was read of them before is dropped: none of it is merged.
    (out.carddav_home, out.books) = (None, None);
    out.cards.clear();
    out.synced_books.clear();
    out.unchanged_books.clear();
    Ok(())
}

/// The server's copy of an item comes in place of the one here: it's read again, whatever its
/// etag.
fn theirs_kept(uid: &str, href: String, out: &mut DavSync, refetch: &mut Vec<String>) {
    out.refused.push(uid.to_string());
    refetch.push(href);
}

/// A calendar or address book read, or `None` when it couldn't be (one the user may not read,
/// an error of the server's): said, and what's here of it kept as it is, while the others sync.
/// A refused sign-in, or no connection, is the whole sync's.
fn unreadable<T>(read: Result<T>, name: &str, errors: &mut Vec<String>) -> Result<Option<T>> {
    match read {
        Ok(t) => Ok(Some(t)),
        Err(e @ (NetError::Auth(_) | NetError::Io(_) | NetError::Tls(_) | NetError::Untrusted { .. })) => Err(e),
        Err(e) => {
            errors.push(format!("{name} couldn't be read: {e}"));
            Ok(None)
        }
    }
}
