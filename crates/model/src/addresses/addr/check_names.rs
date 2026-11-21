//! Check Names (issue #211), as Outlook's: each name typed in a recipient box that isn't an
//! address is looked up among the people the user knows and the organisation's directory. One
//! person is put in as `Name <address>`; several, or none, are left as typed and said, for the
//! user to pick from or to fix.

use super::*;

/// The most people offered for one name.
pub const MAX_PEOPLE: usize = 50;

/// A name that wasn't put in: those it could be (several to pick from; none found).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Unresolved {
    /// As it was typed.
    pub typed: String,
    pub people: Vec<EmailAddress>,
}

/// A recipient box checked: its text, how many names were put in, and those that weren't.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Checked {
    pub field: String,
    pub resolved: usize,
    pub unresolved: Vec<Unresolved>,
}

/// `field` with each entry that isn't an address looked up with `find` (the people a name could
/// be): the one that is it put in. One is it when it's the only one `find` gives, or the only one
/// whose name, address or the part of it before `@` is the name typed (`ana` among Ana Lima and
/// Anabel Ruiz is neither: both are offered). The text is rebuilt as a recipient box keeps it.
pub fn check_names(field: &str, find: impl FnMut(&str) -> Vec<EmailAddress>) -> Checked {
    check_names_waiting(field, find, |_| false)
}

/// [`check_names`] while a directory is still asked about some names (`waiting`): those are put
/// in only by an exact match, as the directory may know others by them.
pub fn check_names_waiting(field: &str, mut find: impl FnMut(&str) -> Vec<EmailAddress>, waiting: impl Fn(&str) -> bool) -> Checked {
    let mut entries: Vec<String> = Vec::new();
    let mut out = Checked::default();
    for part in split_outside_quotes(field) {
        let typed = part.trim();
        if typed.is_empty() {
            continue;
        }
        if let Some(a) = parse_one(typed) {
            entries.push(a.to_string());
            continue;
        }
        let mut people: Vec<EmailAddress> = Vec::new();
        for p in find(typed) {
            if p.is_valid() && !people.iter().any(|q| q.email.eq_ignore_ascii_case(&p.email)) && people.len() < MAX_PEOPLE {
                people.push(p);
            }
        }
        let exact: Vec<&EmailAddress> = people.iter().filter(|p| is_named(p, typed)).collect();
        let one = match (exact.as_slice(), people.as_slice()) {
            ([only], _) => Some((*only).clone()),
            ([], [only]) if !waiting(typed) => Some(only.clone()),
            _ => None,
        };
        match one {
            Some(p) => {
                entries.push(p.to_string());
                out.resolved = out.resolved.saturating_add(1);
            }
            None => {
                entries.push(typed.to_string());
                out.unresolved.push(Unresolved { typed: typed.to_string(), people });
            }
        }
    }
    out.field = if entries.is_empty() { String::new() } else { format!("{}; ", entries.join("; ")) };
    out
}

/// Whether `typed` names `p` exactly: their name, their address, or its part before `@`.
fn is_named(p: &EmailAddress, typed: &str) -> bool {
    let local = p.email.split('@').next().unwrap_or_default();
    p.name.trim().eq_ignore_ascii_case(typed) || p.email.eq_ignore_ascii_case(typed) || local.eq_ignore_ascii_case(typed)
}

/// `field` with the entry typed as `typed` (the first so) put in as `person`: the one picked from
/// those [`check_names`] offered. `None` when the box has no such entry any more.
pub fn pick(field: &str, typed: &str, person: &EmailAddress) -> Option<String> {
    let mut found = false;
    let entries: Vec<String> = split_outside_quotes(field)
        .into_iter()
        .map(str::trim)
        .filter(|e| !e.is_empty())
        .map(|e| {
            if !found && e == typed.trim() {
                found = true;
                person.to_string()
            } else {
                e.to_string()
            }
        })
        .collect();
    found.then(|| format!("{}; ", entries.join("; ")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn person(name: &str, email: &str) -> EmailAddress {
        EmailAddress { name: name.into(), email: email.into() }
    }

    /// The people a name could be, as the address book and directory would answer.
    fn book(typed: &str) -> Vec<EmailAddress> {
        let all =
            [person("Ana Lima", "ana@soundlab.example"), person("Anabel Ruiz", "anabel@studio.example"), person("Zoe Park", "zoe@studio.example")];
        let t = typed.to_lowercase();
        all.into_iter().filter(|p| p.name.to_lowercase().contains(&t) || p.email.starts_with(&t)).collect()
    }

    #[test]
    fn a_name_that_is_one_person_is_put_in() {
        let c = check_names("zoe; ops@studio.example, \"Park, Zoe\" <zoe@studio.example>", book);
        assert_eq!(c.field, "Zoe Park <zoe@studio.example>; ops@studio.example; \"Park, Zoe\" <zoe@studio.example>; ");
        assert_eq!((c.resolved, c.unresolved.len()), (1, 0));
        // Ana is two people by her first name, one by her whole name or her address's name.
        let c = check_names("an", book);
        assert_eq!(c.unresolved, [Unresolved { typed: "an".into(), people: book("an") }]);
        assert_eq!(c.field, "an; ", "left as typed");
        for typed in ["Ana Lima", "ana lima", "ana"] {
            let c = check_names(typed, book);
            assert_eq!(c.field, "Ana Lima <ana@soundlab.example>; ", "{typed}");
        }
        // No one by that name: said, left as typed.
        let c = check_names("Mallory", book);
        assert_eq!((c.field.as_str(), c.unresolved.len(), c.unresolved[0].people.len()), ("Mallory; ", 1, 0));
        assert_eq!(check_names("  ;, ", book).field, "");
    }

    /// A directory still asked about a name: the one person known by part of it isn't put in
    /// yet (the directory may know others); one known by all of it is.
    #[test]
    fn a_name_a_directory_is_still_asked_about_waits_for_it() {
        let c = check_names_waiting("zo; zoe park", book, |_| true);
        assert_eq!(c.field, "zo; Zoe Park <zoe@studio.example>; ");
        assert_eq!(c.unresolved, [Unresolved { typed: "zo".into(), people: book("zo") }]);
    }

    #[test]
    fn what_the_lookup_gives_is_taken_safely() {
        // Twice the same person, ones that aren't addresses, and far too many: each once, held.
        let found = |_: &str| {
            let mut v = vec![person("Zoe", "zoe@studio.example"), person("ZOE", "ZOE@studio.example"), person("x", "not an address")];
            v.extend((0..200).map(|i| person("Someone", &format!("s{i}@x.example"))));
            v
        };
        let c = check_names("someone", found);
        assert_eq!(c.unresolved[0].people.len(), MAX_PEOPLE);
        assert_eq!(c.unresolved[0].people.iter().filter(|p| p.email.eq_ignore_ascii_case("zoe@studio.example")).count(), 1);
    }

    #[test]
    fn one_picked_takes_the_place_of_what_was_typed() {
        let ana = person("Ana Lima", "ana@soundlab.example");
        assert_eq!(pick("an; ops@studio.example; an", "an", &ana).as_deref(), Some("Ana Lima <ana@soundlab.example>; ops@studio.example; an; "));
        assert_eq!(pick("ops@studio.example", "an", &ana), None, "no longer there");
    }
}
