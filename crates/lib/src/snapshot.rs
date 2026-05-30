//! Snapshot — an immutable identifier of a database state at a point in time.
//!
//! A `Snapshot` is the set of tip entry IDs that fully identifies the state
//! of a `Database`. Content-addressing makes the mapping bijective: given a
//! snapshot, the entries (and therefore all reachable content) are uniquely
//! determined.
//!
//! Use a `Snapshot` to pin a read view, anchor a transaction, or describe
//! a state transition (e.g. `WriteEvent { from, to }`).
//!
//! Internally a snapshot is a sorted, deduplicated set of DAG tips. The
//! sorted+deduped invariant is enforced at every construction path; equality
//! and hashing are set-equal as a consequence.
//!
//! The user-facing name is `Snapshot`; `tips` is the structural noun used
//! inside the data-structure layer. Local variables and accessors still
//! talk about "tips" — `for tip in snapshot.tips()` reads naturally.

use serde::{Deserialize, Serialize};

use crate::entry::ID;

/// Identifier for a database state — a sorted, deduplicated set of DAG tips.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub struct Snapshot(Vec<ID>);

impl Snapshot {
    /// A snapshot containing no tips — the state of a database with no entries.
    pub const EMPTY: Snapshot = Snapshot(Vec::new());

    /// Construct a snapshot from a vector of tips.
    ///
    /// The tips are sorted and deduplicated; the resulting snapshot satisfies
    /// the canonical invariant regardless of input order.
    pub fn new(mut tips: Vec<ID>) -> Self {
        tips.sort();
        tips.dedup();
        Self(tips)
    }

    /// Borrow the tips as a sorted, deduplicated slice.
    pub fn tips(&self) -> &[ID] {
        &self.0
    }

    /// Consume the snapshot and return the underlying tips.
    pub fn into_tips(self) -> Vec<ID> {
        self.0
    }

    /// Returns true if this snapshot contains no tips.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Number of tips in this snapshot.
    pub fn len(&self) -> usize {
        self.0.len()
    }
}

impl From<Vec<ID>> for Snapshot {
    fn from(tips: Vec<ID>) -> Self {
        Self::new(tips)
    }
}

impl From<&[ID]> for Snapshot {
    fn from(tips: &[ID]) -> Self {
        Self::new(tips.to_vec())
    }
}

impl<const N: usize> From<[ID; N]> for Snapshot {
    fn from(tips: [ID; N]) -> Self {
        Self::new(tips.to_vec())
    }
}

impl AsRef<[ID]> for Snapshot {
    fn as_ref(&self) -> &[ID] {
        &self.0
    }
}

impl<'a> IntoIterator for &'a Snapshot {
    type Item = &'a ID;
    type IntoIter = std::slice::Iter<'a, ID>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(byte: u8) -> ID {
        ID::from_bytes([byte])
    }

    #[test]
    fn empty_const_is_empty() {
        assert!(Snapshot::EMPTY.is_empty());
        assert_eq!(Snapshot::EMPTY.len(), 0);
        assert_eq!(Snapshot::EMPTY.tips(), &[] as &[ID]);
    }

    #[test]
    fn default_equals_empty() {
        assert_eq!(Snapshot::default(), Snapshot::EMPTY);
    }

    #[test]
    fn new_sorts_input() {
        let a = id(1);
        let b = id(2);
        let c = id(3);
        let unsorted = Snapshot::new(vec![c.clone(), a.clone(), b.clone()]);
        let sorted = Snapshot::new(vec![a, b, c]);
        assert_eq!(unsorted.tips(), sorted.tips());
    }

    #[test]
    fn new_dedups_input() {
        let a = id(1);
        let b = id(2);
        let with_dupes = Snapshot::new(vec![a.clone(), b.clone(), a.clone(), b.clone(), a.clone()]);
        let mut expected = vec![a, b];
        expected.sort();
        assert_eq!(with_dupes.len(), 2);
        assert_eq!(with_dupes.tips(), expected.as_slice());
    }

    #[test]
    fn set_equality_holds_via_canonical_form() {
        let a = id(1);
        let b = id(2);
        let s1: Snapshot = vec![a.clone(), b.clone()].into();
        let s2: Snapshot = vec![b, a].into();
        assert_eq!(s1, s2);
    }

    #[test]
    fn hash_matches_for_set_equal_snapshots() {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let a = id(1);
        let b = id(2);
        let s1: Snapshot = vec![a.clone(), b.clone()].into();
        let s2: Snapshot = vec![b, a].into();
        let mut h1 = DefaultHasher::new();
        s1.hash(&mut h1);
        let mut h2 = DefaultHasher::new();
        s2.hash(&mut h2);
        assert_eq!(h1.finish(), h2.finish());
    }

    #[test]
    fn from_slice_sorts_and_dedups() {
        let a = id(1);
        let b = id(2);
        let snap = Snapshot::from(&[b.clone(), a.clone(), a.clone()][..]);
        let mut expected = vec![a, b];
        expected.sort();
        assert_eq!(snap.tips(), expected.as_slice());
    }

    #[test]
    fn from_array_sorts_and_dedups() {
        let a = id(1);
        let b = id(2);
        let snap = Snapshot::from([b.clone(), a.clone(), a.clone()]);
        let mut expected = vec![a, b];
        expected.sort();
        assert_eq!(snap.tips(), expected.as_slice());
    }

    #[test]
    fn into_tips_returns_sorted_vec() {
        let mut expected = vec![id(1), id(2), id(3)];
        expected.sort();
        let snap = Snapshot::new(vec![id(3), id(1), id(2)]);
        assert_eq!(snap.into_tips(), expected);
    }

    #[test]
    fn iter_yields_sorted_tips() {
        let mut expected = vec![id(1), id(2), id(3)];
        expected.sort();
        let snap = Snapshot::new(vec![id(3), id(1), id(2)]);
        let collected: Vec<&ID> = (&snap).into_iter().collect();
        let expected_refs: Vec<&ID> = expected.iter().collect();
        assert_eq!(collected, expected_refs);
    }

    #[test]
    fn as_ref_exposes_sorted_slice() {
        let a = id(1);
        let b = id(2);
        let snap = Snapshot::new(vec![b.clone(), a.clone()]);
        let slice: &[ID] = snap.as_ref();
        let mut expected = vec![a, b];
        expected.sort();
        assert_eq!(slice, expected.as_slice());
    }

    #[test]
    fn serde_roundtrip_preserves_invariant() {
        let a = id(1);
        let b = id(2);
        let snap = Snapshot::new(vec![b, a]);
        let json = serde_json::to_string(&snap).unwrap();
        let parsed: Snapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, snap);
    }
}
