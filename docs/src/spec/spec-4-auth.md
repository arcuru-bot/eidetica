# SPEC-4: Authentication & Authorization

> **Spec:** 4 · **Status:** Draft · **Version:** v0 (unstable — formats may change without notice)

This specification defines how Entries are signed and how signatures are authorized: the key and signature formats, the full semantics of the `sig` field whose byte-exact serialization [SPEC-1](spec-1-entry.md) already fixed, the `_settings.auth` document that registers keys, the permission model, cross-tree delegation, and the validation algorithm an implementation MUST apply before accepting an Entry.

Conventions and requirement keywords are defined in [SPEC-0](spec-0-conventions.md). The informal design rationale is in [Authentication Design](../design/authentication.md) and [Authentication Internals](../internal/authentication.md); where those documents and this one disagree on exact formats, this specification is authoritative.

## Overview

Every Entry in an authenticated database is signed. Authorization is resolved against the `auth` document of the database's `_settings` subtree: a signature is only as good as the key's registration there. The pipeline is:

1. The Entry embeds a `SigInfo` — a key reference (`SigKey`) plus a signature — covered by the Entry ID ([SPEC-1 § SigInfo](spec-1-entry.md#siginfo)).
2. The `SigKey` is resolved against `_settings.auth` to one or more candidate public keys with permissions and status.
3. The signature is verified over the [SPEC-1 signing input](spec-1-entry.md#signing-input) — the canonical DAG-CBOR serialization with `sig.sig` stripped.
4. The resolved permission is checked against what the Entry does (data write vs. settings write).

Databases without any configured auth operate unsigned: Entries carry the default empty `SigInfo` and are accepted without verification. The two modes are mutually exclusive per validation state — see [Validation Algorithm](#validation-algorithm).

Each Entry additionally records, in its metadata, the `_settings` tips that were current when it was created, so that validation can in principle be replayed against the exact auth configuration the writer observed — see [Settings Pinning & Verification](#settings-pinning--verification) for what the reference implementation currently enforces.

## Key Format

Public and private keys are rendered as prefixed strings:

```text
key-string = algorithm ":" base64(key-bytes)
algorithm  = "ed25519"
```

- `base64` is **standard-alphabet, padded** Base64 (RFC 4648 §4: `A–Z a–z 0–9 + /`, `=` padding). The reference implementation uses a strict decoder: implementations MUST reject non-canonical encodings (wrong padding, invalid trailing bits, whitespace, URL-safe alphabet).
- An unknown `algorithm` prefix, or a missing `:` separator, MUST be rejected. The key enums are crypto-agile (`PublicKey` / `PrivateKey` are non-exhaustive enums dispatching per algorithm), but `ed25519` is the only algorithm defined at v0.
- The same `algorithm:base64` format is used for private keys in local storage; private keys never appear on the wire or in `_settings`.

Fixed sizes for `ed25519`:

| Quantity | Size | Encoded length |
|---|---|---|
| Public key | 32 bytes | 44 base64 chars |
| Private key (seed) | 32 bytes | 44 base64 chars |
| Signature | 64 bytes | 88 base64 chars |
| Sync handshake challenge | 32 bytes | — (raw bytes) |

An `ed25519` public key MUST decode to exactly 32 bytes and be a valid curve point; otherwise the key string is rejected. Public keys serialize (in `_settings.auth`, in `KeyHint.pubkey`, in JSON) as this prefixed string, never as raw bytes.

## Signature Scheme

Signatures are **Ed25519** (RFC 8032) over the SPEC-1 **signing input**: the canonical DAG-CBOR serialization of the Entry with the `sig.sig` map key omitted. The rest of `sig` — including the complete `key` — is covered by the signature, so a signature binds not only the content but also the claimed key reference and delegation path.

- **Signing.** Compute the signing input, sign it with the Ed25519 private key, and store the 64-byte signature as standard padded base64 in `sig.sig` (a CBOR text string). The Entry ID is then computed over the fully signed Entry, so the ID covers the signature.
- **Verification.** Decode `sig.sig` from base64 (64 bytes exactly), recompute the signing input from the received Entry, and verify with the resolved public key. Any failure — absent signature, non-canonical base64, wrong length, or Ed25519 rejection — is a verification failure.

Verification proves only that some holder of the key signed these bytes; whether that key is *authorized* is the separate resolution and permission check below.

## SigInfo, SigKey, and Hints

SPEC-1 fixed the byte serialization of `sig`; this section defines the full structure. All maps below serialize per SPEC-1's DAG-CBOR rules (length-first key order, `Option` fields omitted when unset).

### SigInfo

| Field | Wire name | Type | Presence |
|---|---|---|---|
| `sig` | `sig` | text string — base64 Ed25519 signature | OPTIONAL — omitted when unsigned and when producing the signing input |
| `key` | `key` | SigKey map | REQUIRED |

### SigKey

An externally tagged variant map — exactly one of:

| Variant | Wire form | Meaning |
|---|---|---|
| `Direct` | `{"Direct": {"hint": KeyHint}}` | Signer resolved in this tree's `_settings.auth` |
| `Delegation` | `{"Delegation": {"path": [DelegationStep, …], "hint": KeyHint}}` | Signer resolved through a chain of delegated trees; `hint` names the key in the *last* tree's `_settings.auth` |

The default (unsigned) value is `{"Direct": {"hint": {}}}`.

### KeyHint

A hint tells the validator where to look for the signer's public key. At most one lookup mode applies:

| Field | Wire name | Type | Presence |
|---|---|---|---|
| `pubkey` | `pubkey` | text string — prefixed public key | OPTIONAL — direct lookup by public key |
| `name` | `name` | text string | OPTIONAL — lookup by key name; names are non-unique aliases, so a name may match several keys |
| `is_global` | `is_global` | bool | OPTIONAL — omitted when `false`. When `true`, the signer claims the tree's [global permission](#the-global-permission); `pubkey` MUST also be set to the actual signing key |

A hint is **set** if `pubkey` or `name` is present. Hint precedence when classifying: global (`is_global` + `pubkey`), then `pubkey`, then `name`.

**Malformed states.** An implementation MUST treat these as invalid (validation returns false; they are not "unsigned"):

- `Direct` with a set hint but no `sig`;
- `Direct` with a `sig` but no set hint;
- `Delegation` with no `sig`.

An Entry is **unsigned** exactly when `key` is `Direct` with an empty hint and `sig` is absent.

### DelegationStep

| Field | Wire name | Type | Presence |
|---|---|---|---|
| `tree` | `tree` | text string — Entry ID | REQUIRED — root ID of the delegated tree; the lookup key into the delegating tree's `delegations` map |
| `tips` | `tips` | array of text strings (Entry IDs) | REQUIRED — tips of the delegated tree observed at signing time |

## The `_settings.auth` Document

Authentication configuration lives under the `auth` key of the `_settings` subtree's Doc CRDT. The structures below are the logical schema; the byte encoding of subtree payloads is the Doc payload format (SPEC-5, planned). Auth changes merge by the Doc CRDT's rules (last-write-wins per field); `AuthKey` and `DelegatedTreeRef` nodes are **atomic** — they merge as a unit, never field-by-field.

Top-level structure:

| Key | Value | Purpose |
|---|---|---|
| `keys` | map: public-key string → AuthKey | Registered keys, **indexed by the prefixed public-key string** (collision-proof; names are only aliases) |
| `delegations` | map: root Entry ID → DelegatedTreeRef | Delegated trees, indexed by the delegated tree's root ID |
| `global` | AuthKey | OPTIONAL — the global (wildcard) permission |

### AuthKey

| Field | Type | Presence |
|---|---|---|
| `name` | string | OPTIONAL — human-readable alias; multiple keys MAY share a name |
| `permissions` | Permission | REQUIRED |
| `status` | string — `"Active"` or `"Revoked"` | REQUIRED |

### Permission (stored form)

| Field | Type | Presence |
|---|---|---|
| `type` | string — `"Admin"`, `"Write"`, or `"Read"` | REQUIRED |
| `priority` | unsigned integer (u32) | REQUIRED for `Admin`/`Write`; absent for `Read` |

### DelegatedTreeRef

| Field | Type | Presence |
|---|---|---|
| `permission_bounds` | PermissionBounds — `{max: Permission, min?: Permission}` | REQUIRED — `max` REQUIRED, `min` OPTIONAL |
| `tree` | TreeReference — `{root: ID, tips: [ID, …]}` | REQUIRED — `tips` are the delegating tree's latest known tips of the delegated tree |

Illustrative logical content (not a byte-exact wire form):

```json
{
  "auth": {
    "keys": {
      "ed25519:2loo0cvPc1kGkkivbEURfetWLJFqcoOpNkUlJIT7/Ug=": {
        "name": "laptop",
        "permissions": { "type": "Write", "priority": 10 },
        "status": "Active"
      }
    },
    "delegations": {
      "bafyr4i…root-of-delegated-tree…": {
        "permission_bounds": {
          "max": { "type": "Write", "priority": 10 },
          "min": { "type": "Read" }
        },
        "tree": { "root": "bafyr4i…", "tips": ["bafyr4i…"] }
      }
    },
    "global": {
      "permissions": { "type": "Read" },
      "status": "Active"
    }
  }
}
```

**Corruption fail-safe.** The `auth` key transitioning to a deleted (tombstoned) state, or holding a non-map value, is a corrupted configuration: the reference implementation refuses to commit a transaction that would produce either state, and validation against such a state fails. `auth` absent-because-never-set is the legitimate unsigned mode; `auth` absent-because-deleted is not.

### The global permission

The `global` entry grants a baseline permission to *any* keypair, without registering it. A signer claims it with a `Direct` hint carrying `is_global: true` plus its actual public key; the signature is verified against that key, and the effective permission is the global `AuthKey`'s. This supports world-writable databases and permission floors. A global hint MUST NOT appear as the final hint of a delegation path.

## Permissions

`Permission` is a totally ordered lattice:

```text
Permission = Read | Write(priority: u32) | Admin(priority: u32)
```

The total order is defined by an **ordering value** (this exact arithmetic is normative — independent implementations MUST order identically):

```text
value(Read)     = 0
value(Write(p)) = 1 +     2³² − 1 − p
value(Admin(p)) = 2 + 2·(2³² − 1) − p
```

Higher value = more privilege. Consequences:

- `Read` < every `Write(_)` < every `Admin(_)` — the ranges are disjoint, so any Admin outranks any Write regardless of priority.
- Within `Write` and `Admin`, **lower priority number = higher privilege**: `Write(0)` is the strongest Write, `Admin(0)` the strongest Admin.

What each level authorizes:

| Level | Author data Entries | Author `_settings` Entries | Manage keys |
|---|---|---|---|
| `Admin(p)` | ✓ | ✓ | ✓ — only for target permissions ≤ its own |
| `Write(p)` | ✓ | ✗ | ✗ |
| `Read` | ✗ | ✗ | ✗ |

- An Entry that includes the `_settings` subtree is a **settings write** and requires `Admin`; any other Entry is a **data write** and requires `Write` or `Admin`. `Read` cannot author Entries at all; it exists as a grantable level (notably for global permissions and delegation bounds) and for future read-gating surfaces.
- Key management is priority-gated: an Admin key MAY create or modify a key only if its own permission is ≥ the target key's permission under the total order above. Delegations may likewise only be created with `bounds.max` ≤ the delegating key's own permission.

## Delegation

A tree MAY delegate authentication to another tree: "any key valid in tree *D* is valid here, clamped to these bounds." Delegated trees are ordinary databases with their own `_settings.auth`; delegation composes recursively (a delegated tree may itself delegate), enabling user-identity trees, groups, and independent key rotation.

### Path resolution

A `SigKey::Delegation` carries a non-empty `path` of `DelegationStep`s plus a final `hint`. Resolution proceeds through the steps, starting from the validating tree's (pinned) auth settings:

1. **Lookup.** Look up `step.tree` in the current tree's `delegations` map. Absent → resolution fails.
2. **Load.** Load the delegated tree by the referenced root ID.
3. **Tip check.** Validate the step's claimed `tips` against the delegated tree (see [Tip pinning](#tip-pinning) for current enforcement).
4. **Bounds accumulation.** Fold this delegation's `permission_bounds` into the cumulative bounds: `max ← min(max_so_far, step_max)`; `min ← max(min_so_far, step_min)` where both are present, otherwise whichever is present. Bounds therefore only ever narrow along a path.
5. **Descend.** The delegated tree's `_settings.auth` becomes the current settings; continue with the next step.

After the last step, resolve the final `hint` in the last tree's auth settings (global hints are forbidden here), yielding one or more candidate keys, each with the permission and status recorded *in that tree*. Then clamp each candidate's permission to the cumulative bounds:

```text
effective = delegated permission
if effective > bounds.max:            effective = bounds.max
else if bounds.min set and effective < bounds.min:  effective = bounds.min
```

Bounds with `min > max` are invalid; the reference implementation falls back to applying only `max`. The candidate's `status` is taken from the tree that registered the key — a key revoked in the delegated tree is revoked for delegated use here.

The reference implementation caps delegation resolution depth at **10** to bound recursion.

### Tip pinning

Each `DelegationStep` pins the `tips` of the delegated tree that the signer observed, and each `DelegatedTreeRef` records the delegating tree's latest known tips. The intent is that permission and revocation checks for a delegated signature are evaluated at a delegated-tree state at least as new as previously observed, so revocations propagate and cannot be rolled back by replaying old state.

**Current enforcement (v0):** the reference implementation checks that each claimed tip either matches a current tip of the delegated tree or exists as an Entry in the local backend, and then resolves the final hint against the delegated tree's *current* auth settings — not against the state named by the claimed tips. Full ancestry validation and resolution-at-claimed-tips are **planned**; the wire format above already carries everything they require.

## Key Status & Revocation

```text
KeyStatus = Active | Revoked
```

Status is per-key in `_settings.auth` (and per-key in each delegated tree). Semantics enforced at v0:

- Only `Active` keys validate: during resolution, candidates whose status is not `Active` are skipped, so an Entry signed by a `Revoked` key fails validation against any settings state in which it is revoked. Revocation is effected by an Admin rewriting the key's `AuthKey` with `status: "Revoked"` (an ordinary settings write).
- Historical Entries signed while the key was `Active` remain part of the DAG; revocation is not retroactive content deletion.

**Planned, not in this implementation:** merge-time authority reduction — deterministic rules for which *concurrent* Entries survive when a revocation races with writes (orphaning a revoked key's concurrent entries, banning whole branches, downgrade semantics). At v0 the enforced boundary is admission-time validation as described in [Validation Algorithm](#validation-algorithm), and admission-time validation currently evaluates the settings state described in the next section.

## Settings Pinning & Verification

Authorization is only deterministic if everyone agrees on *which* auth configuration an Entry is judged against. Eidetica pins this per Entry:

- **Pinning (wire format, normative).** Every Entry produced by a transaction records in `tree.metadata` (the opaque string of [SPEC-1 § TreeNode](spec-1-entry.md#treenode)) a JSON object:

  ```json
  { "settings_tips": ["bafyr4i…", "…"], "entropy": null }
  ```

  `settings_tips` is the set of `_settings` subtree tips reachable from the Entry's main-tree parents — the exact settings state the writer built on. `entropy` is the random u64 used only by root entries (SPEC-1 § The `_root` marker), `null` otherwise. Metadata is covered by the signature and the Entry ID.

- **Commit-time validation.** When committing, the reference implementation validates the new Entry against the merged `_settings` state of its parents — the *pre-transaction* configuration — so an Entry cannot authorize itself by the settings changes it introduces. Bootstrap exception: the first Entry to configure auth in a previously auth-less tree is validated against its own staged auth (otherwise no database could ever turn authentication on).

- **After-the-fact verification.** `verify_entry_signature` re-runs the validation algorithm for a stored Entry. The intended behavior is to reconstruct the historical auth settings from the Entry's pinned `settings_tips` (merging the `_settings` CRDT state at those tips) so that verification is stable under later key revocations and settings changes. **Current enforcement (v0):** the reference implementation validates against the database's *current* settings; historical reconstruction from the pinned tips is planned. Consequently, at v0, revoking a key retroactively fails verification of that key's older Entries. A structured multi-entry verification report is likewise planned; the current API verifies one Entry at a time.

## Validation Algorithm

To decide whether an Entry is valid, an implementation MUST apply the following, given the auth settings selected per the previous section:

1. **Malformed check.** If `SigInfo` is in any [malformed state](#keyhint) → **reject**.
2. **Auth-configured check.** Auth is configured iff the settings contain at least one registered key or an active global permission.
   - Entry unsigned and auth configured → **reject**.
   - Entry unsigned and auth not configured → **accept** (unsigned mode).
   - Entry signed and auth not configured → **reject**.
3. **Resolve.** Resolve `sig.key` to candidate `(public key, effective permission, status)` tuples: `Direct` per its hint (pubkey lookup, name lookup — possibly multiple candidates — or global), `Delegation` per [Path resolution](#path-resolution) including bounds clamping. Resolution failure (unknown key/name/delegation, invalid tips, empty path, global hint in a delegation, depth exceeded) → **reject**.
4. **Classify.** The operation is a settings write if the Entry's subtrees include `_settings`, else a data write.
5. **Check candidates.** For each candidate:
   1. Skip unless `status` is `Active`.
   2. Verify the Ed25519 signature over the [signing input](#signature-scheme) with the candidate's public key; on failure, try the next candidate.
   3. Check the effective permission: data write requires `Write` or `Admin`; settings write requires `Admin`. Sufficient → **accept**. Insufficient → try the next candidate (a name may alias several keys with different permissions).
6. No candidate accepted → **reject**.

Rejection here means the Entry fails validation; it is distinguished from infrastructure errors (I/O, missing delegated tree data), which implementations SHOULD surface as errors rather than verdicts.

## Test Vectors

Vector 1 — signed Entry, `Direct` pubkey hint. *Status: verified against the reference implementation (eidetica v0.2.0): signature generated, verification passes, and verification with a different key fails.*

Signing key (Ed25519 seed = the 32 ASCII bytes `eidetica-spec-4-test-vector-key.`):

```text
seed (hex):  65696465746963612d737065632d342d746573742d766563746f722d6b65792e
public key:  ed25519:2loo0cvPc1kGkkivbEURfetWLJFqcoOpNkUlJIT7/Ug=
```

Logical content: identical to [SPEC-1 Test Vector 1](spec-1-entry.md#test-vectors) (same root, parents, height, `users` subtree), except `sig` is:

```json
{ "key": { "Direct": { "hint": { "pubkey": "ed25519:2loo0cvPc1kGkkivbEURfetWLJFqcoOpNkUlJIT7/Ug=" } } } }
```

**Signing input** (canonical DAG-CBOR, `sig.sig` omitted; 292 bytes):

```text
a363736967a1636b6579a166446972656374a16468696e74a1667075626b65797834
656432353531393a326c6f6f3063765063316b476b6b697662455552666574574c4a
4671636f4f704e6b556c4a4954372f55673d6474726565a461680164726f6f74d82a
58250001551e20eca18194ebd22bb8389f432c5426c1323b1e7fe544cfd367b27a1c
2205a6a8af67706172656e747382d82a58250001551e2040fc94c0198419639fda86
65c9350336af051a7e29c3670c3ab3a973513cea4ad82a58250001551e20b6d5b084
d822f943b9d29b9c251424a5f544020a3cc17ac52ddab7d4cbe69193686d65746164
617461f668737562747265657381a364646174616d7b22616c696365223a2231227d
646e616d6565757365727367706172656e747380
```

(Relative to the SPEC-1 vector's 231 bytes, the only change is inside the `sig` map: `hint` is now `{"pubkey": "ed25519:…"}` instead of empty.)

**Signature** (Ed25519 over the bytes above, base64 as stored in `sig.sig`):

```text
9F07VFgWz4rwI2GiFA0QkIZGX8SK4uCaARHKVqK6lhbYV0tbmbOE3JzVq1PmjXBvkwNP8GY7Sx0xbCX3plh0BA==
```

**Signed Entry** (canonical DAG-CBOR; 386 bytes). Identical to the signing input except the `sig` map gains the `sig` key — annotated head, remainder unchanged:

```text
a3                                       # map(3)
  63 736967                              #   "sig"
  a2                                     #   map(2)
    63 6b6579                            #     "key"  (map as in signing input)
    …
    63 736967                            #     "sig"
    78 58 39463037…3042413d3d            #     text(88) — the base64 signature
  64 74726565                            #   "tree"   (as in signing input)
  …
```

Full bytes:

```text
a363736967a2636b6579a166446972656374a16468696e74a1667075626b65797834
656432353531393a326c6f6f3063765063316b476b6b697662455552666574574c4a
4671636f4f704e6b556c4a4954372f55673d63736967785839463037564667577a34
727749324769464130516b495a475838534b347543614152484b56714b366c686259
563074626d624f45334a7a567131506d6a5842766b774e50384759375378307862435833706c683042413d3d
6474726565a461680164726f6f74d82a58250001551e20eca18194ebd22bb8389f43
2c5426c1323b1e7fe544cfd367b27a1c2205a6a8af67706172656e747382d82a5825
0001551e2040fc94c0198419639fda8665c9350336af051a7e29c3670c3ab3a97351
3cea4ad82a58250001551e20b6d5b084d822f943b9d29b9c251424a5f544020a3cc1
7ac52ddab7d4cbe69193686d65746164617461f668737562747265657381a3646461
74616d7b22616c696365223a2231227d646e616d6565757365727367706172656e74
7380
```

Resulting Entry ID (computed over the signed bytes — note it differs from the unsigned SPEC-1 vector's ID):

```text
bafyr4ibv5ascl42vpbxwbwpoe7be5dk3vxziwk5haiq3hcx23dvriy675a
```

Points this vector exercises: `sig.key` hint serialized with the prefixed public-key string; the signing input containing the full `key` but no `sig.sig`; standard padded base64 for both key (44 chars) and signature (88 chars); DAG-CBOR key order inside `sig` (`key` before `sig`); the ID covering the signature. For this key to *authorize* the Entry, `_settings.auth` at the pinned settings state would register it, e.g. `keys."ed25519:2loo…/Ug=" = { permissions: {type: "Write", priority: 10}, status: "Active" }`.
