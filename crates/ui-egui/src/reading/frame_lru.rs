//! Decoded pictures kept by the frame each was last shown in: at most so many, those shown
//! least recently let go first, never one shown in this frame (a message with more than that
//! many decoded all of them again every frame).

use std::borrow::Borrow;
use std::collections::HashMap;
use std::hash::Hash;

/// At most `CAP` values by key, each with the frame it was last shown in.
pub(crate) struct FrameLru<K, V, const CAP: usize> {
    each: HashMap<K, (V, u64)>,
}

impl<K, V, const CAP: usize> Default for FrameLru<K, V, CAP> {
    fn default() -> Self {
        FrameLru { each: HashMap::new() }
    }
}

impl<K: Eq + Hash + Clone, V: Clone, const CAP: usize> FrameLru<K, V, CAP> {
    /// The value kept for `k`, now shown in frame `now`.
    pub(crate) fn get<Q>(&mut self, k: &Q, now: u64) -> Option<V>
    where
        K: Borrow<Q>,
        Q: Eq + Hash + ?Sized,
    {
        let (v, used) = self.each.get_mut(k)?;
        *used = now;
        Some(v.clone())
    }

    /// Whether as many are kept as may be, every one of them shown in frame `now` (none can go
    /// for another).
    pub(crate) fn full_of(&self, now: u64) -> bool {
        self.each.len() >= CAP && self.each.values().all(|(_, used)| *used == now)
    }

    /// Room for one more, those shown least recently let go (never one shown in `now`); whether
    /// there is room now.
    fn make_room(&mut self, now: u64) -> bool {
        while self.each.len() >= CAP {
            let Some(oldest) = self.each.iter().filter(|(_, (_, used))| *used < now).min_by_key(|(_, (_, used))| *used).map(|(k, _)| k.clone())
            else {
                break;
            };
            self.each.remove(&oldest);
        }
        self.each.len() < CAP
    }

    /// Keep `v` for `k`, shown in frame `now`, if there's room once those shown least recently
    /// went; whether it's kept.
    pub(crate) fn keep(&mut self, k: K, v: V, now: u64) -> bool {
        if !self.make_room(now) {
            return false;
        }
        self.each.insert(k, (v, now));
        true
    }

    /// Keep `v` for `k`, shown in frame `now`: those shown least recently go to make room, and
    /// past as many as may be (all shown in this frame) it's kept anyway, to go once it's no
    /// longer shown.
    pub(crate) fn keep_anyway(&mut self, k: K, v: V, now: u64) {
        self.make_room(now);
        self.each.insert(k, (v, now));
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.each.len()
    }

    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.each.is_empty()
    }

    /// Each key with its value.
    #[cfg(test)]
    pub(crate) fn iter(&self) -> impl Iterator<Item = (&K, &V)> {
        self.each.iter().map(|(k, (v, _))| (k, v))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Those shown least recently go first, never one shown in this frame; at the cap, `keep`
    /// turns the new one away and `keep_anyway` takes it.
    #[test]
    fn the_least_recently_shown_go_first() {
        let mut lru: FrameLru<String, u32, 3> = FrameLru::default();
        for (i, k) in ["a", "b", "c"].into_iter().enumerate() {
            assert!(lru.keep(k.to_string(), i as u32, i as u64));
        }
        // "a" shown again: "b" is now the one shown least recently.
        assert_eq!(lru.get("a", 3), Some(0));
        assert!(lru.keep("d".into(), 3, 4));
        let mut keys: Vec<&String> = lru.iter().map(|(k, _)| k).collect();
        keys.sort();
        assert_eq!(keys, ["a", "c", "d"]);
        assert_eq!(lru.get("b", 4), None);
        // All three shown in frame 5: no room, none goes.
        for k in ["a", "c", "d"] {
            lru.get(k, 5);
        }
        assert!(lru.full_of(5) && !lru.full_of(6));
        assert!(!lru.keep("e".into(), 4, 5), "turned away");
        assert_eq!(lru.len(), 3);
        lru.keep_anyway("e".into(), 4, 5);
        assert_eq!((lru.len(), lru.get("e", 5)), (4, Some(4)), "kept past the cap");
        // Next frame, room is made again down to the cap.
        assert!(lru.keep("f".into(), 5, 6));
        assert_eq!(lru.len(), 3);
        assert_eq!(lru.get("f", 6), Some(5));
    }
}
