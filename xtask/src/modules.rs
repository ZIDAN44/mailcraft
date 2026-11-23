//! `cargo xtask modules`: how a crate's modules hang together, to split code where it divides,
//! not where it's long (AGENTS.md › File size and organisation).
//!
//! - `cargo xtask modules <folder>`: the folder's Rust files grouped by what ties them, its
//!   modules' references to each other (`crate::`, `super::`, `self::` paths) and the commits
//!   that changed them together ([`history`]), by [`cluster`]; scored against the folders
//!   they're in now. The groups are suggestions for a person to name and check, not a layout.
//! - `cargo xtask modules --cycles`: each crate's top-level modules that reach each other
//!   round in a circle (the Acyclic Dependencies Principle: a cycle can't be understood, tested
//!   or moved one piece at a time). It fails on a cycle `xtask/cycles.txt` doesn't list: one
//!   there may go, never come.

mod cluster;
mod history;

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// A Rust file's place: its crate's `src/` folder, and its module path in the crate.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Place {
    root: String,
    module: Vec<String>,
}

/// Where a file sits in its crate (`None`: not under a `src/` folder).
fn place(file: &str) -> Option<Place> {
    let (root, rest) = file.split_once("/src/")?;
    let rest = rest.strip_suffix(".rs")?;
    let mut module: Vec<String> = rest.split('/').map(str::to_string).collect();
    if module.len() == 1 && (module[0] == "lib" || module[0] == "main") {
        module.clear();
    }
    if module.last().is_some_and(|m| m == "mod") {
        module.pop();
    }
    Some(Place { root: format!("{root}/src/"), module })
}

/// The text before its unit tests (`#[cfg(test)]` then `mod tests`): their `super` is the file
/// itself, and they aren't what the module depends on.
fn without_tests(text: &str) -> &str {
    let mut at = 0;
    for line in text.split_inclusive('\n') {
        if line.trim() == "#[cfg(test)]" && text.get(at + line.len()..).is_some_and(|rest| rest.trim_start().starts_with("mod tests")) {
            return text.get(..at).unwrap_or(text);
        }
        at += line.len();
    }
    text
}

fn ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// The module paths a file names: `(base, segments)`, the base `crate`, `self` or `super` (as
/// many as written), the segments after it, one path for each item of a `{ … }` group.
fn paths_in(text: &str) -> Vec<(String, usize, Vec<String>)> {
    let code: String = text.lines().map(|l| l.split("//").next().unwrap_or("")).collect::<Vec<_>>().join("\n");
    let chars: Vec<char> = code.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let before = if i == 0 { ' ' } else { chars[i - 1] };
        let rest: String = chars[i..chars.len().min(i + 7)].iter().collect();
        let base = ["crate::", "super::", "self::"].into_iter().find(|b| rest.starts_with(b));
        let fresh = !ident_char(before) && before != ':';
        let Some(base) = base.filter(|_| fresh) else {
            // A bare path (`sync::Plan`, `encoding::{base64}`): from the module itself, so a
            // child module named first is found; any other (`std::`) resolves to nothing.
            if fresh && ident_char(chars[i]) && !chars[i].is_ascii_digit() {
                let (segments, end) = segments_at(&chars, i);
                let after_colons = end >= 2 && chars[end - 1] == ':' && chars[end - 2] == ':';
                if segments.len() > 1 || after_colons {
                    let mut next = end;
                    if chars.get(end) == Some(&'{') {
                        let (items, close) = group_items(&chars, end);
                        for item in items {
                            let mut s = segments.clone();
                            s.extend(item);
                            out.push(("self".into(), 0, s));
                        }
                        next = close;
                    } else {
                        out.push(("self".into(), 0, segments));
                    }
                    i = next.max(i + 1);
                    continue;
                }
                i = end.max(i + 1);
            } else {
                i += 1;
            }
            continue;
        };
        let mut j = i + base.len();
        let mut ups = usize::from(base == "super::");
        while chars[j..].starts_with(&['s', 'u', 'p', 'e', 'r', ':', ':']) {
            ups += 1;
            j += 7;
        }
        let (segments, end) = segments_at(&chars, j);
        let name = base.trim_end_matches("::").to_string();
        let mut next = end;
        if chars.get(end) == Some(&'{') {
            let (items, close) = group_items(&chars, end);
            for item in items {
                let mut s = segments.clone();
                s.extend(item);
                out.push((name.clone(), ups, s));
            }
            next = close;
        } else {
            out.push((name, ups, segments));
        }
        i = next.max(i + 1);
    }
    out
}

