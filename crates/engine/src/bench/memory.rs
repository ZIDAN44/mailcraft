//! What the summaries a session keeps in memory cost ([`Summary::heap_bytes`] and their own
//! size), by part, and what sharing them would save: the numbers the choice between interning a
//! summary's strings and sharing whole summaries (`Arc<Summary>`) was made on (docs/gaps.md).

use std::collections::HashSet;

use crate::Session;
use crate::model::summary::{heap_room, list_room, string_room};
use crate::model::{EmailAddress, Summary};

/// The bytes a run of summaries takes, by part.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Parts {
    pub(super) count: usize,
    /// `size_of::<Summary>()` each: in the list's own buffer.
    pub(super) inline: usize,
    /// The key's account and folder: the same two strings in each of a folder's summaries.
    pub(super) keys: usize,
    /// Sender, To and Cc.
    pub(super) addresses: usize,
    /// Message-ID, In-Reply-To and References.
    pub(super) ids: usize,
    pub(super) subject: usize,
    pub(super) preview: usize,
    pub(super) keywords: usize,
}

impl Parts {
    pub(super) fn of<'a>(list: impl IntoIterator<Item = &'a Summary>) -> Parts {
        let mut p = Parts::default();
        for m in list {
            let addresses = |a: &Vec<EmailAddress>| list_room(a, EmailAddress::heap_bytes);
            p.count += 1;
            p.inline += std::mem::size_of::<Summary>();
            p.keys += m.key.heap_bytes();
            p.addresses += m.from.heap_bytes() + addresses(&m.to) + addresses(&m.cc);
            p.ids += string_room(&m.message_id) + string_room(&m.in_reply_to) + list_room(&m.references, string_room);
            p.subject += string_room(&m.subject);
            p.preview += string_room(&m.preview);
            p.keywords += m.flags.heap_bytes();
        }
        p
    }

    pub(super) fn total(&self) -> usize {
        self.inline + self.keys + self.addresses + self.ids + self.subject + self.preview + self.keywords
    }
}

fn mb(n: usize) -> String {
    format!("{:.1} MB", n as f64 / 1_048_576.0)
}

/// What each holder of summaries holds: the list on view, the folders cached, the list kept
/// while a search shows, and the lists searches replaced, let go of a slice a step and a tick.
fn holders(s: &Session) -> [Parts; 4] {
    let cached = s.cache.values().flat_map(|st| st.messages.iter().map(|m| &**m));
    [Parts::of(&s.list), Parts::of(cached), Parts::of(s.kept_summaries()), Parts::of(s.letting_go_parts().0)]
}

