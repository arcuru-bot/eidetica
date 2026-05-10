//! DAG visualizer for Eidetica databases.
//!
//! Wires a small interactive web UI into the existing `serve` dashboard so a
//! signed-in user can explore the Merkle-DAG of any database they have tracked.
//!
//! # Design
//!
//! The visualizer is split into three concerns:
//!
//! - [`graph`]: pure data extraction. Walks a [`Database`](eidetica::Database)
//!   and produces a typed [`Graph`](graph::Graph) of nodes and edges. The
//!   schema is intentionally tagged with a `kind` discriminator on both nodes
//!   and edges so future link types (IPLD object references, cross-database
//!   pointers, …) can be added without breaking existing clients.
//! - [`handlers`]: axum HTTP handlers that authenticate against the existing
//!   dashboard session, open the requested database via the user's tracked-
//!   database API, and serve [`graph`] output as JSON.
//! - [`assets`]: the embedded HTML/CSS/JS for the single-page viewer. Pure
//!   vanilla JS + SVG with no external dependencies — the DAG is already
//!   pre-layered by [`Entry::height`](eidetica::Entry::height), so the
//!   layered layout is straightforward to compute in the browser.
//!
//! # Extending the schema
//!
//! When a new kind of edge or node becomes representable (for example, when
//! IPLD schemas land), extend the [`graph::Node`] / [`graph::Edge`] enums with
//! a new tagged variant. The frontend renders unknown `kind` values with a
//! generic style and tooltip, so older browser clients won't crash on a newer
//! server's payload.

pub mod assets;
pub mod graph;
pub mod handlers;