/// `ident::ident::…` from `at`: the identifiers, and where reading stopped (at a `{` or `*`
/// after the last `::`, or just past the last identifier).
fn segments_at(chars: &[char], mut at: usize) -> (Vec<String>, usize) {
    let mut segments = Vec::new();
    loop {
        let start = at;
        while chars.get(at).copied().is_some_and(ident_char) {
            at += 1;
        }
        if at == start {
            return (segments, at);
        }
        segments.push(chars[start..at].iter().collect());
        if chars.get(at) == Some(&':') && chars.get(at + 1) == Some(&':') {
            at += 2;
        } else {
            return (segments, at);
        }
    }
}

/// A `{ … }` group's items, each its leading path (`a::b` of `a::b::{c}`, `x` of `x as y`), and
/// where the group ends.
fn group_items(chars: &[char], open: usize) -> (Vec<Vec<String>>, usize) {
    let mut items = Vec::new();
    let mut depth = 0usize;
    let mut start = true;
    let mut at = open;
    while at < chars.len() {
        match chars[at] {
            '{' => {
                depth += 1;
                start = depth == 1;
                at += 1;
            }
            '}' => {
                depth = depth.saturating_sub(1);
                at += 1;
                if depth == 0 {
                    break;
                }
            }
            ',' if depth == 1 => {
                start = true;
                at += 1;
            }
            c if depth == 1 && start && ident_char(c) => {
                let (s, end) = segments_at(chars, at);
                if !s.is_empty() && s[0] != "self" {
                    items.push(s);
                }
                start = false;
                at = end.max(at + 1);
            }
            _ => at += 1,
        }
    }
    (items, at)
}

/// Every crate's modules and the modules each file refers to: file → files.
struct Graph {
    files: BTreeMap<String, Place>,
    refs: BTreeMap<String, BTreeSet<String>>,
}

fn graph(root: &Path) -> Result<Graph, String> {
    let listed = crate::git_files(root, Some("*.rs"))?;
    let files: BTreeMap<String, Place> = listed.iter().filter_map(|f| place(f).map(|p| (f.clone(), p))).collect();
    let by_module: BTreeMap<(String, Vec<String>), String> = files.iter().map(|(f, p)| ((p.root.clone(), p.module.clone()), f.clone())).collect();
    let texts: BTreeMap<&String, String> = files
        .keys()
        .filter_map(|f| std::fs::read_to_string(root.join(f)).ok().map(|t| (f, without_tests(&t.replace("\r\n", "\n")).to_string())))
        .collect();
    // The absolute path a path in file `p` names, and the longest of it that's a module.
    let absolute = |p: &Place, base: &str, ups: usize, segments: Vec<String>| {
        let mut path: Vec<String> = if base == "crate" { Vec::new() } else { p.module.clone() };
        path.truncate(path.len().saturating_sub(ups));
        path.extend(segments);
        path
    };
    let longest = |root: &str, path: &[String]| {
        let mut k = path.len();
        while k > 0 && !by_module.contains_key(&(root.to_string(), path[..k].to_vec())) {
            k -= 1;
        }
        k
    };
    // What each module re-exports (`pub use bar::Thing;`): the item's name and the file it's
    // from, so a path through the re-export reaches the module that has it.
    let mut reexports: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    for (f, text) in &texts {
        let Some(p) = files.get(*f) else { continue };
        for statement in text.split(';').filter(|s| {
            let s = s.trim_start();
            s.starts_with("pub use ") || s.starts_with("pub(crate) use ") || s.starts_with("pub(super) use ")
        }) {
            for (base, ups, segments) in paths_in(statement) {
                let Some(item) = segments.last().cloned() else { continue };
                let path = absolute(p, &base, ups, segments);
                let k = longest(&p.root, &path);
                if let Some(to) = by_module.get(&(p.root.clone(), path[..k].to_vec())).filter(|to| to != f) {
                    reexports.entry((*f).clone()).or_default().insert(item, to.clone());
                }
            }
        }
    }
    let mut refs: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (f, text) in &texts {
        let Some(p) = files.get(*f) else { continue };
        for (base, ups, segments) in paths_in(text) {
            let path = absolute(p, &base, ups, segments);
            let k = longest(&p.root, &path);
            let Some(mut to) = by_module.get(&(p.root.clone(), path[..k].to_vec())).cloned() else { continue };
            // Through a re-export, to where the item is.
            if let Some(through) = path.get(k).and_then(|item| reexports.get(&to).and_then(|r| r.get(item))) {
                to = through.clone();
            }
            if to != **f {
                refs.entry((*f).clone()).or_default().insert(to);
            }
        }
    }
    Ok(Graph { files, refs })
}