/// Print what the summaries in memory take now, and what each way of sharing them would save.
pub(super) fn report(s: &Session, at: &str) {
    let cached: Vec<&Summary> = s.cache.values().flat_map(|st| st.messages.iter().map(|m| &**m)).collect();
    let kept = s.kept_summaries();
    // Beside the lists searches replaced, the keys of what their runs found.
    let (going, going_keys) = s.letting_go_parts();
    let all = holders(s);
    let [list, cache, kept_p, going_p] = all;
    let total: usize = all.iter().map(Parts::total).sum();
    let count: usize = all.iter().map(|p| p.count).sum();
    let key_bytes: usize = going_keys.iter().map(|k| std::mem::size_of::<crate::model::MessageKey>() + k.heap_bytes()).sum();
    println!(
        "memory {at}: summaries {count} of {} bytes each ({} cached in {} folders, {} listed, {} kept for after a search, {} still to be let go of), {}; and {} keys to be let go of, {}",
        std::mem::size_of::<Summary>(),
        cache.count,
        s.cache.len(),
        list.count,
        kept_p.count,
        going_p.count,
        mb(total),
        going_keys.len(),
        mb(key_bytes)
    );
    let sum = |f: fn(&Parts) -> usize| all.iter().map(f).sum::<usize>();
    println!(
        "  by part: inline {}, keys {}, addresses {}, ids {}, subject {}, preview {}, keywords {}",
        mb(sum(|p| p.inline)),
        mb(sum(|p| p.keys)),
        mb(sum(|p| p.addresses)),
        mb(sum(|p| p.ids)),
        mb(sum(|p| p.subject)),
        mb(sum(|p| p.preview)),
        mb(sum(|p| p.keywords))
    );
    // Shared whole: the list and the kept list hold pointers to the cached summaries (8 bytes
    // each); each cached one moves into an `Arc` of its own (its two counts beside it), a
    // pointer to it left in the cache.
    let in_cache: HashSet<&crate::model::MessageKey> = cached.iter().map(|m| &m.key).collect();
    let copies = s.list.iter().chain(kept).chain(going).filter(|m| in_cache.contains(&m.key));
    let copied = Parts::of(copies);
    let size = std::mem::size_of::<Summary>();
    let boxed = 8 + heap_room(16 + size) - size;
    let arc = copied.total().saturating_sub(copied.count * 8).saturating_sub(cache.count * boxed);
    // Interned: each key's account and folder one string per folder (an `Arc<str>` is 16 bytes,
    // a `String` 24: 16 saved inline per key too), each address's name and address one string,
    // the sender's and To's and Cc's alike (To mostly the user's own).
    let keys = sum(|p| p.keys) + count * 16;
    let mut addresses = HashSet::new();
    let (mut address_heap, mut held) = (0usize, 0usize);
    for m in s.list.iter().chain(cached.iter().copied()).chain(kept).chain(going) {
        for a in std::iter::once(&m.from).chain(&m.to).chain(&m.cc) {
            address_heap += a.heap_bytes();
            held += 1;
            addresses.insert(a);
        }
    }
    let unique: usize = addresses.iter().map(|a| a.heap_bytes()).sum();
    let names = address_heap.saturating_sub(unique) + held * 16;
    let share = |n: usize| if total == 0 { 0.0 } else { n as f64 * 100.0 / total as f64 };
    println!(
        "  saved by sharing whole summaries (Arc<Summary>): {} ({:.0}%); by interning keys: {} ({:.0}%); and addresses: {} ({:.0}%)",
        mb(arc),
        share(arc),
        mb(keys),
        share(keys),
        mb(names),
        share(names)
    );
    if let Some(p) = process() {
        println!("  process: {p}");
    }
}

/// This process's private bytes and working set, as Windows counts them (asked of PowerShell:
/// the bench needs no system calls of its own). None elsewhere.
fn process() -> Option<String> {
    if !cfg!(windows) {
        return None;
    }
    let pid = std::process::id();
    let script = format!(
        "$p = Get-Process -Id {pid}; 'private ' + [math]::Round($p.PrivateMemorySize64 / 1MB, 1) + ' MB, working set ' + [math]::Round($p.WorkingSet64 / 1MB, 1) + ' MB'"
    );
    let out = std::process::Command::new("powershell").args(["-NoProfile", "-Command", &script]).output().ok()?;
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

#[test]
fn the_parts_add_up_to_what_each_summary_holds() {
    let store = crate::store::Store::memory();
    crate::sample::big::generate(&store, 1_800_000_000, crate::sample::big::Shape { folders: 3, messages: 300, inbox: 100 }).unwrap();
    let s = Session::open(Some(store), crate::model::dates::Clock::new(1_800_000_000, 0));
    let cached: Vec<&Summary> = s.cache.values().flat_map(|st| st.messages.iter().map(|m| &**m)).collect();
    assert_eq!(cached.len(), 300);
    let each: usize = cached.iter().map(|m| std::mem::size_of::<Summary>() + m.heap_bytes()).sum();
    assert_eq!(Parts::of(cached.iter().copied()).total(), each);
    assert!(cached.iter().all(|m| m.heap_bytes() > 0));
}

#[test]
fn the_lists_a_search_replaced_are_counted_till_let_go_of() {
    let store = crate::store::Store::memory();
    crate::sample::big::generate(&store, 1_800_000_000, crate::sample::big::Shape { folders: 3, messages: 300, inbox: 100 }).unwrap();
    let mut s = Session::open(Some(store), crate::model::dates::Clock::new(1_800_000_000, 0));
    s.run("folder.open", &serde_json::json!({"account": crate::sample::big::ACCOUNT, "folder": "INBOX"})).unwrap();
    let [list, ..] = holders(&s);
    assert!(list.count > 0);
    // A search's list replacing it: the folder's, not let go of till the steps and ticks after.
    let shown = std::mem::take(&mut s.list);
    s.let_go(shown);
    let [_, _, _, going] = holders(&s);
    assert_eq!((going.count, going.total()), (list.count, list.total()));
    s.let_go_some();
    let [_, _, _, going] = holders(&s);
    assert_eq!(going.count, 0);
}
