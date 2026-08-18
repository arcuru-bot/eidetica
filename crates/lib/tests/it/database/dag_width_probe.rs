//! PROBE (throwaway, not for landing): DAG-width measurement.
//!
//! Gates the SEA 2025 chain-decomposition reachability index (Bulteau, David,
//! Horn, Tran-Girard, doi:10.4230/LIPIcs.SEA.2025.9): its index size is
//! O(W) ints/node where W is the chain count of an online chain decomposition
//! of the store DAG, and W scales with the DAG's width (SEA measured W ≈
//! 1.85x width on average, ≤ 2.53x). Git-width histories (~16k) make it
//! prohibitive; Mercurial-width (~151) is trivial. Decision rule from the
//! elf-117 report: width in the low hundreds → attractive; thousands → reject.
//!
//! Builds the two gated workloads through the real transaction path, extracts
//! the store DAG each produces (`subtree_parents("data")`, the `store_parents`
//! edges `find_merge_base` walks), and measures:
//!
//! - exact width — Dilworth minimum chain cover over the transitive closure,
//!   computed as n − maximum bipartite matching (Kuhn's algorithm);
//! - online chain count — first-fit in topological order, the up-growing
//!   online scheme the SEA index builds on (SEA §2 defers the selection rule
//!   to Felsner 1997 / Bosek et al. survey Thm 3.5; first-fit is the
//!   canonical member of that family). This is the W that would drive memory.
//!
//! A third self-labeled calibration workload varies the concurrent-writer
//! count, because width is expected to track writers, not depth.

use std::collections::{HashMap, VecDeque};

use eidetica::{Snapshot, entry::Entry, entry::ID, store::DocStore};

use crate::helpers::setup_tree;

// =====================================================================
// DAG model + width algorithms
// =====================================================================

/// A DAG over compact indices, built from real entries or synthetic edges.
struct Dag {
    ids: Vec<ID>,
    parents: Vec<Vec<usize>>,
    /// Parent IDs referenced but absent from the node set (must be 0 when the
    /// node set is an ancestry closure).
    dropped_foreign_parents: usize,
}

impl Dag {
    fn from_entries(entries: &[Entry], store: &str) -> Self {
        let index: HashMap<ID, usize> =
            entries.iter().enumerate().map(|(i, e)| (e.id(), i)).collect();
        let mut dropped_foreign_parents = 0;
        let mut parents = Vec::with_capacity(entries.len());
        for e in entries {
            let store_parents = e.subtree_parents(store).expect("subtree parents");
            let mut local: Vec<usize> = store_parents
                .iter()
                .filter_map(|p| index.get(p).copied())
                .collect();
            dropped_foreign_parents += store_parents.len() - local.len();
            local.sort_unstable();
            local.dedup();
            parents.push(local);
        }
        Self {
            ids: entries.iter().map(|e| e.id()).collect(),
            parents,
            dropped_foreign_parents,
        }
    }

    /// Synthetic DAG for algorithm self-tests: `parents[i]` are indices.
    fn raw(parents: Vec<Vec<usize>>) -> Self {
        Self {
            ids: (0..parents.len()).map(|i| ID::from_bytes(&i.to_le_bytes())).collect(),
            parents,
            dropped_foreign_parents: 0,
        }
    }

    fn from_main_tree(entries: &[Entry]) -> Self {
        let index: HashMap<ID, usize> =
            entries.iter().enumerate().map(|(i, e)| (e.id(), i)).collect();
        let mut dropped_foreign_parents = 0;
        let mut parents = Vec::with_capacity(entries.len());
        for e in entries {
            let main_parents = e.parents().expect("main tree parents");
            let mut local: Vec<usize> = main_parents
                .iter()
                .filter_map(|p| index.get(p).copied())
                .collect();
            dropped_foreign_parents += main_parents.len() - local.len();
            local.sort_unstable();
            local.dedup();
            parents.push(local);
        }
        Self {
            ids: entries.iter().map(|e| e.id()).collect(),
            parents,
            dropped_foreign_parents,
        }
    }

