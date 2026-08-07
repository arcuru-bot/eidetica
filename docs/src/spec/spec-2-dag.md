# SPEC-2: Merkle-DAG & Height

> **Spec:** 2 · **Status:** Draft · **Version:** v0 (unstable — formats may change without notice)

This specification defines the Merkle-DAG structure formed by Entries: tree and subtree membership, root entries, tips, the height field, and the deterministic total ordering of entries derived from height. That ordering is the foundation of deterministic CRDT merge (SPEC-3, planned).

Conventions and requirement keywords are defined in [SPEC-0](spec-0-conventions.md). The Entry format, the `h` field's serialization, and the canonical ID/CID rules — including **CID order**, used throughout this document — are defined in [SPEC-1](spec-1-entry.md). Vocabulary follows [Terminology](../internal/terminology.md); the DAG model is described informally in [DAG Structure](../internal/dag.md).

## Overview

Entries form a Merkle-DAG: each Entry references its parents by ID, and each ID is a CID over the Entry's canonical bytes ([SPEC-1](spec-1-entry.md#entry-id-computation)). Because parent references are content-addressed, the DAG is acyclic by construction — an Entry cannot reference a descendant, since the descendant's ID depends on the Entry's own ID — and any Entry transitively authenticates its entire ancestry.

A **Database** (also called a **Tree**) is the DAG of all Entries sharing a common root Entry. Within a tree, every named **subtree** forms its own independent sub-DAG over the subset of Entries that participate in it, using per-subtree parent lists. The main-tree DAG and each subtree DAG are traversed and ordered independently.

## Tree & Subtree Membership

### The main tree

An Entry's position in the main tree is given by its `tree` node (SPEC-1: [TreeNode](spec-1-entry.md#treenode)):

- `tree.root` names the tree the Entry belongs to: it MUST be the ID of the tree's root Entry.
- `tree.parents` lists the Entry's direct predecessors in the main-tree DAG. Every listed parent MUST be an Entry of the same tree (an Entry with the same `tree.root`, or the root Entry itself).

An Entry **belongs to** tree `T` if `tree.root` equals `T`, or the Entry's own ID equals `T` (the root Entry belongs to its own tree).

### Root entries

