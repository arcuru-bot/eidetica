# Specifications

This section contains normative specifications for Eidetica's interoperable touchpoints: on-disk formats, wire protocols, and the exact byte-level encodings that independent implementations must agree on.

> **Stability:** All wire and on-disk formats are currently at **version 0** and are **UNSTABLE**. They may change without notice and carry no compatibility guarantee. See [SPEC-0](spec-0-conventions.md) for the versioning policy.

## Specifications vs. Design Documents

The [Design Documents](../design/index.md) section captures *rationale*: the decision-making process, alternatives considered, and implementation tradeoffs behind major features. Specifications are the *normative* counterpart: they define formats and protocols exactly, using RFC 2119 requirement keywords, so that conformance can be checked byte-for-byte. A design document explains why something is the way it is; a specification defines what a conforming implementation must produce and accept.

For prose-level explanations of these systems, see the [Internal](../internal/index.md) documentation.

## Specification Index

| Spec | Title | Status |
|------|-------|--------|
| [SPEC-0](spec-0-conventions.md) | Conventions & Versioning | Draft |
| [SPEC-1](spec-1-entry.md) | Entry Format & Content Addressing | Draft |
| [SPEC-2](spec-2-dag.md) | Merkle-DAG & Height | Draft |
| [SPEC-3](spec-3-crdt.md) | CRDT & Merge Semantics | Draft |
| [SPEC-4](spec-4-auth.md) | Authentication & Authorization | Draft |
| SPEC-5 | Store Payload Formats | Planned |
| SPEC-6 | Sync Protocol | Planned |
| SPEC-7 | Service RPC Protocol | Planned |
| SPEC-8 | Instance & System Databases | Planned |

SPEC-0 through SPEC-4 exist so far. The remaining specs will be added as the corresponding surfaces are formalized.