    fn n(&self) -> usize {
        self.parents.len()
    }

    fn direct_edges(&self) -> usize {
        self.parents.iter().map(|p| p.len()).sum()
    }

    fn roots(&self) -> usize {
        self.parents.iter().filter(|p| p.is_empty()).count()
    }

    /// Kahn topological order (parents before children).
    fn topo(&self) -> Vec<usize> {
        let n = self.n();
        let mut children: Vec<Vec<usize>> = vec![Vec::new(); n];
        let mut indegree = vec![0usize; n];
        for (v, ps) in self.parents.iter().enumerate() {
            indegree[v] = ps.len();
            for &p in ps {
                children[p].push(v);
            }
        }
        let mut queue: VecDeque<usize> = (0..n).filter(|&v| indegree[v] == 0).collect();
        let mut order = Vec::with_capacity(n);
        while let Some(v) = queue.pop_front() {
            order.push(v);
            for &c in &children[v] {
                indegree[c] -= 1;
                if indegree[c] == 0 {
                    queue.push_back(c);
                }
            }
        }
        assert_eq!(order.len(), n, "graph must be acyclic");
        order
    }

    /// Proper-ancestor bitsets, computed over the topological order.
    fn ancestors(&self, topo: &[usize]) -> Vec<Vec<u64>> {
        let n = self.n();
        let words = n.div_ceil(64);
        let mut anc = vec![vec![0u64; words]; n];
        for &v in topo {
            for &p in &self.parents[v] {
                // anc[v] |= {p} ∪ anc[p]
                for w in 0..words {
                    anc[v][w] |= anc[p][w];
                }
                anc[v][p / 64] |= 1u64 << (p % 64);
            }
        }
        anc
    }

    /// Exact width: Dilworth minimum chain cover = n − max matching over the
    /// transitive-closure bipartite graph (left/right copies of each node,
    /// edge u→v when u is a proper ancestor of v).
    fn width_exact(&self, anc: &[Vec<u64>]) -> usize {
        let n = self.n();
        // descendants[u] = { v : u ∈ anc[v] }
        let mut descendants: Vec<Vec<usize>> = vec![Vec::new(); n];
        for v in 0..n {
            for u in 0..n {
                if anc[v][u / 64] >> (u % 64) & 1 == 1 {
                    descendants[u].push(v);
                }
            }
        }
        let mut match_right: Vec<Option<usize>> = vec![None; n];
        let mut matched = 0usize;
        for u in 0..n {
            let mut visited = vec![false; n];
            if kuhn(u, &descendants, &mut match_right, &mut visited) {
                matched += 1;
            }
        }
        n - matched
    }

    /// Online first-fit chain count in topological (up-growing) order: each
    /// node joins the first chain whose top is a proper ancestor, else opens a
    /// new chain. Returns the chain count — the SEA index's W.
    fn first_fit_chains(&self, topo: &[usize], anc: &[Vec<u64>]) -> usize {
        let mut chain_tops: Vec<usize> = Vec::new();
        for &v in topo {
            let mut placed = false;
            for top in chain_tops.iter_mut() {
                if anc[v][*top / 64] >> (*top % 64) & 1 == 1 {
                    *top = v;
                    placed = true;
                    break;
                }
            }
            if !placed {
                chain_tops.push(v);
            }
        }
        chain_tops.len()
    }
}

fn kuhn(
    u: usize,
    descendants: &[Vec<usize>],
    match_right: &mut [Option<usize>],
    visited: &mut [bool],
) -> bool {
    for &v in &descendants[u] {
        if visited[v] {
            continue;
        }
        visited[v] = true;
        if match_right[v].is_none() || kuhn(match_right[v].unwrap(), descendants, match_right, visited)
        {
            match_right[v] = Some(u);
            return true;
        }
    }
    false
}

