# SPEC-3: CRDT & Merge Semantics

> **Spec:** 3 · **Status:** Draft · **Version:** v0 (unstable — formats may change without notice)

This specification defines the merge semantics of Eidetica's CRDT layer: the `Value` model, the `Doc` hierarchical map, the `List` ordered collection, and the normative merge algorithm that folds Entry payloads into a deterministic state. It is semantics-exact: an independent implementation following this document MUST compute identical merged states from identical Entry sets.

Conventions and requirement keywords are defined in [SPEC-0](spec-0-conventions.md). Entries and subtrees are defined in [SPEC-1](spec-1-entry.md); the deterministic DAG traversal order this spec depends on is defined in [SPEC-2](spec-2-dag.md). Vocabulary follows [Terminology](../internal/terminology.md); the informal description is [CRDT Merging](../internal/crdt.md). The byte encoding of CRDT payloads inside Entry `data` fields is SPEC-5 (planned); this spec defines the logical model and merge rules.

## Overview

Eidetica is a **Merkle-CRDT**: CRDT state is not exchanged directly between replicas. Instead, replicas exchange immutable, content-addressed Entries (SPEC-1) forming a Merkle-DAG, and each replica computes state locally by **folding** the Entries of a subtree, in the deterministic traversal order of SPEC-2, through a binary merge operation:

```text
state = ((default ⊕ e₁) ⊕ e₂) ⊕ … ⊕ eₙ     where e₁ … eₙ is the SPEC-2 order
```

Two consequences distinguish this design from conventional state-based CRDTs:

- **Order is deterministic.** Every replica holding the same Entries folds them in the same order (ascending height, ties broken by Entry ID — SPEC-2). Merge therefore does not need to be commutative.
- **Application is exactly-once.** Each Entry appears exactly once in the fold. Merge therefore does not need to be idempotent.

