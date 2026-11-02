//! What the summaries a session keeps in memory cost ([`Summary::heap_bytes`] and their own
//! size), by part. The lists hold them shared ([`SharedSummary`]): a summary the cache, the list
//! shown, the list kept and a search's finds all hold is counted once, the others a pointer each.
//! What a copy in each place would have taken (as the lists held them before, docs/gaps.md) is
//! printed beside it, with what interning a summary's strings would save on top.

use std::collections::HashSet;

use crate::Session;
use crate::model::summary::{heap_room, list_room, string_room};
use crate::model::{EmailAddress, SharedSummary, Summary};

/// The bytes a run of summaries takes, by part.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Parts {
    /// The places that hold one (a summary two lists hold, twice).
    pub(super) count: usize,
    /// Of those, the summaries counted here (no holder counted them before).
    pub(super) own: usize,
    /// A pointer a place: in the list's own buffer.
    pub(super) pointers: usize,
    /// Each summary's `size_of::<Summary>()`, with its `Arc`'s two counts.
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
    /// What the places would take, each holding a copy of its own (as the lists did): the first
    /// the summary itself, the others a copy as cloning makes it (each string no longer than
    /// it is).
    pub(super) as_copies: usize,
}

/// What a summary in an `Arc` of its own takes besides its heap: itself and the two counts.
fn boxed() -> usize {
    heap_room(16 + std::mem::size_of::<Summary>())
}

impl Parts {
    /// What `list` holds that no holder counted before (`seen`: the summaries counted).
    pub(super) fn of<'a>(list: impl IntoIterator<Item = &'a SharedSummary>, seen: &mut HashSet<*const Summary>) -> Parts {
        let mut p = Parts::default();
        for m in list {
            p.count += 1;
            p.pointers += std::mem::size_of::<SharedSummary>();
            if !seen.insert(SharedSummary::as_ptr(m)) {
                p.as_copies += std::mem::size_of::<Summary>() + Summary::clone(m).heap_bytes();
                continue;
            }
            p.as_copies += std::mem::size_of::<Summary>() + m.heap_bytes();
            let addresses = |a: &Vec<EmailAddress>| list_room(a, EmailAddress::heap_bytes);
            p.own += 1;
            p.inline += boxed();
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
        self.pointers + self.inline + self.keys + self.addresses + self.ids + self.subject + self.preview + self.keywords
    }
}

fn mb(n: usize) -> String {
    format!("{:.1} MB", n as f64 / 1_048_576.0)
}

/// What each holder of summaries holds: the list on view, the folders cached, the list kept
/// while a search shows, and the lists searches replaced, let go of a slice a step and a tick.
/// The cache is counted first: what the others share with it, it holds.
fn holders(s: &Session) -> [Parts; 4] {
    let mut seen = HashSet::new();
    let cache = Parts::of(s.cache.values().flat_map(|st| st.messages.iter()), &mut seen);
    let list = Parts::of(&s.list, &mut seen);
    let kept = Parts::of(s.kept_summaries(), &mut seen);
    let going = Parts::of(s.letting_go_parts().0, &mut seen);
    [list, cache, kept, going]
}

/// Print what the summaries in memory take now, what sharing them saves, and what interning
/// their strings would save on top.
pub(super) fn report(s: &Session, at: &str) {
    // Beside the lists searches replaced, the keys of what their runs found.
    let (going, going_keys) = s.letting_go_parts();
    let all = holders(s);
    let [list, cache, kept_p, going_p] = all;
    let total: usize = all.iter().map(Parts::total).sum();
    let count: usize = all.iter().map(|p| p.count).sum();
    let own: usize = all.iter().map(|p| p.own).sum();
    let key_bytes: usize = going_keys.iter().map(|k| std::mem::size_of::<crate::model::MessageKey>() + k.heap_bytes()).sum();
    println!(
        "memory {at}: summaries {own} of {} bytes each, held in {count} places ({} cached in {} folders, {} listed, {} kept for after a search, {} still to be let go of), {}; and {} keys to be let go of, {}",
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
        "  by part: pointers {}, inline {}, keys {}, addresses {}, ids {}, subject {}, preview {}, keywords {}",
        mb(sum(|p| p.pointers)),
        mb(sum(|p| p.inline)),
        mb(sum(|p| p.keys)),
        mb(sum(|p| p.addresses)),
        mb(sum(|p| p.ids)),
        mb(sum(|p| p.subject)),
        mb(sum(|p| p.preview)),
        mb(sum(|p| p.keywords))
    );
    // Shared whole: what a copy in each place took, less what's held now.
    let copies = sum(|p| p.as_copies);
    let saved = copies.saturating_sub(total);
    // Interned: each key's account and folder one string per folder (an `Arc<str>` is 16 bytes,
    // a `String` 24: 16 saved inline per key too), each address's name and address one string,
    // the sender's and To's and Cc's alike (To mostly the user's own). Of each summary once.
    let keys = sum(|p| p.keys) + own * 16;
    let (mut counted, mut addresses) = (HashSet::new(), HashSet::new());
    let (mut address_heap, mut held) = (0usize, 0usize);
    let cached = s.cache.values().flat_map(|st| st.messages.iter());
    for m in cached.chain(&s.list).chain(s.kept_summaries()).chain(going) {
        if !counted.insert(SharedSummary::as_ptr(m)) {
            continue;
        }
        for a in std::iter::once(&m.from).chain(&m.to).chain(&m.cc) {
            address_heap += a.heap_bytes();
            held += 1;
            addresses.insert(a);
        }
    }
    let unique: usize = addresses.iter().map(|a| a.heap_bytes()).sum();
    let names = address_heap.saturating_sub(unique) + held * 16;
    let share = |n: usize, of: usize| if of == 0 { 0.0 } else { n as f64 * 100.0 / of as f64 };
    println!(
        "  a copy in each place would take {}: sharing whole summaries saves {} ({:.0}%); interning would save keys {} ({:.0}%) and addresses {} ({:.0}%) more",
        mb(copies),
        mb(saved),
        share(saved, copies),
        mb(keys),
        share(keys, total),
        mb(names),
        share(names, total)
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
    let cached: Vec<&SharedSummary> = s.cache.values().flat_map(|st| st.messages.iter()).collect();
    assert_eq!(cached.len(), 300);
    let each: usize = cached.iter().map(|m| std::mem::size_of::<SharedSummary>() + boxed() + m.heap_bytes()).sum();
    let mut seen = HashSet::new();
    let p = Parts::of(cached.iter().copied(), &mut seen);
    assert_eq!((p.total(), p.own), (each, 300));
    assert_eq!(p.as_copies, cached.iter().map(|m| std::mem::size_of::<Summary>() + m.heap_bytes()).sum::<usize>());
    assert!(cached.iter().all(|m| m.heap_bytes() > 0));
    // Held again (another list sharing them): a pointer each, nothing else.
    let again = Parts::of(cached.iter().copied(), &mut seen);
    assert_eq!((again.count, again.own, again.total()), (300, 0, 300 * std::mem::size_of::<SharedSummary>()));
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
    assert_eq!((going.count, going.as_copies), (list.count, list.as_copies));
    s.let_go_some();
    let [_, _, _, going] = holders(&s);
    assert_eq!(going.count, 0);
}
