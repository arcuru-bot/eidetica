# DAG Visualizer

The DAG visualizer is a small single-page application served by `eidetica
serve`, alongside the existing dashboard. It is implemented in
[`crates/bin/src/viz/`](https://github.com/arcuru/eidetica/tree/main/crates/bin/src/viz)
and consists of three concerns:

| Module                               | Role                                                                                                                                                                       |
| ------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `viz::graph`                         | Pure data extraction. Takes a [`Database`] and produces a tagged JSON graph of nodes and edges. Independent of HTTP; exercised by unit and integration tests in the module. |
| `viz::handlers`                      | Axum handlers, mounted on the same router as the rest of the dashboard. Reuse the existing session-cookie auth and scope every database open through `User::open_database`. |
| `viz::assets`                        | Embeds the HTML/CSS/JS frontend in the binary via `include_str!`, with attribute-safe substitution for the database id and name on the page shell.                          |

## HTTP surface

The visualizer adds these routes to the existing `serve` router:

| Method | Path                                                | Auth        | Returns                                                          |
| ------ | --------------------------------------------------- | ----------- | ---------------------------------------------------------------- |
| `GET`  | `/dashboard/database/visualize?id=<root_id>`        | Cookie/HTML | The HTML shell (redirects to `/login` if no session)             |
| `GET`  | `/static/viz.css`, `/static/viz.js`                 | Public      | Embedded frontend assets, `Cache-Control: public, max-age=3600`  |
| `GET`  | `/api/dashboard/database/:id/graph`                 | Cookie/JSON | Full DAG payload (see schema below)                              |
| `GET`  | `/api/dashboard/database/:id/entries/:entry_id`     | Cookie/JSON | Detailed view of one entry                                       |
| `GET`  | `/api/dashboard/database/:id/subtrees`              | Cookie/JSON | Database summary including tip count and distinct subtree names  |

API endpoints return a JSON envelope `{ "error": "..." }` on failure with the
appropriate status code (`400` for malformed IDs, `401` for unauthenticated
requests, `403` for permission errors, `404` for unknown databases or
entries).

## Wire format

The wire format is intentionally additive. Every payload starts with
`schema_version`, currently `1`. Bump this only when an existing variant
changes shape; adding new variants to `Node` or `Edge` does **not** require a
version bump, and the frontend renders unknown `kind`s with a fallback style.

### Graph payload

```jsonc
{
  "schema_version": 1,
  "database_id": "bafyr4i…",
  "subtrees": ["_settings", "users", "messages"],
  "nodes": [ /* see Node */ ],
  "edges": [ /* see Edge */ ]
}
```

### `Node`

Tagged with `kind`. Today only one variant is emitted:

```jsonc
{
  "kind": "entry",
  "id":        "bafyr4i…",      // full CID
  "short_id":  "bafyr4ib1234",  // 12-char prefix for display
  "height":    7,                // DAG height
  "is_root":   false,            // true for the database root
  "is_tip":    true,             // true if no children in the main tree
  "subtrees":  ["users", "messages"],
  "sig":       { /* SigInfo */ }
}
```

Future variants planned (additive):

```jsonc
{ "kind": "ipld_object",       "id": "bafy…", "schema": "<schema-id>", … }
{ "kind": "external_database", "id": "bafy…", "name": "rooms",         … }
{ "kind": "external_entry",    "id": "bafy…", "database_id": "bafy…",  … }
```

### `Edge`

All edges point from a child to its parent in the DAG. Today two variants:

```jsonc
{ "kind": "tree_parent",    "from": "bafy…", "to": "bafy…" }
{ "kind": "subtree_parent", "from": "bafy…", "to": "bafy…", "subtree": "users" }
```

Planned additive variants:

```jsonc
{ "kind": "ipld_link",     "from": "bafy…", "to": "bafy…", "schema": "…", "path": "users[3].avatar" }
{ "kind": "cross_database","from": "bafy…", "to_database": "bafy…" }
{ "kind": "cross_entry",   "from": "bafy…", "to_database": "bafy…", "to_entry": "bafy…" }
```

### `EntryDetail`

```jsonc
{
  "schema_version": 1,
  "id":        "bafyr4i…",
  "short_id":  "bafyr4ib1234",
  "root":      "bafyr4i…",      // null for root entries
  "is_root":   false,
  "is_tip":    true,
  "height":    7,
  "tree_parents": ["bafy…", "bafy…"],
  "subtrees": [
    {
      "name":           "messages",
      "height":         null,        // null = inherits the tree's height
      "parents":        ["bafy…"],
      "data_base64":    "eyJoZWxsby4uLg==",
      "data_byte_size": 24,
      "data_text":      "{\"hello\":\"world\"}"  // best-effort UTF-8 decode
    }
  ],
  "sig": { /* SigInfo */ },
  "canonical_byte_size": 312
}
```

## Frontend

The frontend (`crates/bin/src/viz/static_assets/`) is hand-written vanilla JS
plus SVG. Eidetica entries already carry a `height`, so the layout is a
single pass:

1. Group nodes by `height` (the layer).
2. For each layer beyond the first, run one barycenter sweep against the
   previous layer to roughly minimise edge crossings.
3. Place nodes evenly across each layer, render edges as quadratic curves.

Pan is mouse-drag, zoom is wheel; both are applied through a single SVG
group `transform` attribute. The frontend deliberately avoids external
dependencies so the binary is self-contained — there is no CDN fetch and no
build step. Future link kinds in the wire format render with a fallback style
(red dotted lines, generic node shape) until the frontend learns how to draw
them.

## Adding a new link kind

When IPLD object references or cross-database pointers gain a formal place in
the entry layer:

1. Extend [`viz::graph::Node`] or [`viz::graph::Edge`] with a new variant.
   Don't change the existing variants — the bump is additive.
2. Emit the new variant from `build_graph` / `build_entry_detail` wherever the
   information is available.
3. Update the JSON-shape unit tests in `viz::graph::tests` to cover the new
   variant (assertion: `serde_json::to_value(&Edge::Foo { … })["kind"] == "foo"`).
4. Teach the frontend (`viz.js`) about the new `kind` and add a CSS class +
   legend entry in `viz.css`. Until that happens the frontend will render the
   new edges as `unknown` so older builds gracefully degrade.
5. Bump `SCHEMA_VERSION` only if you had to change an existing variant's
   shape.

[`Database`]: ../rustdoc/eidetica/struct.Database.html
[`viz::graph::Node`]: https://github.com/arcuru/eidetica/tree/main/crates/bin/src/viz/graph.rs
[`viz::graph::Edge`]: https://github.com/arcuru/eidetica/tree/main/crates/bin/src/viz/graph.rs
