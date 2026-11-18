//! The session: File › About › Check for updates, and which release is newer than this build.

/// File › About › Check for updates.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum UpdateCheck {
    #[default]
    NotAsked,
    Checking,
    /// This is the newest (the newest release's tag, when there is one).
    UpToDate(String),
    /// A newer one is out.
    Available(mailcraft_model::job::Release),
    Failed(String),
}

/// The newest release `releases` has for a build of `version` made from commit `build_sha`:
/// `Some` when it's newer. A newer version is. Of one version, its release (`v0.19.0`) comes after
/// its test builds (`v0.19.0-test.15`), and a later test build after an earlier one: newer when
/// this build is one of those (known by its commit). With `stable_only` (a build of a release),
/// test builds aren't offered at all. A build made from no release (a developer's), ahead or the
/// same, is up to date.
pub fn newer_release<'a>(
    version: &str,
    build_sha: Option<&str>,
    releases: &'a [mailcraft_model::job::Release],
    stable_only: bool,
) -> Option<&'a mailcraft_model::job::Release> {
    let parse = |v: &str| -> Option<(u32, u32, u32)> {
        let mut p = v.split('.').map(|n| n.parse::<u32>().ok());
        Some((p.next()??, p.next()??, p.next()??))
    };
    // A version's release after its test builds, the later test build after the earlier.
    let rank = |r: &mailcraft_model::job::Release| parse(&r.version).map(|v| (v, !r.pre, r.build));
    let current = parse(version)?;
    let newest = releases.iter().filter(|r| !(stable_only && r.pre)).filter_map(|r| rank(r).map(|k| (k, r))).max_by_key(|(k, _)| *k)?;
    match newest.0.0.cmp(&current) {
        std::cmp::Ordering::Greater => Some(newest.1),
        std::cmp::Ordering::Less => None,
        std::cmp::Ordering::Equal => {
            let sha = build_sha.map(str::trim).filter(|s| s.len() >= 7)?;
            let mine = releases.iter().find(|r| !r.commit.is_empty() && (r.commit.starts_with(sha) || sha.starts_with(&r.commit)))?;
            rank(mine).is_some_and(|k| k < newest.0).then_some(newest.1)
        }
    }
}