/// One measurement over a DAG, with a label for the printed line.
fn measure(label: &str, dag: &Dag) -> (usize, usize) {
    let topo = dag.topo();
    let anc = dag.ancestors(&topo);
    let width = dag.width_exact(&anc);
    let chains = dag.first_fit_chains(&topo, &anc);
    println!(
        "WIDTH {label}: nodes={} direct_edges={} roots={} dropped_foreign_parents={} \
         exact_width={} online_first_fit_chains={} per_node_index_ints={}",
        dag.n(),
        dag.direct_edges(),
        dag.roots(),
        dag.dropped_foreign_parents,
        width,
        chains,
        chains,
    );
    (width, chains)
}

// =====================================================================
// Algorithm self-tests on known graphs (a probe that cannot detect a
// positive proves nothing)
// =====================================================================

#[test]
fn dag_width_probe_algorithms_selftest() {
    // Two disjoint 3-chains: width 2.
    let dag: Dag = Dag::raw(vec![vec![], vec![0], vec![1], vec![], vec![3], vec![4]]);
    assert_eq!(measure("selftest_two_disjoint_chains", &dag), (2, 2));

    // V (a→b, a→c): width 2.
    let dag = Dag::raw(vec![vec![], vec![0], vec![0]]);
    assert_eq!(measure("selftest_v", &dag), (2, 2));

    // Diamond (a→b, a→c, b→d, c→d): width 2 (antichain {b,c}).
    let dag = Dag::raw(vec![vec![], vec![0], vec![0], vec![1, 2]]);
    assert_eq!(measure("selftest_diamond", &dag), (2, 2));

    // Transitive edge a→d added to a→b→c→d: width still 1 (closure-based).
    let dag = Dag::raw(vec![vec![], vec![0], vec![1], vec![2], vec![0, 3]]);
    assert_eq!(measure("selftest_transitive_edge", &dag).0, 1);

    // 8-way fork from one root, then one merge of all 8 tips: width 8.
    let mut parents: Vec<Vec<usize>> = vec![vec![]];
    for _ in 0..8 {
        parents.push(vec![0]);
    }
    let tips: Vec<usize> = (1..=8).collect();
    parents.push(tips.clone());
    let dag = Dag::raw(parents);
    assert_eq!(measure("selftest_eight_way_fork", &dag).0, 8);

    // Ladder (the sustained criss-cross shape): width 2.
    // root → l0, r0; then (l_i, r_i) → l_{i+1}, r_{i+1}.
    let mut parents: Vec<Vec<usize>> = vec![vec![], vec![0], vec![0]];
    for i in 0..40 {
        let l = 1 + 2 * i;
        let r = 2 + 2 * i;
        parents.push(vec![l, r]); // l_{i+1}
        parents.push(vec![l, r]); // r_{i+1}
    }
    let dag = Dag::raw(parents);
    assert_eq!(measure("selftest_ladder_40", &dag), (2, 2));
}

// =====================================================================
// Workload builders
// =====================================================================

async fn write_at(tree: &eidetica::Database, tips: &[ID], key: &str, value: &str) -> ID {
    let txn = tree
        .new_transaction_at(&Snapshot::from(tips.to_vec()))
        .await
        .expect("create transaction at tips");
    txn.get_store::<DocStore>("data")
        .await
        .expect("open data store")
        .set(key, value)
        .await
        .expect("write key");
    txn.commit().await.expect("commit")
}

/// elf-096's criss-cross bench topology: two independent store roots (genesis
/// fork, each side's first write roots the store), each chain extended `depth`
/// entries, then two separate merges of the same tip pair — common ancestors
/// exist, none dominates every path. This is the shape whose `find_merge_base`
/// None-detection cost ~1.5 s at depth 100.
async fn build_criss_cross_bench(tree: &eidetica::Database, depth: usize) -> (ID, ID) {
    let genesis = tree
        .snapshot()
        .await
        .expect("snapshot")
        .into_tips()
        .into_iter()
        .next()
        .expect("genesis tip");

    // Two independent first writes to "data" on either side of the fork.
    let mut tip1 = write_at(tree, &[genesis.clone()], "root1", "v").await;
    let mut tip2 = write_at(tree, &[genesis], "root2", "v").await;

    for i in 0..depth {
        tip1 = write_at(tree, &[tip1], &format!("c1_{i}"), "v").await;
        tip2 = write_at(tree, &[tip2], &format!("c2_{i}"), "v").await;
    }

    // Two separate merges of the same pair of tips: the criss-cross.
    let m1 = write_at(tree, &[tip1.clone(), tip2.clone()], "merge1", "v").await;
    let m2 = write_at(tree, &[tip1, tip2], "merge2", "v").await;
    (m1, m2)
}

