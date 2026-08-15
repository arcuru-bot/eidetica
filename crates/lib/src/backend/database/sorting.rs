//! Height-based sorting shared by every database backend.
//!
//! Traversal order feeds CRDT merge, so all backends must order entries
//! identically: by height ascending, tied on [`ID`]'s `Ord` — the CID tuple.
//! Keeping the comparators in one place is what enforces that; a per-backend
//! copy that drifts silently diverges materialized state.
//!
//! Ordering is applied in-process rather than in SQL. The ID tiebreak must
//! follow `ID`'s `Ord`, and SQL `id` columns hold the base32lower string form,
//! whose ASCII order differs from it: base32lower encodes the values 26-31 as
//! the digits `2`-`7`, which sort before letters in ASCII while standing for
//! larger values.
//!
//! Entry sorts use `sort_by_cached_key`: [`Entry::id`] re-serializes and
//! re-hashes the entry on every call, so each key is computed once instead of
//! per comparison.

use crate::entry::{Entry, ID};

/// Sort entries by tree height, with ID as tiebreaker.
pub(crate) fn sort_entries_by_height(entries: &mut [Entry]) {
    entries.sort_by_cached_key(|e| (e.height(), e.id()));
}

/// Sort entries by store height, with ID as tiebreaker.
///
/// Entries missing a height for `store` sort as height 0.
pub(crate) fn sort_entries_by_store_height(store: &str, entries: &mut [Entry]) {
    entries.sort_by_cached_key(|e| (e.subtree_height(store).unwrap_or(0), e.id()));
}

/// Sort `(id, height)` rows by height, with ID as tiebreaker.
///
/// For traversals that carry the height alongside the ID rather than a full
/// entry.
pub(crate) fn sort_ids_by_height<H: Ord>(rows: &mut [(ID, H)]) {
    rows.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
}
