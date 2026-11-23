//! An organisation's profile as MailCraft starts (issue #245): read once from where this system
//! keeps it (`mailcraft_engine::policy::sources`), and from beside a portable copy, before
//! anything signs in (the organisation's own Microsoft registration is taken then); given to the
//! session once the window's own settings are back, so its defaults and locks go over them. What
//! was read, and what wasn't, goes to the log; File › Info shows it too.

use std::path::Path;

use mailcraft_engine::policy::Organisation;

/// The largest logo read.
const LOGO_MAX: u64 = 2 << 20;

/// The organisation's profile on this computer; `portable` the data folder of a portable copy
/// (its `policies.json` is in the folder beside it, and gives defaults only).
pub(crate) fn read(portable: Option<&Path>) -> Organisation {
    let beside = portable.and_then(Path::parent);
    let o = mailcraft_engine::policy::sources::read(beside);
    // Said even when nothing could be taken: a file refused is what an administrator looks for.
    for why in &o.ignored {
        log::warn!("organisation's profile: {why}");
    }
    if o.is_empty() {
        return o;
    }
    log::info!("organisation's profile from {}", o.sources.join(", "));
    // Its Microsoft registrations, each for the accounts of its domains.
    let apps = o.microsoft_apps().into_iter().map(|(domains, app)| mailcraft_net::oauth::OrganisationApp {
        domains,
        client_id: app.client_id,
        tenant: app.tenant,
    });
    mailcraft_net::oauth::set_organisation_apps(apps.collect());
    o
}

/// The organisation's logo file's bytes, when it names one that can be read (2 MB at most).
pub(crate) fn logo(path: &str) -> Option<Vec<u8>> {
    use std::io::Read;
    let f = std::fs::File::open(path).ok()?;
    let mut bytes = Vec::new();
    f.take(LOGO_MAX + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > LOGO_MAX {
        log::warn!("organisation's logo: {path} is larger than 2 MB");
        return None;
    }
    Some(bytes)
}
