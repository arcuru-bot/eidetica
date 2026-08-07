# SPEC-1: Entry Format & Content Addressing

> **Spec:** 1 · **Status:** Draft · **Version:** v0 (unstable — formats may change without notice)

This specification defines the Entry: its logical structure, its canonical DAG-CBOR serialization, and the computation of its content-addressed ID. It is byte-exact: an independent implementation following this document MUST produce identical Entry IDs for identical logical content.

Conventions, requirement keywords, and the fixed multiformats choices are defined in [SPEC-0](spec-0-conventions.md). Vocabulary (Entry, Database/Tree, subtree, tip, root) follows [Terminology](../internal/terminology.md); the DAG model is described informally in [DAG Structure](../internal/dag.md).

## Overview

The Entry is the atomic, immutable, content-addressed unit of data in Eidetica. Entries are nodes in a Merkle-DAG: each Entry references its parents by ID, and each Entry's own ID is a CID over its serialized bytes. An Entry carries:

- position in the **main tree** DAG (root reference, parent list, height), and
- data for zero or more named **subtrees**, each with its own parent list forming an independent sub-DAG, and
- signature information authenticating the Entry (detail in SPEC-4, planned).

Because the ID is a pure function of the canonical serialization, two Entries with the same logical content MUST have the same ID, and any change to an Entry's content produces a different ID.

## Logical Structure

An Entry consists of a version, one tree node, an ordered list of subtree nodes, and signature information.

### Entry