A **root Entry** starts a new tree. It is identified by all three of (SPEC-1: [The `_root` marker](spec-1-entry.md#the-_root-marker)):

1. a subtree named `_root` (the root marker),
2. an omitted `tree.root` field, and
3. an empty main `parents` list.

A root Entry's ID is the tree's ID. Root entries carry random entropy in `metadata` so independently created trees receive distinct IDs.

### Subtree DAGs

Each subtree node in an Entry (SPEC-1: [SubTreeNode](spec-1-entry.md#subtreenode)) carries its own `parents` list, referencing the Entry's direct predecessors *within that named subtree*. The subtree DAG for name `S` in tree `T` is the DAG over all Entries of `T` that contain a subtree node named `S`, connected by those per-subtree parent links.

Subtree DAGs are independent of the main tree DAG in shape: an Entry's subtree parents are typically *not* its main-tree parents, because the subtree DAG skips Entries that do not participate in `S`. A subtree node with an **empty `parents` list roots a new subtree DAG** — this is how a subtree first appears within an existing tree, and it MAY occur on any Entry, not only the tree's root Entry. Subtree parents MUST reference Entries of the same tree that themselves contain subtree `S`.

Because subtree DAGs are self-contained, a subtree can be synced and verified without fetching Entries that do not participate in it (see [Sparse checkouts](../internal/dag.md#sparse-verified-checkouts)).

## Tips

The **tips** of a tree are the set of Entries belonging to that tree that are not listed in the main `parents` list of any other Entry belonging to the same tree — the childless frontier of the DAG.

The **tips of a subtree** `S` are the set of Entries participating in `S` that are not listed in the subtree-`S` `parents` list of any other Entry of the same tree participating in `S`.

Tips are a function of a replica's local store: two replicas holding different subsets of a tree have different tips. An implementation MUST compute tips against the set of Entries it currently holds. Tips drive both writes and sync:

- **Writes.** A new Entry SHOULD list the current tree tips as its main `parents`, and for each subtree it modifies, the current subtree tips as its subtree `parents`. This makes every write a merge of all locally known heads, keeping the frontier narrow. An implementation MAY instead build on an explicit older set of heads (creating a branch); the result remains a valid DAG.
- **Sync.** Exchanging tips is sufficient to compare replica states: equal tip sets imply equal trees, and a peer's unknown tips identify the ancestry that must be fetched.

An implementation MAY additionally compute *scoped* subtree tips relative to a chosen set of main-tree heads (the subtree frontier reachable from those heads, rather than from the whole store); the reference implementation uses this for validating against historical states.

## Height

Every Entry carries a **height** in the main tree (`tree.h`, u64, omitted from serialization when `0`; SPEC-1). Height is assigned by the producer at Entry creation, is covered by the Entry ID, and is never recomputed by receivers. It is *advisory but deterministic*: correctness of the DAG does not depend on it, but all replicas observe identical stored values and therefore derive identical orderings from it.

### Strategy configuration

The height strategy is tree-level configuration stored under the key `height_strategy` in the `_settings` subtree, as the string `"incremental"` or `"timestamp"`. When unset, the strategy is **Incremental** (the default).

Per-subtree overrides: a subtree MAY be configured (via its settings in the `_index` subtree, key `height_strategy`) to track an independent height. System subtrees (names beginning with `_`) always inherit and cannot be overridden. `_settings` and `_index` semantics are defined in later specs (planned); this spec defines only their effect on height.

### Subtree height & inheritance

A subtree node's `h` field is OPTIONAL:

- **Absent (the default):** the subtree **inherits the Entry's main-tree height**. Note the inherited value is the position in the *main tree*, not the longest path within the subtree DAG (which is generally shorter, since subtree DAGs skip Entries).
- **Present:** the subtree tracks an independent height, computed by the subtree's configured strategy over the *subtree* parents' heights (each parent's height read with the same inheritance rule).

An implementation MUST apply this inheritance rule whenever it reads a subtree height (for the effective height of an Entry within subtree `S`: the subtree node's `h` if present, else the Entry's `tree.h`).

### Incremental strategy

The default. For a new Entry with parent heights `H = {h(p) : p ∈ parents}` (main-tree parents for the tree height; subtree parents for an independent subtree height):

```text
height = 0                if parents is empty (root)
height = max(H) + 1       otherwise
```

Height is then the length of the longest path from the root: root entries have height `0`, and every Entry's height is strictly greater than each of its parents'. Under this strategy height is a pure function of the DAG, so any two correct producers extending the same parents assign the same height.

### Timestamp strategy

Height is the producer's clock reading in milliseconds since the Unix epoch, floored to remain monotonic over parents:

```text
min_height = 0                    if parents is empty
min_height = max(H) + 1           otherwise

height     = max(now_ms, min_height)
```

Properties, exactly as the reference implementation behaves:

- Heights are non-decreasing along every path and strictly increasing past each parent (never less than `max(H) + 1`), even when the local clock is behind.
- **Skew detection:** when `min_height > now_ms`, the producer detects clock skew of `min_height − now_ms` milliseconds. The reference implementation logs a warning and proceeds with `min_height`; it does not reject the write, adjust the clock, or track cumulative skew. Receivers are not required to (and the reference implementation does not) reject Entries with future-dated heights; an implementation MAY apply its own skew policy.
- A skewed-fast producer permanently ratchets the branch's heights into the future; descendants from well-synchronized producers will carry `parent + 1` heights until real time catches up. Deployments choosing this strategy SHOULD have reasonably synchronized clocks.

Timestamp heights are not a pure function of the DAG — two producers extending the same parents at different times assign different heights. Determinism of ordering is unaffected: the assigned height is embedded in the Entry and content-addressed, so all replicas sort by the same values.

## Deterministic Ordering

Merge and replay require a total, replica-independent order over any set of Entries. The normative sort key is the pair:

1. **height**, ascending (the effective height in the DAG being ordered: `tree.h` for the main tree; the [inherited-or-explicit subtree height](#subtree-height--inheritance) for a subtree), then
2. **Entry ID**, ascending in **CID order** as defined in [SPEC-1](spec-1-entry.md#canonicalization).

This yields a total order because IDs are unique. It is a topological order of the DAG whenever heights are ancestor-monotonic (which both strategies guarantee for Entries produced correctly): a parent's height is strictly less than its child's, so parents always sort before their children. Same-height Entries are causally unrelated under correct production, so the ID tiebreak never reorders an ancestor after a descendant.

An implementation MUST use this order wherever a deterministic sequence of Entries is required — in particular for CRDT state computation, which replays the Entries between a merge base and a set of tips in exactly this order (SPEC-3, planned). Two replicas holding the same Entries MUST produce the same sequence.

> **Implementation note.** The reference implementation's SQL backends tiebreak by ordering ID strings (multibase base32lower) as text, while its in-memory backend compares CIDs by the SPEC-1 tuple order. Base32lower does **not** preserve byte order (the characters `2`–`7` encode the *highest* 5-bit values but sort *before* `a`–`z` in ASCII), so the two can disagree on same-height Entries whose digests differ at a group boundary ≥ 26. CID order is normative; the SQL text ordering is a known deviation.

## Validation

Structural rules a receiver MUST enforce (the reference implementation enforces these at storage time, on both locally committed and synced Entries; they extend SPEC-1's [structural validation](spec-1-entry.md#versioning--validation)):

- An Entry containing the `_root` marker subtree MUST have an empty main `parents` list.
- A non-root Entry MUST have a non-empty main `parents` list.
- Every parent reference (main tree and subtree) and the `root` field, when present, MUST be a well-formed, non-empty CID.

Rules that are semantic requirements of this specification but are **not currently enforced by the reference implementation** — receivers MAY reject on them:

- **Height consistency.** Stored heights are taken from the Entry as-is; a receiver does not recompute `max(parent heights) + 1` or check monotonicity against the parents it holds. A malformed producer can therefore ship inconsistent heights, which corrupts ordering (but not DAG integrity). Receivers MAY verify Incremental heights against fetched parents, and MAY apply skew bounds to Timestamp heights.
- **Parent existence and tree membership.** Entries may be stored before their parents arrive (sync delivers Entries out of order), so dangling parent references are tolerated at storage time; cross-tree parent references are likewise not checked at this layer. Higher layers validate reachability when computing state.
- **Empty subtree parents on non-root Entries** are structurally permitted (they may legitimately root a new subtree DAG); whether the subtree genuinely has no prior Entries is only checkable with DAG access, and the reference implementation defers this to its transaction layer for locally created Entries.

Cycles need not be checked: constructing a cycle requires predicting a content hash, so any Entry set connected by valid CID references is acyclic.

## Worked Example

*Status: heights, tips, and traversal order verified against the reference implementation (eidetica v0.2.0), Incremental strategy, in-memory backend.* Entry IDs are shown symbolically — root entropy makes concrete IDs non-reproducible.

Five Entries in one tree, all participating in one subtree `notes`:

```text
        ┌── E2 ──┐
R ── E1 ┤        ├── E4
        └── E3 ──┘
```

- `R` — root Entry (`_root` marker, no parents)
- `E1` — parents `[R]`
- `E2` — parents `[E1]`
- `E3` — parents `[E1]` (created concurrently with E2)
- `E4` — parents `[E2, E3]` (merges both branches)

Resulting Incremental heights:

| Entry | parents | height |
|---|---|---|
| `R` | — | 0 |
| `E1` | `R` | 1 |
| `E2` | `E1` | 2 |
| `E3` | `E1` | 2 |
| `E4` | `E2`, `E3` | 3 |

Tips after each write: `{R}` → `{E1}` → `{E2}` (then `{E2, E3}` once E3 commits) → `{E4}`.

Deterministic traversal order: `R`, `E1`, then `E2` and `E3` in ascending CID order of their IDs, then `E4`. In the verified run the concurrent pair sorted `E3` before `E2` — the ID tiebreak, not creation order, decides. Under the Timestamp strategy the same DAG yields heights equal to each producer's clock milliseconds (root included), with `E4 ≥ max(E2, E3) + 1`; the traversal rule is unchanged.