pub fn run(root: &Path, args: &[&str]) -> Result<(), String> {
    match args.first() {
        Some(&"--cycles") => cycles(root),
        Some(folder) => suggest(root, folder),
        None => Err("usage: cargo xtask modules <folder> | --cycles".into()),
    }
}

/// The folder's files grouped by what ties them, against the folders they're in now.
fn suggest(root: &Path, folder: &str) -> Result<(), String> {
    let prefix = format!("{}/", folder.trim_end_matches('/').replace('\\', "/"));
    let g = graph(root)?;
    let nodes: Vec<String> = g.files.keys().filter(|f| f.starts_with(&prefix)).cloned().collect();
    if nodes.is_empty() {
        return Err(format!("no Rust files under {prefix}"));
    }
    let index: BTreeMap<&str, usize> = nodes.iter().enumerate().map(|(i, f)| (f.as_str(), i)).collect();
    // A reference either way counts once, at 1; the commits together add their share.
    let mut weight: BTreeMap<(usize, usize), f64> = BTreeMap::new();
    for (from, tos) in &g.refs {
        let Some(&a) = index.get(from.as_str()) else { continue };
        for to in tos {
            if let Some(&b) = index.get(to.as_str()) {
                weight.insert((a.min(b), a.max(b)), 1.0);
            }
        }
    }
    for ((a, b), share) in history::coupling(root, &prefix)? {
        if let (Some(&x), Some(&y)) = (index.get(a.as_str()), index.get(b.as_str())) {
            *weight.entry((x.min(y), x.max(y))).or_default() += share;
        }
    }
    let edges: cluster::Edges = weight.into_iter().map(|((a, b), w)| (a, b, w)).collect();
    let suggested = cluster::groups(nodes.len(), &edges);
    // Now: a file's group is its first step below the folder (`sync.rs` with `sync/…`).
    let now_key = |f: &str| f[prefix.len()..].split('/').next().unwrap_or("").trim_end_matches(".rs").to_string();
    let keys: BTreeSet<String> = nodes.iter().map(|f| now_key(f)).collect();
    let key_index: BTreeMap<String, usize> = keys.into_iter().enumerate().map(|(i, k)| (k, i)).collect();
    let now: Vec<usize> = nodes.iter().map(|f| key_index.get(&now_key(f)).copied().unwrap_or(0)).collect();
    println!(
        "{prefix}: {} files; modularity now {:.2}, suggested {:.2} (higher: groups tie together inside, less across)",
        nodes.len(),
        cluster::modularity(nodes.len(), &edges, &now),
        cluster::modularity(nodes.len(), &edges, &suggested)
    );
    let mut by_group: BTreeMap<usize, Vec<&str>> = BTreeMap::new();
    for (i, f) in nodes.iter().enumerate() {
        by_group.entry(suggested[i]).or_default().push(&f[prefix.len()..]);
    }
    let mut listed: Vec<Vec<&str>> = by_group.into_values().collect();
    listed.sort_by_key(|g| std::cmp::Reverse(g.len()));
    let (alone, together): (Vec<_>, Vec<_>) = listed.into_iter().partition(|g| g.len() == 1);
    for (n, g) in together.iter().enumerate() {
        println!("  group {}: {}", n + 1, g.join(" "));
    }
    if !alone.is_empty() {
        println!("  tied to nothing here: {}", alone.iter().flatten().copied().collect::<Vec<_>>().join(" "));
    }
    Ok(())
}