| Field | Wire name | Type | Presence |
|---|---|---|---|
| `version` | `_v` | unsigned integer (u8) | OPTIONAL — omitted when `0` (see [Versioning & Validation](#versioning--validation)) |
| `tree` | `tree` | TreeNode map | REQUIRED |
| `subtrees` | `subtrees` | array of SubTreeNode maps | REQUIRED (MAY be empty) |
| `sig` | `sig` | SigInfo map | REQUIRED |

### TreeNode

The Entry's position in the main tree DAG.

| Field | Wire name | Type | Presence |
|---|---|---|---|
| `root` | `root` | link (CID) | OPTIONAL — omitted for root entries, which have no root reference (they are their own root) |
| `parents` | `parents` | array of links | REQUIRED (empty only for root entries) |
| `metadata` | `metadata` | text string or `null` | REQUIRED — `null` when unset. Opaque serialized metadata about this Entry only; never merged between Entries |
| `height` | `h` | unsigned integer (u64) | OPTIONAL — omitted when `0`. Longest path from the tree root: root entries have height 0, children have max(parent heights) + 1. Semantics in SPEC-2 (planned) |

### SubTreeNode

One named subtree the Entry participates in. Subtrees are named (analogous to table names), not identified by ID.

| Field | Wire name | Type | Presence |
|---|---|---|---|
| `name` | `name` | text string | REQUIRED — MUST be non-empty |
| `parents` | `parents` | array of links | REQUIRED (MAY be empty; an empty list roots a new subtree DAG) |
| `data` | `data` | text string | OPTIONAL — omitted when the Entry participates in the subtree without changing data. Opaque serialized payload; payload formats are SPEC-5 (planned) |
| `height` | `h` | unsigned integer (u64) | OPTIONAL — omitted when the subtree inherits the main tree's height; present only for subtrees tracking an independent height |

`data` and `metadata` are opaque strings at this layer (the reference implementation stores serialized JSON in them). They are encoded as CBOR **text strings**, not byte strings, and their contents do not affect this specification beyond their exact bytes contributing to the ID.

### SigInfo

Signature information is embedded in the Entry and is covered by the ID. Its full structure (key hints, delegation paths, signature encoding) is defined in SPEC-4 (planned); this spec defines only what is needed for byte-exact serialization:

| Field | Wire name | Type | Presence |
|---|---|---|---|
| `sig` | `sig` | text string (base64 signature) | OPTIONAL — omitted when the Entry is unsigned or when producing the signing input |
| `key` | `key` | map (SigKey) | REQUIRED |

`key` is serialized as an externally tagged variant map: `{"Direct": {"hint": {...}}}` or `{"Delegation": {"path": [...], "hint": {...}}}`. The default (unsigned) value is `{"Direct": {"hint": {}}}` — an empty hint map with all hint fields omitted.

### The `_root` marker

A root entry (the first Entry of a new tree) is identified by containing a subtree named `_root`, an omitted `root` field, and an empty main `parents` list. Root entries carry random entropy in `metadata` so that independently created trees receive distinct IDs.

## Serialization

Entries are serialized to [DAG-CBOR](https://ipld.io/specs/codecs/dag-cbor/spec/) (codec `0x71`; see the multiformats table in [SPEC-0](spec-0-conventions.md#content-addressing--multiformats)). The DAG-CBOR specification's strict encoding rules are normative; in particular:

- **Map keys are sorted length-first**: shorter keys before longer keys, ties broken by bytewise comparison of the UTF-8 key bytes. The field tables above list fields in declaration order; on the wire the DAG-CBOR ordering governs (e.g. the top-level Entry map serializes as `sig`, `tree`, `subtrees`).
- **Integers** use the shortest possible CBOR encoding.
- **Links** (`root` and every element of `parents` lists) are encoded as IPLD links: CBOR tag 42 wrapping a byte string containing the multibase identity prefix `0x00` followed by the binary CID.
- **Strings** (`name`, `data`, `metadata`, `sig.sig`) are CBOR text strings (major type 3) and MUST be valid UTF-8.
- **Omitted fields are absent**, not null: `_v` when `0`, `tree.root` when absent, `tree.h` when `0`, subtree `data` when absent, subtree `h` when inherited, and `sig.sig` when unsigned MUST NOT appear as map keys. Exception: `tree.metadata` is always present, encoded as CBOR `null` (`0xf6`) when unset.

This determinism — fixed key ordering, fixed omission rules, canonical CBOR primitives — is what makes the Entry ID reproducible: there is exactly one valid byte serialization for a given Entry.

## Canonicalization

Before serialization (and therefore before ID computation and signing), an Entry MUST be normalized as follows:

1. **Sort each parents list** — the main tree's `parents` and every subtree's `parents` — in ascending CID order (defined below).
2. **Deduplicate each parents list**: after sorting, equal adjacent IDs MUST be collapsed to one, so each parents list is a strictly ascending set.
3. **Sort the subtrees list** by `name`, ascending bytewise over the UTF-8 bytes of the name. Subtree names are unique within an Entry, so this order is total.

**CID order** is the tuple comparison (CID version, codec, multihash code, digest length, digest bytes): integers compared numerically, digest bytes compared lexicographically. For the homogeneous case of CIDv1 + dag-cbor + BLAKE3-256 entry IDs this reduces to ascending raw digest byte order. Note this is **not** the lexicographic order of the base32lower string form — base32 does not preserve byte order — even though the reference implementation's documentation informally calls the order "alphabetical".

Canonicalization is a producer-side requirement: an implementation MUST NOT serialize, hash, or sign an Entry that violates these rules. A non-canonical serialization is not an alternate encoding of the same Entry — it deserializes to different logical content and yields a different ID. The reference implementation does not currently reject non-canonical Entries on receipt; receivers MAY do so.

## Entry ID Computation

The ID of an Entry is computed as follows. An implementation MUST:

1. Canonicalize the Entry per [Canonicalization](#canonicalization), with the `sig` field in its final state (including the signature, if the Entry is signed).
2. Serialize the Entry to DAG-CBOR per [Serialization](#serialization).
3. Hash the serialized bytes with BLAKE3-256 and wrap the 32-byte digest as a multihash (code `0x1e`).
4. Construct a CIDv1 with the dag-cbor codec (`0x71`) and that multihash.
5. Render the string form as multibase base32lower (`b` prefix).

Codec and multihash numbers are fixed in [SPEC-0's multiformats table](spec-0-conventions.md#content-addressing--multiformats), including the rule that other multihash algorithms MAY be accepted when parsing but MUST NOT be produced. The resulting entry ID strings begin `bafyr4i`.

## Signing Input

Signing operates on a sig-stripped variant of the same serialization:

- The **signing input** is the canonical DAG-CBOR serialization of the Entry with `sig.sig` removed (the key is omitted per the rules above; the rest of `sig`, including `key`, remains). Signature generation and verification both use these bytes.
- The **Entry ID** is computed over the fully signed Entry — `sig.sig` present and populated. The ID therefore covers the signature itself: the same content signed differently (or unsigned) yields a different ID.

For an unsigned Entry (`sig.sig` absent), the signing input and the ID input are byte-identical. Signature schemes, key resolution, and authorization are SPEC-4 (planned); this section defines only the byte-production boundary.

## Versioning & Validation

- `ENTRY_VERSION` is `0`. The `_v` field MUST be `0`, MUST be omitted from serialization when `0`, and defaults to `0` on deserialization.
- Per [SPEC-0's versioning policy](spec-0-conventions.md#versioning-policy), an implementation encountering any other `_v` value MUST fail closed and reject the Entry.

Structural validation. An implementation MUST reject an Entry where:

- an Entry containing the `_root` marker subtree has a non-empty main `parents` list;
- a non-root Entry has an empty main `parents` list;
- any subtree `name` is the empty string;
- the `root` field or any parent ID (main tree or subtree) is present but empty or not a valid CID.

Empty subtree `parents` lists on non-root entries are structurally permitted (they may legitimately root a new subtree); their consistency is validated against the DAG at higher layers.

## Test Vectors

Vector 1 — unsigned Entry, one subtree, two parents. *Status: verified against the reference implementation (eidetica v0.2.0).*

Input (logical content):

- `root` = `bafkr4ihmugazj26sfo4drh2dfrkcnqjshmph7zkez7jwpmt2dqraljviv4`
- main `parents` = the two IDs below, supplied in reverse order to exercise canonical sorting:
  - `bafkr4ica7skmagmedfrz7wugmxetkazwv4cru7rjyntqyovtvfzvcphkji`
  - `bafkr4ifw2wyijwbc7fb3tuu3tqsrijff6vcaecr4yf5mklo2w7kmxzursm`
- `height` = `1`; `metadata` unset
- one subtree: `name` = `users`, `data` = `{"alice":"1"}`, `parents` = `[]`, `height` inherited
- `sig` = default unsigned (`sig` absent, `key` = `{"Direct": {"hint": {}}}`)

(The root and parent CIDs were generated as raw-codec (`0x55`) BLAKE3 CIDs of the ASCII strings `spec1-vector-root`, `spec1-vector-parent-a`, `spec1-vector-parent-b`; any valid CIDs work identically.)

Canonical DAG-CBOR serialization (231 bytes), annotated:

```text
a3                                       # map(3)
  63 736967                              #   "sig"
  a1                                     #   map(1)
    63 6b6579                            #     "key"
    a1                                   #     map(1)
      66 446972656374                    #       "Direct"
      a1                                 #       map(1)
        64 68696e74                      #         "hint"
        a0                               #         map(0)
  64 74726565                            #   "tree"
  a4                                     #   map(4)
    61 68                                #     "h"
    01                                   #     1
    64 726f6f74                          #     "root"
    d8 2a                                #     tag(42) — IPLD link
      58 25                              #       bytes(37): 0x00 ++ binary CID
      00 01551e20 eca18194ebd22bb8389f432c5426c1323b1e7fe544cfd367b27a1c2205a6a8af
    67 706172656e7473                    #     "parents"
    82                                   #     array(2) — ascending CID order
      d8 2a 58 25
      00 01551e20 40fc94c0198419639fda8665c9350336af051a7e29c3670c3ab3a973513cea4a
      d8 2a 58 25
      00 01551e20 b6d5b084d822f943b9d29b9c251424a5f544020a3cc17ac52ddab7d4cbe69193
    68 6d65746164617461                  #     "metadata"
    f6                                   #     null
  68 7375627472656573                    #   "subtrees"
  81                                     #   array(1)
    a3                                   #     map(3)
      64 64617461                        #       "data"
      6d 7b22616c696365223a2231227d      #       text(13) "{\"alice\":\"1\"}"
      64 6e616d65                        #       "name"
      65 7573657273                      #       text(5) "users"
      67 706172656e7473                  #       "parents"
      80                                 #       array(0)
```

Resulting Entry ID:

```text
bafyr4igxejpn6evlgs5wviwrvhoaejrnqz7wgwi4vbpso3gdcnaftpmkbm
```

Points this vector exercises: `_v` omitted (version 0); top-level and nested map keys in DAG-CBOR length-first order; links as tag 42 with the `0x00` identity prefix; parents sorted into ascending CID order regardless of input order; `metadata` present as `null`; subtree `h` and `sig.sig` omitted; `data` as a text string. Because the Entry is unsigned, its signing input equals these 231 bytes exactly.