/// The sustained-concurrency workload from the Segment-Product design
/// measurements: `writers` concurrent writers, `rounds` rounds; each round
/// every writer commits at the previous round's full tip set, then the merged
/// state is read. writers=2, rounds=40 is the probe the design doc measured.
async fn build_concurrent_writers(
    tree: &eidetica::Database,
    writers: usize,
    rounds: usize,
) -> Vec<ID> {
    // One shared store root, committed at the current frontier (as in the
    // measured probe: `tree.new_transaction()` for the seed write).
    let root = {
        let txn = tree.new_transaction().await.expect("seed txn");
        txn.get_store::<DocStore>("data")
            .await
            .expect("data store")
            .set("seed", "0")
            .await
            .expect("write seed");
        txn.commit().await.expect("commit seed")
    };

    // Initial fork: one tip per writer.
    let mut tips: Vec<ID> = Vec::with_capacity(writers);
    for w in 0..writers {
        tips.push(write_at(tree, std::slice::from_ref(&root), &format!("w{w}"), "init").await);
    }

    for r in 0..rounds {
        let base = tips.clone();
        for w in 0..writers {
            tips[w] = write_at(tree, &base, &format!("w{w}_r{r}"), "v").await;
        }
        // One frontier read per round, as in the measured workload.
        let txn = tree
            .new_transaction_at(&Snapshot::from(tips.clone()))
            .await
            .expect("read transaction");
        let state = txn
            .get_store::<DocStore>("data")
            .await
            .expect("store for read")
            .get_all()
            .await
            .expect("read merged state");
        assert!(state.get("seed").is_some(), "converged state must retain seed");
    }
    tips
}

/// The disjoint-roots variant of the 40-round workload: the store is created
/// independently on both sides of a fork (seed written to a different store),
/// so the store history has two roots and no common ancestor at all.
async fn build_disjoint_roots_40_rounds(tree: &eidetica::Database) -> (ID, ID) {
    let seed = {
        let txn = tree.new_transaction().await.expect("seed txn");
        txn.get_store::<DocStore>("other")
            .await
            .expect("other store")
            .set("seed", "0")
            .await
            .expect("write seed");
        txn.commit().await.expect("commit seed")
    };
    let fork = Snapshot::from(std::slice::from_ref(&seed));

    let mut left = {
        let txn = tree.new_transaction_at(&fork).await.expect("left root txn");
        txn.get_store::<DocStore>("data")
            .await
            .expect("data store")
            .set("left", "seed")
            .await
            .expect("write left");
        txn.commit().await.expect("commit left root")
    };
    let mut right = {
        let txn = tree.new_transaction_at(&fork).await.expect("right root txn");
        txn.get_store::<DocStore>("data")
            .await
            .expect("data store")
            .set("right", "seed")
            .await
            .expect("write right");
        txn.commit().await.expect("commit right root")
    };

    for i in 0..40 {
        let tips = Snapshot::from(vec![left.clone(), right.clone()]);
        let new_left = {
            let txn = tree.new_transaction_at(&tips).await.expect("left txn");
            txn.get_store::<DocStore>("data")
                .await
                .expect("data store")
                .set(&format!("left_{i}"), "v")
                .await
                .expect("write left");
            txn.commit().await.expect("commit left")
        };
        let new_right = {
            let txn = tree.new_transaction_at(&tips).await.expect("right txn");
            txn.get_store::<DocStore>("data")
                .await
                .expect("data store")
                .set(&format!("right_{i}"), "v")
                .await
                .expect("write right");
            txn.commit().await.expect("commit right")
        };
        left = new_left;
        right = new_right;

        let read_tips = Snapshot::from(vec![left.clone(), right.clone()]);
        let txn = tree.new_transaction_at(&read_tips).await.expect("read txn");
        let state = txn
            .get_store::<DocStore>("data")
            .await
            .expect("data store")
            .get_all()
            .await
            .expect("read merged state");
        assert!(state.get("left").is_some() && state.get("right").is_some());
    }
    (left, right)
}