The only algebraic law REQUIRED of a merge operation is **associativity** (see [Algebraic Requirements](#algebraic-requirements)).

**"Last write wins" is defined against traversal order, not clocks.** No wall-clock timestamp participates in merging. Wherever this document says a later value *wins*, "later" means: appearing later in the SPEC-2 traversal order — greater height, or equal height and greater Entry ID. Between two Entries where one is an ancestor of the other, the descendant always wins (it has greater height); between concurrent Entries, the winner is the deterministic-but-arbitrary ID tiebreak.

## Algebraic Requirements

The merge operation has the signature (reference: `crates/lib/src/crdt/traits.rs`):

```text
merge : (&Self, &Self) -> Result<Self>      // left ⊕ right, non-destructive
```

A conforming CRDT type MUST satisfy:

- **Associativity**: `(a ⊕ b) ⊕ c == a ⊕ (b ⊕ c)` for all values `a`, `b`, `c`.

A conforming CRDT type is NOT required to satisfy commutativity (`a ⊕ b == b ⊕ a`) or idempotency (`a ⊕ a == a`). This is safe because the SPEC-2 fold fixes the operand order and applies each Entry exactly once; relaxing these laws is what permits last-write-wins registers (inherently non-commutative) and the atomic-replacement semantics below. Associativity alone guarantees that the fold's result is independent of how the Entry sequence is *batched* — the property implementations exploit for caching and merge-base shortcuts (see [Merge Algorithm](#merge-algorithm)).

Merge is asymmetric: the **right** operand is the later one in traversal order.

## The `Value` Model

`Value` is the unit of merged data (reference: `crates/lib/src/crdt/doc/value.rs`):

| Variant | Type | Kind |
|---|---|---|
| `Null` | unit | leaf |
| `Bool` | boolean | leaf |
| `Int` | signed 64-bit integer | leaf |
| `Text` | UTF-8 string | leaf |
| `Doc` | nested [`Doc`](#doc-merge) | branch |
| `List` | ordered [`List`](#list-merge) | branch |
| `Deleted` | tombstone | leaf (CRDT marker) |

### Value merge rules

`merge(left, right)` MUST be resolved by the first matching rule, in this order:

1. **`left` is `Deleted`** → result is `right` (**resurrection**: any later write, including another branch value, overwrites a tombstone).
2. **`right` is `Deleted`** → result is `Deleted` (**deletion**: a later tombstone wins over any value, leaf or branch).
3. **Both are `Doc`** → recursive [`Doc` merge](#doc-merge).
4. **Both are `List`** → positional [`List` merge](#list-merge).
5. **Otherwise** (both leaves, or mismatched types) → result is `right` (last-write-wins).

Rule 5 applies at type granularity: if `left` is a `Doc` and `right` is an `Int` (or vice versa, or `Doc` vs `List`), `right` replaces `left` wholesale — there is no structural merging across types.

### Tombstones

Deletion is represented by *writing* the `Deleted` value, never by removing a key. Tombstones MUST be retained in stored and serialized CRDT state — they are the only record that a deletion happened, and dropping one would let a merge resurrect stale data. Read APIs hide them: a key whose value is `Deleted` reads as absent, and tombstones are excluded from lengths, iteration, and emptiness checks. Tombstones are currently retained forever; garbage collection of tombstones is planned.

## `Doc` Merge

`Doc` (reference: `crates/lib/src/crdt/doc/mod.rs`) is a hierarchical last-write-wins map:

| Field | Wire name | Type | Presence |
|---|---|---|---|
| `version` | `_v` | unsigned integer (u8) | OPTIONAL — omitted when `0`; `DOC_VERSION` is `0`; any other value MUST be rejected on deserialization (SPEC-0 versioning policy) |
| `atomic` | `_a` | boolean | OPTIONAL — omitted when `false` |
| `children` | `children` | map of string → `Value` | REQUIRED |

`merge(left, right)` for two `Doc`s MUST proceed as:

1. If `right.atomic` is true → the result is a copy of `right` in its entirety (whole-document LWW). No field-wise merging occurs; keys present only in `left` are dropped.
2. Otherwise, start from a copy of `left` and, for each key in `right.children`: if the key exists in the result, merge the values per the [`Value` rules](#value-merge-rules) (with the result's value as `left` and `right`'s as `right`); if it does not exist, insert `right`'s value. Keys present only in `left` are preserved. The result's `atomic` flag is inherited from `left`.

### The `atomic` flag

`atomic` declares "this document is a complete replacement — take all of it," and it changes merging in two ways:

- **As the right operand**, an atomic `Doc` replaces the left operand entirely (rule 1 above) instead of merging field-by-field.
- **As the left operand** merged with a non-atomic right, fields merge structurally but the result *remains atomic* — the flag is **contagious**.

Contagion is what preserves associativity. In a fold `e₁ ⊕ e₂ ⊕ e₃ ⊕ e₄` where `e₃` is atomic and `e₄` edits individual fields, the grouping `(e₁ ⊕ e₂) ⊕ (e₃ ⊕ e₄)` computes `e₃ ⊕ e₄` first; because that intermediate result stays atomic, it still replaces `(e₁ ⊕ e₂)` wholesale, matching the left-to-right fold. An implementation MUST propagate the flag this way.

`atomic` applies per `Doc` node, including nested ones: a non-atomic document may contain an atomic sub-document that merges as a unit while its siblings merge field-wise. Atomic documents are intended for configuration or typed values that must always be written as a consistent whole.

Verified behavior (this block is run by the book's doctests):

```rust
use eidetica::crdt::{Doc, traits::CRDT};

let mut base = Doc::new();
base.set("keep", 1);

let mut replacement = Doc::atomic();
replacement.set("x", 10);

// Atomic right operand: whole-document replacement.
let merged = base.merge(&replacement).unwrap();
assert_eq!(merged.get_as::<i64>("keep"), None); // dropped, not merged
assert_eq!(merged.get_as::<i64>("x"), Some(10));
assert!(merged.is_atomic());

// Atomic left, non-atomic right: structural merge, flag is contagious.
let mut edit = Doc::new();
edit.set("y", 20);
let merged2 = replacement.merge(&edit).unwrap();
assert_eq!(merged2.get_as::<i64>("x"), Some(10));
assert_eq!(merged2.get_as::<i64>("y"), Some(20));
assert!(merged2.is_atomic());
```

### Path addressing

Nested values are addressed by dot-separated paths (reference: `crates/lib/src/crdt/doc/path.rs`): `user.profile.name` names key `name` inside the `Doc` at key `profile` inside the `Doc` at key `user`. Rules:

- A path is a sequence of **components** joined by `.`. Components MUST NOT contain `.`; there is no escape mechanism, so keys containing dots cannot be path-addressed.
- Paths are **normalized** by dropping empty components: leading, trailing, and consecutive dots collapse (`.user`, `user.`, `user..profile` → `user`, `user.profile`); a path of only dots normalizes to the empty path, which denotes the document itself.
- Writing to a path creates missing intermediate `Doc` nodes, and **replaces** any non-`Doc` intermediate (leaf, tombstone, or `List`) with a fresh `Doc` node.
- When traversing (read-only) through a `List`, a numeric component is interpreted as a 0-based index into the list's live (non-tombstone) elements.
- Deleting by path writes a `Deleted` tombstone at that exact path (creating intermediates as needed — deleting a never-written key still records a tombstone). Deleting a parent makes its descendants unreachable, but does not write tombstones at descendant paths.

### JSON projection

`Doc` and `Value` define a lossy JSON projection for display and export. In this projection, tombstones are **absent**, not `null`: a tombstoned map key is omitted from the JSON object, and a tombstoned list element is omitted from the JSON array. (Only a bare, directly projected `Deleted` value — one not reached through a `Doc` or `List` — renders as `null`.) The projection MUST NOT be used to transfer or persist CRDT state, precisely because it erases tombstones; state transfer uses the full serialization (SPEC-5, planned), which retains them.

## `List` Merge

`List` (reference: `crates/lib/src/crdt/doc/list.rs`) is an ordered collection stored as an ordered map from `Position` to `Value`. Order is a property of the stored positions, never of arrival order, which makes concurrent insertion convergent without coordination.

### Position

| Field | Type | Meaning |
|---|---|---|
| `numerator` | signed 64-bit integer | rational numerator |
| `denominator` | unsigned 64-bit integer | rational denominator, MUST be > 0 |
| `unique_id` | UUID (random, version 4) | tiebreak |

A `Position` represents the rational number `numerator/denominator`, stored in lowest terms (reduced by GCD on construction). Positions are totally ordered:

1. Compare the rationals exactly: `a/b < c/d` iff `a·d < c·b`, evaluated in 128-bit arithmetic (which cannot overflow for the representable range).
2. If the rationals are equal, compare `unique_id` bytewise (16-byte lexicographic order).

The random `unique_id` makes every generated Position globally unique, so two replicas that independently generate the "same" rational position still have a deterministic relative order after merge — deterministic given the IDs, but not predictable in advance.

### Position generation

The reference implementation generates positions as follows; conforming implementations MAY generate any Position that preserves the intended relative order, since order is fully determined by the stored Position values:

- **First element / beginning marker**: `0/1`. **End marker**: `i64::MAX / 1`.
- **Append (push)**: `(nₗ + 1)/1` where `nₗ` is the numerator of the current last position (saturating at `i64::MAX`). Note this uses the last position's *numerator*, not its rational value; for a last position with a negative numerator and denominator > 1 (e.g. `-5/2`), `(nₗ+1)/1 = -4` compares *less than* `-5/2 = -2.5`, so an appended element can sort before the previous last element. This is a known v0 limitation; implementations SHOULD generate an appended position strictly greater than every existing position.
- **Insert at index 0**: `(n_f − 1)/d_f` where `n_f/d_f` is the current first position — always strictly less than it.
- **Insert between neighbors** `l = a/b` and `r = c/d`: bring both to the common denominator `b·d` (`l = A/(b·d)`, `r = C/(b·d)`) and take the truncated midpoint `⌊(A+C)/2⌋ / (b·d)`. If truncation collides with either endpoint (adjacent numerators), double the precision instead: use the exact midpoint `(A+C) / (2·b·d)`. The result is then reduced. Examples: between `1/1` and `2/1` → `3/2`; between `1/1` and `3/1` → `2/1`. *(Verified against the reference implementation, eidetica v0.2.0.)*

Every generated Position receives a fresh random `unique_id`.

### List merge rules

`merge(left, right)` for two `List`s MUST be the union of their position maps: for each `(position, value)` in `right`, if `left` has an entry at *exactly* that Position (rational **and** `unique_id`), merge the two values per the [`Value` rules](#value-merge-rules); otherwise insert `right`'s entry. Entries only in `left` are preserved. Because each replica's inserts carry unique IDs, concurrent inserts never collide — both elements survive the merge, ordered by the Position total order. A shared position (same rational, same ID — i.e. an element both replicas inherited from a common ancestor) merges its values, so concurrent edits to the same element resolve by the `Value` rules, and deleting an element is writing `Deleted` at its Position (rule 2: the tombstone wins; the position, and therefore ordering context for its former neighbors, is retained).

Index-based read APIs skip tombstoned elements; `len` counts only live elements. The advanced position-based removal API deletes a map entry physically instead of tombstoning; physically removed entries can be resurrected by merging with any replica that still holds them, so physical removal MUST NOT be used where convergence matters.

Lists serialize as a sequence of `[position, value]` pairs in position order, tombstones included.

Verified behavior:

```rust
use eidetica::crdt::doc::{List, list::Position};

let first = Position::new(1, 1);
let last = Position::new(2, 1);
let mut base = List::new();
base.insert_at_position(first.clone(), "item1");
base.insert_at_position(last.clone(), "item3");

// Two replicas concurrently insert between item1 and item3.
let mut replica_a = base.clone();
let mut replica_b = base.clone();
replica_a.insert_at_position(Position::between(&first, &last), "a-item"); // 3/2, fresh UUID
replica_b.insert_at_position(Position::between(&first, &last), "b-item"); // 3/2, fresh UUID

let mut merged = replica_a.clone();
merged.merge(&replica_b);

// Both inserts survive; endpoints keep their places; the two 3/2
// positions order deterministically by UUID.
assert_eq!(merged.len(), 4);
assert_eq!(merged.get(0).unwrap().as_text(), Some("item1"));
assert_eq!(merged.get(3).unwrap().as_text(), Some("item3"));

// Merging in the other grouping yields the same list.
let mut merged2 = replica_b.clone();
merged2.merge(&replica_a);
assert_eq!(merged, merged2);
```

## Merge Algorithm

Given a subtree (SPEC-1) and a set of tip Entries, the merged CRDT state is defined as follows. An implementation MUST:

1. Collect every Entry that is an ancestor-or-self of any tip within that subtree's DAG.
2. Sort them in the SPEC-2 traversal order: ascending height (the subtree's own height, where tracked), ties broken by ascending Entry ID.
3. Starting from the CRDT type's default (empty) value, fold left-to-right: `state = state ⊕ entryᵢ`, where `entryᵢ` is the Entry's deserialized subtree payload, or the default value if the Entry carries no `data` for the subtree.
4. Apply each Entry **exactly once**.

The result MUST be identical regardless of how the sorted sequence is batched into sub-folds — this is exactly the associativity guarantee, and it is the only algebraic law an implementation may rely on. Commutativity and idempotency are NOT assumed: an implementation MUST NOT reorder Entries relative to the SPEC-2 order and MUST NOT apply an Entry more than once, since either can change the result of an LWW merge.

Implementations MAY shortcut the linear fold provided the result is bit-identical. The reference implementation caches computed states per `(entry, subtree)` and, for multiple tips, computes the state of the tips' **merge base** (the common dominator entry — SPEC-2) and folds only the Entries on the paths from the merge base to the tips, in sorted order, on top of the merge base's state. This decomposition is sound because every ancestor of the merge base has height ≤ it, every path Entry has height > it, and each segment is internally sorted — so the concatenation is a batching of the same globally sorted sequence.

## Worked Examples

The `rust` blocks in this document, including those below, are compiled and executed by the book's test suite (SPEC-0, Conformance); those scenarios are *verified* against the reference implementation (eidetica v0.2.0). Scenarios that depend on DAG traversal use symbolic entry names and are *logical* — they follow from the SPEC-2 order plus the rules above, and were checked at the CRDT-operation level only.

### Example 1 — concurrent writes to one key (logical)

Entries in one subtree: `BASE` (height 1) sets `k = "v0"`; `A` and `B` are concurrent children of `BASE` (both height 2), setting `k = "vA"` and `k = "vB"` respectively; suppose `id(A) < id(B)` in the SPEC-2 ID order.

Traversal order: `BASE`, `A`, `B` (equal heights tie-broken by ID). Fold: `k` becomes `"v0"`, then `"vA"`, then — by Value rule 5, right operand wins — `"vB"`. **`B` wins** solely because its ID sorts later; no timestamp is involved. Had `id(B) < id(A)`, `A` would win.

### Example 2 — delete, then resurrect (verified)

```rust
use eidetica::crdt::{Doc, traits::CRDT};

// Three entries' payloads for one subtree, in traversal order:
let mut e1 = Doc::new();
e1.set("k", "v1");           // e1: write
let mut e2 = Doc::new();
e2.remove("k");              // e2: delete — writes a Deleted tombstone
let mut e3 = Doc::new();
e3.set("k", "v3");           // e3: write again

// After e1 ⊕ e2 the key reads as absent, but the tombstone is retained.
let s12 = e1.merge(&e2).unwrap();
assert!(s12.get("k").is_none());
assert!(s12.is_tombstone("k"));

// e3 resurrects the key (Value rule 1: a later write overwrites a tombstone).
let s123 = s12.merge(&e3).unwrap();
assert_eq!(s123.get_as::<&str>("k"), Some("v3"));

// Associativity: batching does not change the result.
let s23 = e2.merge(&e3).unwrap();
assert_eq!(s123, e1.merge(&s23).unwrap());
```

### Example 3 — concurrent list inserts (verified)

See the [`List` merge rules](#list-merge-rules) block above: replicas A and B both insert between the same neighbors (`1/1` and `2/1`); both generated positions are `3/2` with distinct random UUIDs; after merging in either grouping the list is `[item1, x, y, item3]` with `{x, y} = {a-item, b-item}` ordered by UUID comparison — the same on every replica.