/// The budget of cycles allowed (those there when the rule came in), one per line.
const CYCLES: &str = "xtask/cycles.txt";

/// Each crate's cycles among its top-level modules: the strongly connected groups of the
/// graph where a module refers to another if any file in it names a path in the other.
fn found_cycles(root: &Path) -> Result<Vec<String>, String> {
    let g = graph(root)?;
    let mut per_crate: BTreeMap<String, BTreeMap<String, BTreeSet<String>>> = BTreeMap::new();
    // Tests reach everything they test, and only tests reach them: not part of the design.
    let top = |p: &Place| p.module.first().filter(|_| !p.module.iter().any(|m| m == "tests")).cloned();
    for (from, tos) in &g.refs {
        let Some(p) = g.files.get(from) else { continue };
        let Some(a) = top(p) else { continue };
        for to in tos {
            let Some(b) = g.files.get(to).and_then(top) else { continue };
            if a != b {
                per_crate.entry(p.root.clone()).or_default().entry(a.clone()).or_default().insert(b);
            }
        }
    }
    let mut out = Vec::new();
    for (krate, edges) in per_crate {
        for component in strongly_connected(&edges) {
            if component.len() > 1 {
                out.push(format!("{krate} {}", component.join(" ")));
            }
        }
    }
    Ok(out)
}

/// Tarjan's strongly connected components, each sorted, in order of their first module.
fn strongly_connected(edges: &BTreeMap<String, BTreeSet<String>>) -> Vec<Vec<String>> {
    let nodes: BTreeSet<&String> = edges.keys().chain(edges.values().flatten()).collect();
    let nodes: Vec<&String> = nodes.into_iter().collect();
    let at: BTreeMap<&String, usize> = nodes.iter().enumerate().map(|(i, n)| (*n, i)).collect();
    let next: Vec<Vec<usize>> =
        nodes.iter().map(|n| edges.get(*n).map(|s| s.iter().filter_map(|t| at.get(t).copied()).collect()).unwrap_or_default()).collect();
    let (mut index, mut low, mut on) = (vec![usize::MAX; nodes.len()], vec![0; nodes.len()], vec![false; nodes.len()]);
    let (mut stack, mut counter, mut out) = (Vec::new(), 0, Vec::new());
    // Iterative, so a deep graph can't overflow the stack: (node, next edge to look at).
    for start in 0..nodes.len() {
        if index[start] != usize::MAX {
            continue;
        }
        let mut work = vec![(start, 0usize)];
        while let Some(&(v, e)) = work.last() {
            if e == 0 && index[v] == usize::MAX {
                index[v] = counter;
                low[v] = counter;
                counter += 1;
                stack.push(v);
                on[v] = true;
            }
            if let Some(&w) = next[v].get(e) {
                if let Some(top) = work.last_mut() {
                    top.1 += 1;
                }
                if index[w] == usize::MAX {
                    work.push((w, 0));
                } else if on[w] {
                    low[v] = low[v].min(index[w]);
                }
                continue;
            }
            work.pop();
            if let Some(&(parent, _)) = work.last() {
                low[parent] = low[parent].min(low[v]);
            }
            if low[v] == index[v] {
                let mut component = Vec::new();
                while let Some(w) = stack.pop() {
                    on[w] = false;
                    component.push(nodes[w].clone());
                    if w == v {
                        break;
                    }
                }
                component.sort();
                out.push(component);
            }
        }
    }
    out.sort();
    out
}

/// Whether cycle `c` (`crate module module…`) is within one of `allowed`: the same crate, its
/// modules all in that one (a listed cycle may shrink or split, never take in more).
fn within(c: &str, allowed: &BTreeSet<String>) -> bool {
    let mut words = c.split_whitespace();
    let krate = words.next().unwrap_or("");
    let modules: BTreeSet<&str> = words.collect();
    allowed.iter().any(|a| {
        let mut aw = a.split_whitespace();
        aw.next() == Some(krate) && modules.is_subset(&aw.collect())
    })
}

