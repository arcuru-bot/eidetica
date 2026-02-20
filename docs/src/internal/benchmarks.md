# Benchmarks

Eidetica uses [Criterion](https://github.com/bheisler/criterion.rs) for benchmarking. Benchmarks measure local database operations (no sync) across different store types, database sizes, and workload patterns.

## Running Benchmarks

### Quick Reference

| Command                     | Scope                               |
| --------------------------- | ----------------------------------- |
| `just bench`                | Default benchmarks                  |
| `just bench-all`            | All benchmarks (default + extended) |
| `nix build .#bench.default` | Default benchmarks (hermetic)       |
| `nix build .#bench.all`     | All benchmarks (hermetic)           |
| `nix run .#bench`           | Default benchmarks (interactive)    |
| `nix run .#bench-all`       | All benchmarks (interactive)        |

The `just` commands use the sqlite backend by default. Hermetic Nix builds produce reproducible results suitable for CI comparisons. Interactive runners accept additional Criterion arguments.

### Running a Specific Benchmark

Filter by benchmark name using Criterion's argument passthrough:

```bash
# Via cargo (most flexible)
cargo bench --bench scale_benchmarks --all-features -- "docstore_write"

# Via nix interactive runner
nix run .#bench -- "table_read"
```

### Choosing a Backend

Benchmarks respect the `TEST_BACKEND` environment variable:

```bash
TEST_BACKEND=inmemory cargo bench --bench scale_benchmarks --all-features
TEST_BACKEND=sqlite cargo bench --bench scale_benchmarks --all-features
```

The default is `sqlite` (in-memory SQLite via SQLx).

## Benchmark Organization

Benchmarks live in `crates/lib/benches/` and are split into separate binaries:

| Binary                   | Content                                                  |
| ------------------------ | -------------------------------------------------------- |
| `benchmarks`             | Core operations (entry creation, DAG traversal)          |
| `backend_benchmarks`     | Storage backend operations (read/write/tip tracking)     |
| `table_cache_benchmarks` | Table store caching behavior                             |
| `scale_benchmarks`       | Scaling benchmarks with default parameters (up to 1000)  |
| `extended_benchmarks`    | Scaling benchmarks with extended parameters (up to 5000) |
| `iroh_benchmarks`        | Iroh networking benchmarks                               |

### Default vs Extended

The **default** set (`benchmarks`, `backend_benchmarks`, `table_cache_benchmarks`, `scale_benchmarks`) runs during `just bench`, CI, and `nix build .#bench.default`. These use moderate parameters to keep wall-clock time reasonable.

The **extended** set adds `extended_benchmarks`, which runs the same benchmark logic as `scale_benchmarks` but with larger parameters (database sizes up to 5000, batch sizes up to 1000). These are excluded from routine CI runs due to their longer execution time. Run them with `just bench-all` or `nix build .#bench.all`.

Both binaries share their implementation through a common `bench_lib` module. The thin wrappers in each binary pass different parameter arrays to the shared implementations.

## Scale Benchmarks

The `scale_benchmarks` and `extended_benchmarks` binaries cover these categories:

### Write Scaling

- **DocStore single write**: Insert one entry into databases of varying sizes. Measures how write cost scales with existing data.
- **DocStore batch write**: Insert N entries in a single transaction. Throughput measured per entry.
- **Table single insert**: Same as DocStore but for Table (record-oriented) stores.
- **Table batch insert**: Batch record insertion in a single transaction.

### Read Scaling

- **DocStore single read**: Read a single key from databases of varying sizes.
- **DocStore get_all**: Read all key-value pairs. Throughput measured per entry.
- **DocStore contains_key**: Key existence check (both hit and miss cases).
- **Table single get**: Read a single record by key.
- **Table search**: Full-scan search with predicate (broad match and selective ~10% match).

### Update and Delete

- **DocStore overwrite**: Overwrite an existing key at varying database sizes.
- **Table set_existing**: Update an existing record at varying sizes.
- **DocStore delete**: Delete a key at varying sizes.
- **Table delete**: Delete a record at varying sizes.

### Operational

- **Transaction granularity**: Fixed total work split across 1 to N transactions. Reveals per-transaction overhead.
- **Multi-store transaction**: Single transaction accessing 1 to 8 stores.
- **Store viewer**: Read-only viewer creation and read at varying sizes (DocStore and Table).
- **Database lifecycle**: Single and multi-database creation overhead.
- **Mixed workload**: Interleaved reads and writes in read-heavy (90/10), write-heavy (10/90), and balanced (50/50) patterns.
- **Incremental growth**: Amortized write cost into a continuously growing database (unlike write scaling which uses fresh databases).
- **Entry history**: `get_all_entries` and `get_tips` performance at scale.

### Parameter Sets

| Category              | Default (`scale_benchmarks`) | Extended (`extended_benchmarks`) |
| --------------------- | ---------------------------- | -------------------------------- |
| Write/read/update/del | 0-1000 entries               | 2000, 5000 entries               |
| Batch sizes           | 10, 50, 100, 200             | 500, 1000                        |
| Transaction splits    | 50 entries across 1-50 txns  | 200 entries across 1-200 txns    |
| Mixed workload        | 200 entry database           | 1000 entry database              |
| Incremental growth    | 0, 100, 500 initial          | 1000, 2000 initial               |
| Entry history         | 100, 500, 1000               | 2000, 5000                       |

Extended benchmark group names are prefixed with `ext/` in Criterion reports to distinguish them from default results.

## Viewing Results

After running benchmarks, Criterion generates HTML reports:

```bash
# Open the report (just bench does this automatically)
xdg-open target/criterion/report/index.html
```

The report shows timing distributions, throughput, and comparisons against previous runs. Criterion stores baseline data in `target/criterion/` so subsequent runs show regressions or improvements.

## CI Integration

The [benchmarks.yml](https://github.com/arcuru/eidetica/blob/main/.github/workflows/benchmarks.yml) workflow runs default benchmarks on every push to `main` and on pull requests. Results are tracked via [Bencher](https://bencher.dev/) for performance regression detection. PR benchmarks compare against the `main` baseline.
