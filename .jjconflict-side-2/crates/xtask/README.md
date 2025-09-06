# Eidetica Development Tools (xtask)

Development automation tasks for the Eidetica project using the `cargo xtask` pattern.

This CLI mirrors the justfile interface, so `xtask <cmd>` and `just <cmd>` work consistently.

## Available Commands

Run `xtask --help` or `xtask list` to see all available commands:

```
xtask build [mode]        Build the project (debug/release)
xtask test [variant]      Run tests (sqlite/postgres/all-backends/doc/full/ignored/minimal/todo/<filter>)
xtask lint [tools...]     Run linters (clippy/audit/typos/udeps/min-versions/all)
xtask fmt                 Format code (cargo fmt + alejandra + prettier + typos)
xtask sanitize [targets]  Run sanitizers (miri/careful/asan/tsan/lsan/all)
xtask doc [action]        Documentation (api/api-full/book/serve/test/clean/links/links-online/stats)
xtask coverage [backend]  Code coverage (inmemory/sqlite/postgres/all/ignored)
xtask ci [mode]           CI pipeline (local/full/nix)
xtask nix [action]        Nix commands (build/check/integration/full)
xtask container [type]    Build container (docker/nix)
xtask bench               Run benchmarks

xtask dev                 Quick feedback: build + test + clippy
xtask fix                 Auto-fix: clippy --fix + format

xtask list                Show available commands
xtask completions <shell> Generate shell completions
```

## Usage Examples

### Build and test:

```bash
cargo xtask build           # Debug build
cargo xtask build release   # Release build
cargo xtask test            # Run all tests
cargo xtask test sqlite     # Test with SQLite backend
```

### Code quality:

```bash
cargo xtask lint            # Run clippy + audit
cargo xtask lint all        # Run all linters
cargo xtask fmt             # Format all code
cargo xtask fix             # Auto-fix + format
```

### Development workflow:

```bash
cargo xtask dev             # Quick feedback (build + test + lint)
cargo xtask ci              # Full local CI
```

## Options

All commands support:

- `--dry-run`: Show what would run without executing
- `--verbose` / `-v`: Show command output in real-time

## Shell Completions

Generate shell completions for your shell:

```bash
cargo xtask completions bash > ~/.local/share/bash-completion/completions/xtask
cargo xtask completions zsh > ~/.zfunc/_xtask
cargo xtask completions fish > ~/.config/fish/completions/xtask.fish
```

## Architecture

The xtask binary is self-contained in `src/main.rs` with a simple, direct implementation:

- Uses clap for CLI parsing with subcommands and positional arguments
- Async execution with tokio for running shell commands
- Progress display with indicatif for streaming output

Built using the `cargo xtask` pattern for fast, reliable development automation.
