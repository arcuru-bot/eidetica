# DAG Visualizer

The Eidetica server ships with an interactive DAG visualizer for any database
the signed-in user has tracked. It's a browser-based tool that renders the
Merkle-DAG of an entire database, lets you click any entry to see its full
detail, and walks you through the parent/child links between entries.

The visualizer is part of `eidetica serve` — there is no separate process and
no extra port to manage. Everything runs against the same in-memory or
on-disk backend the rest of the server uses.

## Launching it

1. Start the server as usual:

   ```bash
   eidetica serve
   ```

2. Open the dashboard at <http://localhost:3000/>, sign in (or register), and
   bootstrap the databases you want to inspect using their tickets.
3. On the dashboard each tracked database has a **Visualize DAG** action.
   Clicking it opens the visualizer for that database. The same link is
   available at the top of the database detail page.

The visualizer URL is:

```
GET /dashboard/database/visualize?id=<database root id>
```

It is gated by the same session cookie as the rest of the dashboard, so a
visitor who is not signed in is redirected to `/login`.

## What you can do

- **Click an entry** to open the right-hand details panel: full content-
  addressable ID, height, root, tree parents, every subtree the entry
  participates in, the bytes it contributes per subtree (decoded as UTF-8 when
  possible), the canonical DAG-CBOR size, and the entry's signature.
- **Pan** by dragging the canvas. **Zoom** with the mouse wheel.
- **Highlight a subtree** with the dropdown in the sidebar — edges in that
  subtree pop forward, all others fade.
- **Toggle edges** on or off (tree-level vs. subtree-level) when you want to
  reduce visual noise.
- **Show only tips and their ancestors** — handy for quickly seeing the
  current frontier of a long history.
- **Fit-to-view** with the *Fit* button if you've panned away from the graph.
- **Click any parent ID** in the details panel to jump to that entry.

## Reading the graph

- A node's **vertical position** is its `height` in the DAG. The root entry
  sits at the top; tips drift downward. Higher-height entries are deeper in
  the history.
- **Solid arrows** are tree-DAG parent edges (the main history).
- **Dashed arrows** are subtree-DAG parent edges (per-store history). When
  the same entry contributes to multiple subtrees, you'll see one dashed
  edge per subtree it has a parent in.
- **Orange** nodes are root entries.
- **Green** nodes are current tips (no children in the main DAG).
- **Grey** nodes are everything in between.

The legend on the left mirrors all of this; if a future Eidetica release adds
new kinds of links (for example, references to external IPLD objects) they'll
appear with a third style and a *Unknown / future link* legend entry — older
binaries will simply ignore them.

## Wire format and extension

The visualizer is powered by three JSON endpoints:

- `GET /api/dashboard/database/:id/graph` — full nodes and edges for the DAG.
- `GET /api/dashboard/database/:id/entries/:entry_id` — full detail for one
  entry, including every subtree's parents and payload.
- `GET /api/dashboard/database/:id/subtrees` — a database summary including
  distinct subtree names and tip counts.

All three return objects tagged with `schema_version`, and node/edge
records carry a `kind` discriminator so the format can grow new variants
without breaking older clients. See
[Internal: DAG Visualizer](../internal/visualizer.md) for the schema and
extension model.

## Performance notes

- The visualizer loads every entry of the database in one request. For most
  Eidetica databases this is fine; for very large histories you'll want to
  filter ahead of time. A future revision will add server-side filters
  (height range, tip-restricted, etc.).
- Layout runs entirely in the browser. Because Eidetica entries already carry
  a `height`, the layered layout is computed in a single pass with a
  barycenter sweep to reduce edge crossings — there is no force simulation
  to wait for.
