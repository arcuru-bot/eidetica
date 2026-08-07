# SPEC-0: Conventions & Versioning

> **Spec:** 0 · **Status:** Draft · **Version:** v0 (unstable — formats may change without notice)

This specification defines the conventions shared by all Eidetica specifications: the status banner, requirement keywords, versioning policy, the system-wide version register, and the fixed content-addressing choices.

## Status Banner

Every specification MUST begin with a banner block stating its spec number, status, and version. Later specs copy this template verbatim, substituting their own values:

```markdown
> **Spec:** N · **Status:** Draft · **Version:** v0 (unstable — formats may change without notice)
```

- **Spec** is the specification's number in the [index](index.md).
- **Status** is one of `Draft`, `Stable`, or `Superseded`.
- **Version** is the version of the format or protocol the spec defines (see below), not a revision counter for the document itself.

## Requirement Keywords

The key words MUST, MUST NOT, REQUIRED, SHALL, SHALL NOT, SHOULD, SHOULD NOT, RECOMMENDED, MAY, and OPTIONAL in this specification are to be interpreted as described in [RFC 2119](https://www.rfc-editor.org/rfc/rfc2119) and [RFC 8174](https://www.rfc-editor.org/rfc/rfc8174) when, and only when, they appear in all capitals.

All Eidetica specifications adopt this convention.

## Versioning Policy

Every format and protocol in Eidetica carries an explicit version, serialized on the wire or on disk (as an integer field, or embedded in an identifier string such as an ALPN).

- **Version 0 means unstable.** A version-0 format is pre-release: it MAY change incompatibly at any time, and no compatibility guarantee exists across releases. All Eidetica formats are currently version 0.
- **Unknown versions MUST be rejected.** An implementation encountering a version it does not support MUST fail closed — reject the entry, message, or connection with an error — rather than attempt a best-effort parse.
- **Versions increment on incompatible change.** Once a format leaves version 0, an incompatible change requires a new version number and a corresponding spec update.

A spec's own **Version** field (in its status banner) mirrors the version constant of the format it defines: the spec documenting `ENTRY_VERSION = 0` is itself at v0. When a format's version increments, the spec is revised and its banner updated in lockstep, so the banner always states which on-wire version the document is normative for.

## Version Register

Every version constant in the system, in one place. This table is the audit surface for the "everything is v0" claim; specs that define these formats MUST keep this register current.

| Surface | Constant | Value | Location |
|---|---|---|---|
| Entry format | `ENTRY_VERSION` (serialized as the `_v` field) | `0` (u8) | `crates/lib/src/entry/mod.rs` |
| Sync protocol | `PROTOCOL_VERSION` | `0` (u32) | `crates/lib/src/sync/protocol.rs` |
| Sync transport (iroh) | `SYNC_ALPN` | `eidetica/v0` | `crates/lib/src/sync/transports/iroh.rs` |
| PasswordStore encryption metadata | `EncryptionInfo.version` | `"v0"` (string) | `crates/lib/src/store/password_store.rs` |

Notes:

- The `_v` entry field is omitted from serialization when it equals `0` and defaults to `0` on deserialization; any other value is rejected.
- The service RPC protocol (SPEC-7) is planned and does not yet define a version constant. It will be added to this register when it lands.

## Content Addressing & Multiformats

Eidetica identifiers follow the [multiformats](https://github.com/multiformats/cid) specifications. These specs are normative references; Eidetica does not redefine them. The fixed choices, used throughout all specifications (verified against `crates/lib/src/entry/id.rs`):

| Choice | Value |
|---|---|
| CID version | [CIDv1](https://github.com/multiformats/cid) |
| String form | [multibase](https://github.com/multiformats/multibase) base32lower (`b` prefix) |
| Entry content codec | [DAG-CBOR](https://ipld.io/specs/codecs/dag-cbor/spec/) (`0x71`) |
| Opaque blob codec | raw (`0x55`) |
| Hash | [multihash](https://github.com/multiformats/multihash) BLAKE3-256 (`0x1e`) |

BLAKE3-256 is the hash for all newly created IDs. Because the multihash is self-describing, an implementation MAY accept CIDs carrying other multihash algorithms when parsing existing data, but MUST NOT produce them.

The resulting entry ID strings look like `bafyr4i...` (CIDv1 + dag-cbor + blake3, base32lower).

## Terminology

[Terminology](../internal/terminology.md) is the authoritative vocabulary. Specifications reuse its terms — Entry, Database, Store, subtree, TreeNode, tip, root — and do not redefine them. When a spec needs a new term, it defines the term locally and SHOULD propose it for inclusion in the terminology page.

## Conformance & Test Vectors

Specifications SHOULD provide byte-exact test vectors: input bytes, expected CID, expected serialized form. Vectors make conformance checkable without reading the reference implementation.

Fenced `rust` code blocks in these docs are compiled and run as doctests by the `eidetica-book-tests` crate. Keep illustrative-but-non-compiling snippets (pseudocode, partial structs, wire dumps) in non-`rust` fenced blocks (e.g. `text`, `json`, `markdown`) so the book test suite stays green.
