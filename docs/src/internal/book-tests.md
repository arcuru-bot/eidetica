# Book Tests

The `crates/book-tests/` crate validates that every code example in the
rendered mdBook documentation actually compiles against the real
Eidetica API. Without it, code blocks in the user guide would drift
silently as the API evolves.

## How It Works

The `eidetica-book-tests` crate is a thin shell around a `build.rs`
that auto-discovers every `docs/src/**/*.md` file at build time. For
each file it generates a Rust module wrapping the entire document in a
doc comment:

```rust
#[doc = include_str!("...absolute/path...")]
mod summary { }
```

Running `cargo test --doc -p eidetica-book-tests` compiles and
optionally runs every fenced Rust code block through cargo's normal
dependency resolution. Any compilation error or failing assertion
turns into a test failure.

### Module Name Mangling

The build script converts file paths to valid Rust module names:

- `/` (directory separator) → `__` (double underscore)
- `-` (hyphen) → `_` (underscore)
- `.md` extension is stripped

For example, `docs/src/user_guide/getting-started.md` becomes the
module `user_guide__getting_started`.

Directories named `rustdoc` are skipped — they contain generated API
reference output from `cargo doc`, not book source.

### Discovery is Automatic

New markdown files in `docs/src/` are picked up without manual
registration. The build script walks the directory tree on every
build, and `cargo::rerun-if-changed` on the source directory ensures
the generated file is rebuilt when documentation changes.

## Running Book Tests

```bash
just doc test    # Run all book tests
```

The command expands to `cargo test --doc -p eidetica-book-tests`, plus
link checking. In CI, the `nix flake check` target `doc.booktest`
exercises the same path. The Nix derivation ensures the crate's
dependencies are available at build time.

## Code Block Annotations

Markdown code blocks in the book source use standard Rust annotations
to control how they are processed:

| Annotation       | Compiled? | Executed? | Use case                                    |
| ---------------- | --------- | --------- | ------------------------------------------- |
| `rust`           | yes       | yes       | Core API examples that should stay current  |
| `rust,no_run`    | yes       | no        | Examples that compile but shouldn't run     |
| `rust,ignore`    | no        | no        | Illustrations that won't compile as-is      |

**Untested illustration pattern** — when a code block carries `ignore`,
always precede it with an HTML comment explaining why:

```markdown
<!-- Code block ignored: Requires network connectivity for peer synchronization -->

``rust,ignore
use eidetica::{Instance, backend::database::Sqlite};
…
``
```

## Hidden Setup Lines

Tested examples often need imports and boilerplate that the user
doesn't need to see in the rendered book. Lines prefixed with `#` are
included in the compilation but hidden from the rendered output:

````markdown
``rust
# extern crate eidetica;
# extern crate tokio;
# use eidetica::{Instance, NewUser};
#
# #[tokio::main]
# async fn main() -> eidetica::Result<()> {
// Open an Instance by URL — `memory://` is an ephemeral in-process backend.
let (_instance, _) =
    Instance::connect_or_create("memory://", NewUser::passwordless("alice")).await?;
# Ok(())
# }
``
````

The rendered page shows only the un-commented lines — the `Instance::connect_or_create`
call — while the compiler sees the full block including `extern crate`, imports,
and the `#[tokio::main]` wrapper.

## Why Not `mdbook test -L`?

The standard `mdbook test` command accepts a `-L` flag to point the
compiler at build artifacts, but this fails with E0464 ("multiple rlib
candidates") in workspaces with complex dependency trees. When cargo
builds dependencies with different feature sets across crate targets,
the `-L` directory contains multiple `lib<name>-<hash>.rlib` files for
the same dependency, and `rustc` cannot disambiguate them.

The `eidetica-book-tests` crate avoids this entirely by compiling
through cargo — each code block sees the same dependency resolution
that the crate itself uses, matching exactly what a downstream consumer
would get.

## Integration Points

Book tests run in these contexts:

- **Local development**: `just test` includes them; `just doc test` runs them in isolation
- **CI (GitHub Actions)**: `nix flake check` builds the `doc.booktest` target
- **CI (Forgejo)**: Same Nix derivation, providing redundancy
- **Pre-push hygiene**: `just ci` includes `doc.test` among the full checks

A book test failure in CI blocks the merge, ensuring all published
documentation examples are tested against the current codebase.