fn cycles(root: &Path) -> Result<(), String> {
    let found = found_cycles(root)?;
    let allowed: BTreeSet<String> = std::fs::read_to_string(root.join(CYCLES))
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_string)
        .collect();
    let new: Vec<&String> = found.iter().filter(|c| !within(c, &allowed)).collect();
    for c in &found {
        println!(
            "cycle: {c}{}",
            if allowed.contains(c) {
                " (listed)"
            } else if within(c, &allowed) {
                " (shrank: list it as it is now)"
            } else {
                ""
            }
        );
    }
    let gone = allowed.iter().filter(|a| !found.contains(a)).count();
    if gone > 0 {
        println!("modules: {gone} listed cycles shrank or are gone: list them in {CYCLES} as they are now");
    }
    if new.is_empty() {
        println!("modules: no new cycles between top-level modules ({} listed)", allowed.len());
        Ok(())
    } else {
        Err(format!(
            "modules: top-level modules that reach each other round in a circle (AGENTS.md › File size and organisation):\n  {}\n\
             move what both need into a module of its own, or have one depend on the other only",
            new.iter().map(|c| c.as_str()).collect::<Vec<_>>().join("\n  ")
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_s_place_in_its_crate() {
        assert_eq!(place("crates/net/src/lib.rs").map(|p| p.module), Some(vec![]));
        assert_eq!(
            place("crates/net/src/worker/sync.rs").map(|p| (p.root, p.module)),
            Some(("crates/net/src/".into(), vec!["worker".into(), "sync".into()]))
        );
        assert_eq!(place("apps/mailcraft/src/main.rs").map(|p| p.module), Some(vec![]));
        assert_eq!(place("apps/mailcraft-cli/tests/real.rs"), None);
    }

    #[test]
    fn paths_named_in_code() {
        let text =
            "use crate::{a::b, c as d, self};\nuse super::super::x::y;\nfn f() { self::z::w(); crate::q(); } // crate::not\nlet s = foo::crate::no;";
        let found = paths_in(text);
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert!(found.contains(&("crate".into(), 0, s(&["a", "b"]))), "{found:?}");
        assert!(found.contains(&("crate".into(), 0, s(&["c"]))));
        assert!(found.contains(&("super".into(), 2, s(&["x", "y"]))));
        assert!(found.contains(&("self".into(), 0, s(&["z", "w"]))));
        assert!(found.contains(&("crate".into(), 0, s(&["q"]))));
        let bare = paths_in(
            "use sync::Plan;
pub use encoding::{base64, utf7};",
        );
        assert!(bare.contains(&("self".into(), 0, s(&["sync", "Plan"]))), "{bare:?}");
        assert!(bare.contains(&("self".into(), 0, s(&["encoding", "utf7"]))), "{bare:?}");
        assert!(!found.iter().any(|(_, _, p)| p.contains(&"not".to_string())), "a comment: {found:?}");
        assert!(!found.iter().any(|(b, _, p)| b != "self" && p.contains(&"no".to_string())), "not a path from the crate: {found:?}");
        assert_eq!(found.iter().filter(|(_, _, p)| p.first().is_some_and(|s| s == "a")).count(), 1, "a group's item once: {found:?}");
    }

    #[test]
    fn unit_tests_are_left_out() {
        assert_eq!(without_tests("use super::a;\n#[cfg(test)]\nmod tests {\n use super::*;\n}\n"), "use super::a;\n");
        assert_eq!(without_tests("#[cfg(test)]\nfn helper() {}\n"), "#[cfg(test)]\nfn helper() {}\n");
    }

    #[test]
    fn cycles_are_found() {
        let mut e: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for (a, b) in [("a", "b"), ("b", "c"), ("c", "a"), ("c", "d"), ("d", "e")] {
            e.entry(a.into()).or_default().insert(b.into());
        }
        let found: Vec<Vec<String>> = strongly_connected(&e).into_iter().filter(|c| c.len() > 1).collect();
        assert_eq!(found, vec![vec!["a".to_string(), "b".into(), "c".into()]]);
    }

    /// A listed cycle may shrink, never grow or move to another crate.
    #[test]
    fn a_listed_cycle_may_shrink() {
        let allowed: BTreeSet<String> = ["crates/x/src/ a b c".to_string()].into();
        assert!(within("crates/x/src/ a b c", &allowed));
        assert!(within("crates/x/src/ a c", &allowed));
        assert!(!within("crates/x/src/ a b c d", &allowed));
        assert!(!within("crates/y/src/ a b", &allowed));
    }
}