// =====================================================================
// The measurements
// =====================================================================

async fn extract_store_dag(tree: &eidetica::Database, tips: &[ID]) -> Dag {
    let entries = tree
        .get_store_entries("data", tips, Default::default())
        .await
        .expect("store entries at tips");
    assert!(!entries.is_empty(), "store_at returned no entries");
    Dag::from_entries(&entries, "data")
}

/// Workload (a): the elf-096 criss-cross bench topology at depths 10 and 100.
#[tokio::test]
async fn dag_width_probe_criss_cross_bench() {
    for depth in [10usize, 100] {
        // Fresh tree per depth: tree.snapshot() must return genesis, not the
        // previous iteration's frontier.
        let (_instance, tree) = setup_tree().await;
        let (m1, m2) = build_criss_cross_bench(&tree, depth).await;

        // Topology pin: no dominator over the two merge tips — the case the
        // 1.5 s measurement was about. On the small depth this is cheap.
        if depth == 10 {
            let engine = tree
                .backend()
                .expect("backend")
                .local_engine()
                .expect("local engine");
            let mb = engine
                .find_merge_base(tree.root_id(), "data", &[m1.clone(), m2.clone()])
                .await
                .expect("find_merge_base must resolve");
            assert_eq!(mb, None, "criss-cross must have no dominating base");
        }

        let dag = extract_store_dag(&tree, &[m1, m2]).await;
        // 2 roots + 2·depth chain entries + 2 merges.
        assert_eq!(dag.n(), 2 + 2 * depth + 2, "store node count at depth {depth}");
        assert_eq!(dag.dropped_foreign_parents, 0, "closure must be parent-complete");
        measure(&format!("criss_cross_bench depth={depth}"), &dag);
    }
}

/// Workload (b): the 40-round two-writer concurrent-store workload (both the
/// shared-root and disjoint-root variants the Segment-Product measurements
/// drove), plus the writer-count calibration.
#[tokio::test]
async fn dag_width_probe_concurrent_store_40_rounds() {
    // Shared root (criss-cross every round).
    {
        let (_instance, tree) = setup_tree().await;
        let tips = build_concurrent_writers(&tree, 2, 40).await;
        let dag = extract_store_dag(&tree, &tips).await;
        // root + 2 initial fork entries + 2·40 round entries.
        assert_eq!(dag.n(), 1 + 2 + 80, "store node count");
        assert_eq!(dag.dropped_foreign_parents, 0);
        measure("concurrent_store_40_rounds_shared_root", &dag);

        let main = tree.get_all_entries().await.expect("main tree entries");
        let main_dag = Dag::from_main_tree(&main);
        measure("concurrent_store_40_rounds_shared_root MAIN TREE", &main_dag);
    }

    // Disjoint roots.
    {
        let (_instance, tree) = setup_tree().await;
        let (left, right) = build_disjoint_roots_40_rounds(&tree).await;
        let dag = extract_store_dag(&tree, &[left, right]).await;
        // 2 independent roots + 2·40 round entries.
        assert_eq!(dag.n(), 2 + 80, "store node count");
        assert_eq!(dag.dropped_foreign_parents, 0);
        assert_eq!(dag.roots(), 2, "disjoint roots must both be present");
        measure("concurrent_store_40_rounds_disjoint_roots", &dag);
    }

    // Writer-count calibration: width vs concurrent writers, rounds fixed.
    for writers in [4usize, 8] {
        let (_instance, tree) = setup_tree().await;
        let tips = build_concurrent_writers(&tree, writers, 40).await;
        let dag = extract_store_dag(&tree, &tips).await;
        assert_eq!(dag.n(), 1 + writers + writers * 40, "store node count");
        measure(&format!("CALIBRATION writers={writers} rounds=40"), &dag);
    }
}
