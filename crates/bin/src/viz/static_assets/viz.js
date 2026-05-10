// Eidetica DAG visualizer — pure SVG, no external deps.
//
// The DAG is already pre-layered server-side by Entry.height, so layout
// reduces to:
//   1. Group nodes by height (each layer is one row).
//   2. Order nodes within a layer to roughly minimize edge crossings using a
//      single barycenter sweep against the previous layer.
//   3. Place nodes evenly across the layer; render edges as quadratic curves.
//
// Node/edge schemas use a `kind` discriminator so future link types
// (IPLD object refs, cross-database pointers, …) render with a fallback style
// instead of breaking the UI.

(function () {
    "use strict";

    const NODE_R = 9;
    const LAYER_GAP = 90;     // vertical spacing between layers (height units)
    const NODE_GAP = 48;      // horizontal spacing between nodes within a layer
    const MARGIN = 80;
    const LABEL_OFFSET = 18;

    const body = document.body;
    const databaseId = body.dataset.databaseId;
    const databaseName = body.dataset.databaseName || "";

    const svg = document.getElementById("dag");
    const viewport = document.getElementById("viewport");
    const edgesLayer = document.getElementById("edges-layer");
    const nodesLayer = document.getElementById("nodes-layer");
    const errBanner = document.getElementById("error-banner");
    const subtreeSelect = document.getElementById("subtree-select");
    const treeToggle = document.getElementById("toggle-tree-edges");
    const subtreeToggle = document.getElementById("toggle-subtree-edges");
    const tipsOnlyToggle = document.getElementById("toggle-only-tips");
    const statsList = document.getElementById("stats-list");
    const dbIdEl = document.getElementById("db-id");
    const detailsEmpty = document.getElementById("details-empty");
    const detailsBody = document.getElementById("details-body");

    let graph = null;
    /** Map from id -> rendered node element. */
    const nodeEls = new Map();
    /** Map from id -> { x, y, layer }. */
    const positions = new Map();
    /** All edge elements, indexed for fast filtering. */
    let edgeEls = [];
    /** Currently selected entry id. */
    let selectedId = null;

    // ----- View transform (pan + zoom) -----
    let view = { x: 0, y: 0, k: 1 };
    function applyView() {
        viewport.setAttribute("transform",
            `translate(${view.x},${view.y}) scale(${view.k})`);
    }

    function showError(msg) {
        errBanner.textContent = msg;
        errBanner.hidden = false;
    }
    function clearError() {
        errBanner.hidden = true;
    }

    // ----- Boot -----
    init().catch((e) => {
        console.error(e);
        showError("Failed to load: " + (e && e.message ? e.message : e));
    });

    async function init() {
        if (!databaseId) {
            showError("No database id supplied");
            return;
        }
        dbIdEl.textContent = databaseId;
        await loadGraph();
        bindControls();
    }

    async function loadGraph() {
        const url = `/api/dashboard/database/${encodeURIComponent(databaseId)}/graph`;
        const resp = await fetch(url, { headers: { "accept": "application/json" } });
        if (!resp.ok) {
            const text = await resp.text();
            throw new Error(`graph fetch failed: ${resp.status} ${text}`);
        }
        graph = await resp.json();
        populateSubtreeSelect(graph.subtrees);
        renderStats(graph);
        layout(graph);
        render(graph);
        fitToView();
    }

    function populateSubtreeSelect(subtrees) {
        // Keep "(none)" as the default first option.
        for (const name of subtrees) {
            const opt = document.createElement("option");
            opt.value = name;
            opt.textContent = name;
            subtreeSelect.appendChild(opt);
        }
    }

    function renderStats(g) {
        const tipCount = g.nodes.filter(n => n.kind === "entry" && n.is_tip).length;
        const rootCount = g.nodes.filter(n => n.kind === "entry" && n.is_root).length;
        const treeEdges = g.edges.filter(e => e.kind === "tree_parent").length;
        const subEdges = g.edges.filter(e => e.kind === "subtree_parent").length;
        const otherEdges = g.edges.length - treeEdges - subEdges;
        statsList.innerHTML = "";
        const rows = [
            ["Entries", g.nodes.length],
            ["Tips", tipCount],
            ["Roots", rootCount],
            ["Tree edges", treeEdges],
            ["Subtree edges", subEdges],
            ["Subtrees", g.subtrees.length],
        ];
        if (otherEdges > 0) rows.push(["Other edges", otherEdges]);
        for (const [k, v] of rows) {
            const li = document.createElement("li");
            li.innerHTML = `<span>${k}</span><b>${v}</b>`;
            statsList.appendChild(li);
        }
    }

    // ----- Layout -----

    function layout(g) {
        // Group entry-nodes by height; non-entry kinds (future) get height 0
        // so they cluster near the root and can later be styled separately.
        const layers = new Map(); // height -> [nodes...]
        for (const n of g.nodes) {
            const h = (n.kind === "entry") ? (n.height || 0) : 0;
            if (!layers.has(h)) layers.set(h, []);
            layers.get(h).push(n);
        }
        const layerHeights = [...layers.keys()].sort((a, b) => a - b);

        // Barycenter sweep: for each layer beyond the first, order nodes by
        // the average x of their tree-parents in the previous layer (if any).
        // Stable but cheap; it dramatically reduces visible crossings on
        // typical Eidetica DAGs without needing a full crossing-min algorithm.
        const treeParents = buildTreeParentIndex(g.edges);

        let prevOrder = layers.get(layerHeights[0]) || [];
        prevOrder.sort((a, b) => idCompare(a, b));
        positions.clear();
        layoutLayer(prevOrder, layerHeights[0]);

        for (let i = 1; i < layerHeights.length; i++) {
            const layer = layers.get(layerHeights[i]);
            for (const n of layer) {
                const ps = treeParents.get(n.id) || [];
                let acc = 0, count = 0;
                for (const p of ps) {
                    const pos = positions.get(p);
                    if (pos) { acc += pos.x; count += 1; }
                }
                n._bary = count > 0 ? acc / count : Number.POSITIVE_INFINITY;
            }
            layer.sort((a, b) => {
                if (a._bary !== b._bary) return a._bary - b._bary;
                return idCompare(a, b);
            });
            layoutLayer(layer, layerHeights[i]);
        }
    }

    function buildTreeParentIndex(edges) {
        const idx = new Map();
        for (const e of edges) {
            if (e.kind !== "tree_parent") continue;
            if (!idx.has(e.from)) idx.set(e.from, []);
            idx.get(e.from).push(e.to);
        }
        return idx;
    }

    function idCompare(a, b) {
        return (a.id < b.id) ? -1 : (a.id > b.id ? 1 : 0);
    }

    function layoutLayer(nodes, height) {
        // Center the row around x=0 so panning works intuitively.
        const w = (nodes.length - 1) * NODE_GAP;
        const startX = -w / 2;
        const y = -height * LAYER_GAP; // higher height -> visually lower (toward root at top)
        nodes.forEach((n, i) => {
            positions.set(n.id, { x: startX + i * NODE_GAP, y, layer: height });
        });
    }

    // ----- Rendering -----

    function render(g) {
        edgesLayer.innerHTML = "";
        nodesLayer.innerHTML = "";
        nodeEls.clear();
        edgeEls = [];

        for (const e of g.edges) {
            const el = renderEdge(e);
            if (el) {
                edgesLayer.appendChild(el);
                edgeEls.push({ el, edge: e });
            }
        }
        for (const n of g.nodes) {
            const el = renderNode(n);
            nodesLayer.appendChild(el);
            nodeEls.set(n.id, el);
        }
        applyEdgeFilters();
    }

    function renderNode(n) {
        const pos = positions.get(n.id) || { x: 0, y: 0 };
        const g = document.createElementNS("http://www.w3.org/2000/svg", "g");
        const isEntry = n.kind === "entry";
        const isRoot = isEntry && n.is_root;
        const isTip = isEntry && n.is_tip;
        let cls = "node";
        if (isRoot) cls += " is-root";
        if (isTip) cls += " is-tip";
        if (!isEntry) cls += " is-other";
        g.setAttribute("class", cls);
        g.setAttribute("transform", `translate(${pos.x},${pos.y})`);
        g.dataset.id = n.id;

        const c = document.createElementNS("http://www.w3.org/2000/svg", "circle");
        c.setAttribute("r", NODE_R);
        g.appendChild(c);

        const label = document.createElementNS("http://www.w3.org/2000/svg", "text");
        label.setAttribute("x", 0);
        label.setAttribute("y", -LABEL_OFFSET);
        label.setAttribute("text-anchor", "middle");
        label.textContent = n.short_id || n.id || n.kind;
        g.appendChild(label);

        if (isEntry) {
            const heightLabel = document.createElementNS("http://www.w3.org/2000/svg", "text");
            heightLabel.setAttribute("x", 0);
            heightLabel.setAttribute("y", LABEL_OFFSET + 6);
            heightLabel.setAttribute("text-anchor", "middle");
            heightLabel.textContent = `h=${n.height}`;
            g.appendChild(heightLabel);
        }

        g.addEventListener("click", (ev) => {
            ev.stopPropagation();
            selectNode(n.id);
        });
        return g;
    }

    function renderEdge(e) {
        const a = positions.get(e.from);
        const b = positions.get(e.to);
        if (!a || !b) return null;

        const path = document.createElementNS("http://www.w3.org/2000/svg", "path");
        // Quadratic curve with a small bow so multiple edges between layers
        // don't completely overlap.
        const dx = b.x - a.x;
        const dy = b.y - a.y;
        const mx = (a.x + b.x) / 2 + dy * 0.05;
        const my = (a.y + b.y) / 2 - dx * 0.05;
        path.setAttribute("d", `M ${a.x} ${a.y} Q ${mx} ${my} ${b.x} ${b.y}`);

        const known = (e.kind === "tree_parent" || e.kind === "subtree_parent");
        const cls = "edge " + (known ? e.kind : "unknown");
        path.setAttribute("class", cls);
        path.setAttribute("data-kind", e.kind);
        if (e.subtree) path.setAttribute("data-subtree", e.subtree);
        if (known) {
            path.setAttribute("marker-end",
                e.kind === "tree_parent" ? "url(#arrow-tree)" : "url(#arrow-subtree)");
        }
        return path;
    }

    // ----- Selection + details -----

    async function selectNode(id) {
        for (const el of nodeEls.values()) el.classList.remove("selected");
        const el = nodeEls.get(id);
        if (el) el.classList.add("selected");
        selectedId = id;
        applyEdgeFilters();
        await loadDetails(id);
    }

    async function loadDetails(id) {
        try {
            const url = `/api/dashboard/database/${encodeURIComponent(databaseId)}/entries/${encodeURIComponent(id)}`;
            const resp = await fetch(url, { headers: { "accept": "application/json" } });
            if (!resp.ok) throw new Error(`detail fetch failed: ${resp.status}`);
            const detail = await resp.json();
            renderDetails(detail);
        } catch (e) {
            console.error(e);
            showError(e.message);
        }
    }

    function renderDetails(d) {
        clearError();
        detailsEmpty.hidden = true;
        detailsBody.hidden = false;

        document.getElementById("d-id").textContent = d.id;
        document.getElementById("d-height").textContent = d.height;
        document.getElementById("d-root").textContent = d.root || "(this entry is the root)";
        document.getElementById("d-size").textContent = `${d.canonical_byte_size} bytes`;

        const flags = document.getElementById("d-flags");
        flags.innerHTML = "";
        if (d.is_root) flags.appendChild(pill("ROOT", "is-root"));
        if (d.is_tip) flags.appendChild(pill("TIP", "is-tip"));
        if (!d.is_root && !d.is_tip) flags.appendChild(pill("INTERNAL", ""));

        renderLinkList(document.getElementById("d-tree-parents"), d.tree_parents);

        const subBox = document.getElementById("d-subtrees");
        subBox.innerHTML = "";
        if (d.subtrees.length === 0) {
            subBox.innerHTML = `<p class="muted">No subtree participation.</p>`;
        }
        for (const st of d.subtrees) {
            const card = document.createElement("div");
            card.className = "subtree-card";
            const heightLabel = (st.height === null || st.height === undefined)
                ? "inherits tree height"
                : `h=${st.height}`;
            card.innerHTML = `
                <div class="subtree-name">${escapeHtml(st.name)}</div>
                <div class="subtree-meta">${heightLabel} · ${st.data_byte_size} bytes</div>
            `;
            const parents = document.createElement("ul");
            parents.className = "link-list";
            renderLinkList(parents, st.parents);
            card.appendChild(parents);
            if (st.data_text) {
                const pre = document.createElement("pre");
                pre.className = "mono code";
                pre.textContent = truncate(st.data_text, 4000);
                card.appendChild(pre);
            } else if (st.data_byte_size > 0) {
                const note = document.createElement("p");
                note.className = "muted";
                note.textContent = "Binary payload (not UTF-8 decodable).";
                card.appendChild(note);
            } else {
                const note = document.createElement("p");
                note.className = "muted";
                note.textContent = "No payload (this entry only contributes parent linkage).";
                card.appendChild(note);
            }
            subBox.appendChild(card);
        }

        document.getElementById("d-sig").textContent = JSON.stringify(d.sig, null, 2);
    }

    function renderLinkList(ul, ids) {
        ul.innerHTML = "";
        if (!ids || ids.length === 0) {
            const li = document.createElement("li");
            li.className = "empty";
            li.textContent = "(none)";
            ul.appendChild(li);
            return;
        }
        for (const id of ids) {
            const li = document.createElement("li");
            const a = document.createElement("a");
            a.textContent = id;
            a.addEventListener("click", () => selectNode(id));
            li.appendChild(a);
            ul.appendChild(li);
        }
    }

    function pill(text, cls) {
        const span = document.createElement("span");
        span.className = "flag-pill " + cls;
        span.textContent = text;
        return span;
    }

    function escapeHtml(s) {
        return String(s).replace(/[&<>"']/g, (c) => ({
            "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;"
        })[c]);
    }
    function truncate(s, max) {
        return s.length > max ? s.slice(0, max) + "\n… (" + (s.length - max) + " bytes truncated)" : s;
    }

    // ----- Filtering / highlighting -----

    function applyEdgeFilters() {
        const showTree = treeToggle.checked;
        const showSubtree = subtreeToggle.checked;
        const highlightSubtree = subtreeSelect.value || null;
        const tipsOnly = tipsOnlyToggle.checked;

        let visibleNodes = null;
        if (tipsOnly && graph) {
            visibleNodes = computeTipsAndAncestors(graph);
        }

        for (const { el, edge } of edgeEls) {
            const isTree = edge.kind === "tree_parent";
            const isSubtree = edge.kind === "subtree_parent";
            const isOther = !isTree && !isSubtree;

            let visible = true;
            if (isTree && !showTree) visible = false;
            if (isSubtree && !showSubtree) visible = false;
            if (visibleNodes && (!visibleNodes.has(edge.from) || !visibleNodes.has(edge.to))) {
                visible = false;
            }
            el.classList.toggle("hidden", !visible);

            // Faded vs highlighted (dim non-matching subtree edges).
            el.classList.remove("faded", "highlighted");
            if (highlightSubtree) {
                if (edge.subtree === highlightSubtree) el.classList.add("highlighted");
                else el.classList.add("faded");
            }
            if (selectedId && (edge.from === selectedId || edge.to === selectedId)) {
                el.classList.remove("faded");
                el.classList.add("highlighted");
            }
            if (isOther) {
                // Always render unknown future kinds; they don't participate in
                // the standard tree/subtree toggle but the user can still see
                // they exist.
                el.classList.remove("hidden");
            }
        }

        for (const [id, el] of nodeEls) {
            const visible = !visibleNodes || visibleNodes.has(id);
            el.classList.toggle("faded", !visible);
        }
    }

    function computeTipsAndAncestors(g) {
        const parents = new Map();
        for (const e of g.edges) {
            if (e.kind !== "tree_parent") continue;
            if (!parents.has(e.from)) parents.set(e.from, []);
            parents.get(e.from).push(e.to);
        }
        const seen = new Set();
        const stack = g.nodes.filter(n => n.kind === "entry" && n.is_tip).map(n => n.id);
        while (stack.length) {
            const id = stack.pop();
            if (seen.has(id)) continue;
            seen.add(id);
            for (const p of (parents.get(id) || [])) stack.push(p);
        }
        return seen;
    }

    // ----- Pan + zoom + fit -----

    function bindControls() {
        // Pan
        let dragging = false;
        let last = { x: 0, y: 0 };
        svg.addEventListener("mousedown", (e) => {
            if (e.target.closest(".node")) return; // let the node handle clicks
            dragging = true;
            last = { x: e.clientX, y: e.clientY };
            svg.classList.add("panning");
        });
        window.addEventListener("mousemove", (e) => {
            if (!dragging) return;
            view.x += (e.clientX - last.x);
            view.y += (e.clientY - last.y);
            last = { x: e.clientX, y: e.clientY };
            applyView();
        });
        window.addEventListener("mouseup", () => {
            dragging = false;
            svg.classList.remove("panning");
        });

        // Zoom around mouse position
        svg.addEventListener("wheel", (e) => {
            e.preventDefault();
            const rect = svg.getBoundingClientRect();
            const mx = e.clientX - rect.left;
            const my = e.clientY - rect.top;
            const worldX = (mx - view.x) / view.k;
            const worldY = (my - view.y) / view.k;
            const factor = Math.exp(-e.deltaY * 0.0015);
            view.k = Math.max(0.05, Math.min(8, view.k * factor));
            view.x = mx - worldX * view.k;
            view.y = my - worldY * view.k;
            applyView();
        }, { passive: false });

        // Click on empty canvas = clear selection
        svg.addEventListener("click", () => {
            for (const el of nodeEls.values()) el.classList.remove("selected");
            selectedId = null;
            detailsEmpty.hidden = false;
            detailsBody.hidden = true;
            applyEdgeFilters();
        });

        treeToggle.addEventListener("change", applyEdgeFilters);
        subtreeToggle.addEventListener("change", applyEdgeFilters);
        tipsOnlyToggle.addEventListener("change", applyEdgeFilters);
        subtreeSelect.addEventListener("change", applyEdgeFilters);
        document.getElementById("btn-fit").addEventListener("click", fitToView);
        document.getElementById("btn-reset-highlight").addEventListener("click", () => {
            subtreeSelect.value = "";
            tipsOnlyToggle.checked = false;
            for (const el of nodeEls.values()) el.classList.remove("selected");
            selectedId = null;
            applyEdgeFilters();
        });
    }

    function fitToView() {
        if (positions.size === 0) return;
        let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
        for (const p of positions.values()) {
            if (p.x < minX) minX = p.x;
            if (p.y < minY) minY = p.y;
            if (p.x > maxX) maxX = p.x;
            if (p.y > maxY) maxY = p.y;
        }
        const w = (maxX - minX) + 2 * MARGIN;
        const h = (maxY - minY) + 2 * MARGIN;
        const rect = svg.getBoundingClientRect();
        const k = Math.min(rect.width / w, rect.height / h, 2);
        view.k = k;
        const cx = (minX + maxX) / 2;
        const cy = (minY + maxY) / 2;
        view.x = rect.width / 2 - cx * k;
        view.y = rect.height / 2 - cy * k;
        applyView();
    }
})();
