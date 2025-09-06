//! Xtask - Development automation for Eidetica
//!
//! This CLI mirrors the justfile interface, so `xtask <cmd>` and `just <cmd>`
//! work consistently.

use anyhow::Result;
use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
use clap_complete::{Shell, generate};
use std::io;
use tokio::process::Command as TokioCommand;
use xtask::GlobalOptions;
use xtask::streaming::{StreamingRunner, StreamingTask};

/// Development automation tasks for Eidetica
#[derive(Parser)]
#[command(name = "xtask")]
#[command(about = "Development automation tasks for Eidetica")]
#[command(after_help = "Run 'xtask <command> --help' for more information on a specific command.")]
struct Cli {
    /// Show what commands would be run without executing them
    #[arg(long, global = true)]
    dry_run: bool,

    /// Show verbose output including real-time command output
    #[arg(long, short, global = true)]
    verbose: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Build the project
    Build {
        /// Build mode: debug (default) or release
        #[arg(default_value = "debug")]
        mode: BuildMode,
    },

    /// Run tests
    Test {
        /// Test variant or filter. Special values: sqlite, postgres, all-backends, doc, full, ignored, minimal, todo
        /// Or provide a test filter string.
        #[arg(trailing_var_arg = true)]
        args: Vec<String>,
    },

    /// Run linter(s)
    Lint {
        /// Tools to run: clippy, audit, typos, udeps, min-versions, all
        /// Defaults to: clippy audit typos
        #[arg(default_values_t = vec!["clippy".to_string(), "audit".to_string(), "typos".to_string()])]
        tools: Vec<String>,
    },

    /// Run all formatters (cargo fmt + alejandra + prettier)
    Fmt,

    /// Run sanitizer(s) for dynamic analysis
    Sanitize {
        /// Sanitizers to run: miri, careful, asan, tsan, lsan, all
        /// Shows help if no targets specified.
        targets: Vec<String>,
    },

    /// Build documentation
    Doc {
        /// Action: api (default), api-full, book, serve, test, clean, links, links-online, stats
        #[arg(default_value = "api")]
        action: String,
    },

    /// Generate code coverage
    Coverage {
        /// Backend: inmemory (default), sqlite, postgres, all, ignored
        #[arg(default_value = "inmemory")]
        backend: String,
    },

    /// Run CI pipeline
    Ci {
        /// Mode: local (default), full, nix
        #[arg(default_value = "local")]
        mode: String,
    },

    /// Nix commands
    Nix {
        /// Action: check (default), build, integration, full
        #[arg(default_value = "check")]
        action: String,
    },

    /// Build container image
    Container {
        /// Type: docker (default), nix
        #[arg(default_value = "docker")]
        r#type: String,
    },

    /// Run benchmarks
    Bench,

    /// Quick development feedback (build + test + lint)
    Dev,

    /// Run automatic fixes (clippy fix + format)
    Fix,

    /// List available commands
    List,

    /// Generate shell completions
    Completions {
        /// Shell to generate completions for
        shell: Shell,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum, Default)]
enum BuildMode {
    #[default]
    Debug,
    Release,
}

/// Execution context for commands
struct Context {
    dry_run: bool,
    verbose: bool,
}

impl Context {
    /// Run a command with a header
    async fn run(&self, cmd: TokioCommand, description: &str) -> Result<()> {
        self.run_group(description, vec![(description.to_string(), cmd)])
            .await
    }

    /// Run multiple commands in parallel under a group header
    async fn run_group(&self, header: &str, tasks: Vec<(String, TokioCommand)>) -> Result<()> {
        if tasks.is_empty() {
            return Ok(());
        }

        let global_options = GlobalOptions {
            dry_run: self.dry_run,
            verbose: self.verbose,
        };

        let runner = StreamingRunner::new(header, &global_options);
        let streaming_tasks: Vec<_> = tasks
            .into_iter()
            .map(|(name, cmd)| StreamingTask::new(name, cmd))
            .collect();

        runner.run_streaming_tasks(streaming_tasks).await
    }

    /// Run multiple commands sequentially under a single group header
    async fn run_seq(&self, header: &str, tasks: Vec<(String, TokioCommand)>) -> Result<()> {
        if tasks.is_empty() {
            return Ok(());
        }

        if self.dry_run {
            println!("{} {}", xtask::output::DRY_RUN, header);
            for (name, _) in tasks {
                println!("  {} {}", xtask::output::DRY_RUN, name);
            }
            return Ok(());
        }

        // Print header once
        println!("{}...", header);

        let global_options = GlobalOptions {
            dry_run: self.dry_run,
            verbose: self.verbose,
        };

        // Run each task sequentially, using empty header to skip header print
        for (name, cmd) in tasks {
            let runner = StreamingRunner::new(&name, &global_options);
            runner
                .run_streaming_tasks(vec![StreamingTask::new(name, cmd)])
                .await?;
        }
        Ok(())
    }
}

#[tokio::main]
async fn main() {
    let exit_code = match run().await {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("Error: {}", e);
            1
        }
    };
    std::process::exit(exit_code);
}

async fn run() -> Result<()> {
    let cli = Cli::parse();

    // Verify we're in the project root
    if !std::path::Path::new("Cargo.toml").exists() {
        anyhow::bail!(
            "Expected to be in project root with Cargo.toml, but current directory is: {:?}",
            std::env::current_dir()?
        );
    }

    let ctx = Context {
        dry_run: cli.dry_run,
        verbose: cli.verbose,
    };

    match cli.command {
        Commands::Build { mode } => cmd_build(&ctx, mode).await,
        Commands::Test { args } => cmd_test(&ctx, args).await,
        Commands::Lint { tools } => cmd_lint(&ctx, tools).await,
        Commands::Fmt => cmd_fmt(&ctx).await,
        Commands::Sanitize { targets } => cmd_sanitize(&ctx, targets).await,
        Commands::Doc { action } => cmd_doc(&ctx, &action).await,
        Commands::Coverage { backend } => cmd_coverage(&ctx, &backend).await,
        Commands::Ci { mode } => cmd_ci(&ctx, &mode).await,
        Commands::Nix { action } => cmd_nix(&ctx, &action).await,
        Commands::Container { r#type } => cmd_container(&ctx, &r#type).await,
        Commands::Bench => cmd_bench(&ctx).await,
        Commands::Dev => cmd_dev(&ctx).await,
        Commands::Fix => cmd_fix(&ctx).await,
        Commands::List => cmd_list(),
        Commands::Completions { shell } => generate_completions(shell),
    }
}

// =============================================================================
// Build
// =============================================================================

async fn cmd_build(ctx: &Context, mode: BuildMode) -> Result<()> {
    let mut cmd = TokioCommand::new("cargo");
    cmd.arg("build")
        .arg("--workspace")
        .arg("--all-targets")
        .arg("--all-features");

    if matches!(mode, BuildMode::Release) {
        cmd.arg("--release");
    }

    ctx.run(cmd, "Building project").await
}

// =============================================================================
// Test
// =============================================================================

async fn cmd_test(ctx: &Context, args: Vec<String>) -> Result<()> {
    if args.is_empty() {
        // Default: run main tests (sqlite backend)
        return run_nextest_with_backend(ctx, "sqlite", &[]).await;
    }

    let first = args[0].as_str();
    let rest: Vec<&str> = args[1..].iter().map(|s| s.as_str()).collect();

    match first {
        "sqlite" => run_nextest_with_backend(ctx, "sqlite", &rest).await,
        "postgres" => run_nextest_with_postgres(ctx, &rest).await,
        "all-backends" => {
            run_nextest(ctx, None).await?;
            run_nextest_with_backend(ctx, "sqlite", &[]).await?;
            run_nextest_with_postgres(ctx, &[]).await
        }
        "doc" => run_doc_tests(ctx).await,
        "full" => {
            // Test group: unit tests and doc tests
            let mut nextest = TokioCommand::new("cargo");
            nextest
                .arg("nextest")
                .arg("run")
                .arg("--workspace")
                .arg("--all-features")
                .arg("--no-fail-fast")
                .arg("--status-level")
                .arg("fail");

            let mut doctest = TokioCommand::new("cargo");
            doctest
                .arg("test")
                .arg("--doc")
                .arg("--workspace")
                .arg("--all-features");

            ctx.run_group(
                "Test",
                vec![
                    ("nextest".to_string(), nextest),
                    ("doctest".to_string(), doctest),
                ],
            )
            .await?;

            // Book test group
            cmd_doc(ctx, "test").await
        }
        "ignored" => run_nextest_ignored(ctx).await,
        "minimal" => run_minimal_tests(ctx).await,
        "todo" => run_todo_tests(ctx).await,
        _ => {
            // Treat as test filter
            let filter = args.join(" ");
            run_nextest(ctx, Some(&filter)).await
        }
    }
}

async fn run_nextest(ctx: &Context, filter: Option<&str>) -> Result<()> {
    let mut cmd = TokioCommand::new("cargo");
    cmd.arg("nextest")
        .arg("run")
        .arg("--workspace")
        .arg("--all-features")
        .arg("--no-fail-fast")
        .arg("--status-level")
        .arg("fail");

    if let Some(f) = filter {
        cmd.arg(f);
    }

    ctx.run(cmd, "Running tests").await
}

async fn run_nextest_with_backend(ctx: &Context, backend: &str, extra_args: &[&str]) -> Result<()> {
    let mut cmd = TokioCommand::new("cargo");
    cmd.env("TEST_BACKEND", backend)
        .arg("nextest")
        .arg("run")
        .arg("--workspace")
        .arg("--all-features")
        .arg("--no-fail-fast")
        .arg("--status-level")
        .arg("fail");

    for arg in extra_args {
        cmd.arg(arg);
    }

    ctx.run(cmd, &format!("Running tests with {} backend", backend))
        .await
}

async fn run_nextest_with_postgres(ctx: &Context, extra_args: &[&str]) -> Result<()> {
    let container_name = "eidetica-test-postgres";
    let db_name = "eidetica_test";
    let db_port = "54321";

    // Start postgres container
    start_postgres_container(ctx, container_name, db_name, db_port).await?;

    // Run tests
    let result = async {
        let mut cmd = TokioCommand::new("cargo");
        cmd.env("TEST_BACKEND", "postgres")
            .env(
                "TEST_POSTGRES_URL",
                format!("postgres://postgres@localhost:{}/{}", db_port, db_name),
            )
            .arg("nextest")
            .arg("run")
            .arg("--workspace")
            .arg("--all-features")
            .arg("--no-fail-fast")
            .arg("--status-level")
            .arg("fail");

        for arg in extra_args {
            cmd.arg(arg);
        }

        ctx.run(cmd, "Running tests against PostgreSQL").await
    }
    .await;

    // Cleanup container
    cleanup_container(container_name).await;

    result
}

async fn run_nextest_ignored(ctx: &Context) -> Result<()> {
    let mut cmd = TokioCommand::new("cargo");
    cmd.arg("nextest")
        .arg("run")
        .arg("--workspace")
        .arg("--all-features")
        .arg("--no-fail-fast")
        .arg("--status-level")
        .arg("fail")
        .arg("--run-ignored")
        .arg("all");

    ctx.run(cmd, "Running ignored tests").await
}

async fn run_doc_tests(ctx: &Context) -> Result<()> {
    let mut cmd = TokioCommand::new("cargo");
    cmd.arg("test")
        .arg("--doc")
        .arg("--workspace")
        .arg("--all-features");

    ctx.run(cmd, "Running documentation tests").await
}

async fn run_minimal_tests(ctx: &Context) -> Result<()> {
    // Build with no default features
    let mut build_cmd = TokioCommand::new("cargo");
    build_cmd
        .arg("build")
        .arg("-p")
        .arg("eidetica")
        .arg("--no-default-features");
    ctx.run(build_cmd, "Building with minimal features").await?;

    // Test with no default features + testing feature
    let mut test_cmd = TokioCommand::new("cargo");
    test_cmd
        .arg("test")
        .arg("-p")
        .arg("eidetica")
        .arg("--no-default-features")
        .arg("--features")
        .arg("testing");
    ctx.run(test_cmd, "Testing with minimal features").await
}

async fn run_todo_tests(ctx: &Context) -> Result<()> {
    let mut cmd = TokioCommand::new("./test.sh");
    cmd.current_dir("examples/todo");
    ctx.run(cmd, "Running todo example tests").await
}

// =============================================================================
// Lint
// =============================================================================

async fn cmd_lint(ctx: &Context, tools: Vec<String>) -> Result<()> {
    // Expand "all" to full list
    let expanded_tools: Vec<String> = tools
        .iter()
        .flat_map(|t| {
            if t == "all" {
                vec![
                    "clippy".to_string(),
                    "audit".to_string(),
                    "typos".to_string(),
                    "udeps".to_string(),
                    "min-versions".to_string(),
                ]
            } else {
                vec![t.clone()]
            }
        })
        .collect();

    // Separate parallel-safe tools from sequential ones
    let mut parallel_tasks: Vec<(String, TokioCommand)> = Vec::new();
    let mut has_min_versions = false;

    for tool in &expanded_tools {
        match tool.as_str() {
            "clippy" => {
                let mut cmd = TokioCommand::new("cargo");
                cmd.arg("clippy")
                    .arg("--workspace")
                    .arg("--all-targets")
                    .arg("--all-features")
                    .arg("--")
                    .arg("-D")
                    .arg("warnings");
                parallel_tasks.push(("clippy".to_string(), cmd));
            }
            "audit" => {
                let mut cmd = TokioCommand::new("cargo");
                cmd.arg("deny").arg("check");
                parallel_tasks.push(("audit".to_string(), cmd));
            }
            "typos" => {
                let mut cmd = TokioCommand::new("typos");
                cmd.arg("--config").arg(".config/typos.toml");
                parallel_tasks.push(("typos".to_string(), cmd));
            }
            "udeps" => {
                let mut cmd = TokioCommand::new("cargo");
                cmd.arg("udeps").arg("--workspace").arg("--all-targets");
                parallel_tasks.push(("udeps".to_string(), cmd));
            }
            "min-versions" => {
                has_min_versions = true;
            }
            _ => {
                eprintln!("Unknown linter: {}", tool);
                eprintln!("Options: clippy, audit, typos, udeps, min-versions, all");
                anyhow::bail!("Unknown linter: {}", tool);
            }
        }
    }

    // Run parallel-safe linters together
    if !parallel_tasks.is_empty() {
        ctx.run_group("Lint", parallel_tasks).await?;
    }

    // Run min-versions sequentially (has internal dependencies)
    if has_min_versions {
        let mut update_cmd = TokioCommand::new("cargo");
        update_cmd.arg("update").arg("-Z").arg("minimal-versions");
        ctx.run(update_cmd, "Updating to minimal versions").await?;

        let mut build_cmd = TokioCommand::new("cargo");
        build_cmd
            .arg("build")
            .arg("--workspace")
            .arg("--all-targets")
            .arg("--all-features");
        ctx.run(build_cmd, "Building with minimal versions").await?;

        let mut test_cmd = TokioCommand::new("cargo");
        test_cmd
            .arg("nextest")
            .arg("run")
            .arg("--workspace")
            .arg("--all-features")
            .arg("--status-level")
            .arg("fail");
        ctx.run(test_cmd, "Testing with minimal versions").await?;
    }

    Ok(())
}

// =============================================================================
// Format
// =============================================================================

async fn cmd_fmt(ctx: &Context) -> Result<()> {
    // Run all formatters in parallel (they operate on different file types)
    let mut tasks: Vec<(String, TokioCommand)> = Vec::new();

    // Cargo fmt (Rust)
    let mut cargo_fmt = TokioCommand::new("cargo");
    cargo_fmt.arg("fmt").arg("--all");
    tasks.push(("cargo fmt".to_string(), cargo_fmt));

    // Alejandra (Nix)
    let mut alejandra = TokioCommand::new("alejandra");
    alejandra.arg(".");
    tasks.push(("alejandra".to_string(), alejandra));

    // Prettier (web files)
    let mut prettier = TokioCommand::new("prettier");
    prettier
        .arg("--write")
        .arg(".")
        .arg("--log-level")
        .arg("warn");
    tasks.push(("prettier".to_string(), prettier));

    // Typos (fix typos)
    let mut typos = TokioCommand::new("typos");
    typos
        .arg("--write-changes")
        .arg("--config")
        .arg(".config/typos.toml");
    tasks.push(("typos".to_string(), typos));

    ctx.run_group("Format", tasks).await
}

// =============================================================================
// Sanitize
// =============================================================================

async fn cmd_sanitize(ctx: &Context, targets: Vec<String>) -> Result<()> {
    if targets.is_empty() {
        println!("Available sanitizers:");
        println!("  xtask sanitize miri     - Miri: Stacked Borrows, UB detection");
        println!("  xtask sanitize careful  - cargo-careful: extra std debug assertions");
        println!("  xtask sanitize asan     - AddressSanitizer: memory errors, use-after-free");
        println!("  xtask sanitize tsan     - ThreadSanitizer: data races");
        println!("  xtask sanitize lsan     - LeakSanitizer: memory leaks");
        println!();
        println!("  xtask sanitize all      - Run all sanitizers except miri");
        println!();
        println!("Multiple: xtask sanitize miri asan");
        return Ok(());
    }

    // Expand "all" to full list (excludes miri since it's slow)
    let expanded: Vec<String> = targets
        .iter()
        .flat_map(|t| {
            if t == "all" {
                vec![
                    "careful".to_string(),
                    "asan".to_string(),
                    "tsan".to_string(),
                    "lsan".to_string(),
                ]
            } else {
                vec![t.clone()]
            }
        })
        .collect();

    // Build all sanitizer commands
    let mut tasks: Vec<(String, TokioCommand)> = Vec::new();

    for target in &expanded {
        let cmd = match target.as_str() {
            "miri" => {
                let mut cmd = TokioCommand::new("cargo");
                cmd.arg("miri")
                    .arg("test")
                    .arg("--workspace")
                    .arg("--all-features");
                cmd
            }
            "careful" => {
                let mut cmd = TokioCommand::new("cargo");
                cmd.arg("careful")
                    .arg("test")
                    .arg("--workspace")
                    .arg("--all-features");
                cmd
            }
            "asan" => {
                let mut cmd = TokioCommand::new("cargo");
                cmd.env("RUSTFLAGS", "-Zsanitizer=address")
                    .arg("test")
                    .arg("--workspace")
                    .arg("--all-features")
                    .arg("--lib")
                    .arg("--bins")
                    .arg("--tests")
                    .arg("--examples")
                    .arg("--target")
                    .arg("x86_64-unknown-linux-gnu");
                cmd
            }
            "tsan" => {
                let cwd = std::env::current_dir()?;
                let suppressions = cwd.join(".config/tsan");
                let mut cmd = TokioCommand::new("cargo");
                cmd.env("CARGO_TARGET_DIR", "target/tsan")
                    .env(
                        "RUSTFLAGS",
                        "-Zsanitizer=thread -Zsanitizer-memory-track-origins=1",
                    )
                    .env(
                        "TSAN_OPTIONS",
                        format!("suppressions={}", suppressions.display()),
                    )
                    .env("RUSTC_BOOTSTRAP", "1")
                    .arg("test")
                    .arg("-Zbuild-std")
                    .arg("--workspace")
                    .arg("--all-features")
                    .arg("--lib")
                    .arg("--bins")
                    .arg("--tests")
                    .arg("--examples")
                    .arg("--target")
                    .arg("x86_64-unknown-linux-gnu");
                cmd
            }
            "lsan" => {
                let mut cmd = TokioCommand::new("cargo");
                cmd.env("RUSTFLAGS", "-Zsanitizer=leak")
                    .arg("test")
                    .arg("--workspace")
                    .arg("--all-features")
                    .arg("--lib")
                    .arg("--bins")
                    .arg("--tests")
                    .arg("--examples")
                    .arg("--target")
                    .arg("x86_64-unknown-linux-gnu");
                cmd
            }
            _ => {
                eprintln!("Unknown sanitizer: {}", target);
                eprintln!("Options: miri, careful, asan, tsan, lsan, all");
                anyhow::bail!("Unknown sanitizer: {}", target);
            }
        };
        tasks.push((target.clone(), cmd));
    }

    // Run all sanitizers in parallel
    ctx.run_group("Sanitize", tasks).await
}

// =============================================================================
// Doc
// =============================================================================

async fn cmd_doc(ctx: &Context, action: &str) -> Result<()> {
    match action {
        "api" => {
            let mut cmd = TokioCommand::new("cargo");
            cmd.arg("doc")
                .arg("--workspace")
                .arg("--all-features")
                .arg("--no-deps");
            ctx.run(cmd, "Building API documentation").await
        }
        "api-full" => {
            let mut cmd = TokioCommand::new("cargo");
            cmd.arg("doc").arg("--workspace").arg("--all-features");
            ctx.run(cmd, "Building full API documentation").await
        }
        "book" => {
            // Build lib docs first
            let mut doc_cmd = TokioCommand::new("cargo");
            doc_cmd
                .arg("doc")
                .arg("-p")
                .arg("eidetica")
                .arg("--all-features")
                .arg("--no-deps");

            // Run cargo doc (first part of Book group)
            ctx.run_seq("Book", vec![("cargo doc".to_string(), doc_cmd)])
                .await?;

            // Create symlink (needed before mdbook build)
            let _ = std::fs::remove_file("docs/src/rustdoc");
            std::os::unix::fs::symlink("../../target/doc", "docs/src/rustdoc")?;

            // Build mdbook (second part, but need fresh runner after symlink)
            let mut mdbook_cmd = TokioCommand::new("mdbook");
            mdbook_cmd.arg("build").arg("docs");

            // Skip header since we already printed "Book..." above
            let global_options = GlobalOptions {
                dry_run: ctx.dry_run,
                verbose: ctx.verbose,
            };
            let runner = StreamingRunner::new("mdbook build", &global_options);
            runner
                .run_streaming_tasks(vec![StreamingTask::new("mdbook build", mdbook_cmd)])
                .await
        }
        "serve" => {
            Box::pin(cmd_doc(ctx, "book")).await?;
            let mut cmd = TokioCommand::new("mdbook");
            cmd.arg("serve").arg("docs").arg("--open");
            ctx.run(cmd, "Serving documentation").await
        }
        "test" => {
            // Build book (sequential due to symlink requirement)
            Box::pin(cmd_doc(ctx, "book")).await?;

            // Clean up stale artifacts
            let _ = TokioCommand::new("sh")
                .arg("-c")
                .arg("rm -f target/debug/deps/libeidetica-*.rlib target/debug/deps/libeidetica-*.rmeta")
                .status()
                .await;

            // Book Test: lychee, build, mdbook test (sequential)
            let mut lychee = TokioCommand::new("lychee");
            lychee
                .arg("--offline")
                .arg("--exclude-path")
                .arg("rustdoc")
                .arg("docs/book");

            let mut build_cmd = TokioCommand::new("cargo");
            build_cmd.arg("build").arg("-p").arg("eidetica");

            let mut test_cmd = TokioCommand::new("mdbook");
            test_cmd
                .arg("test")
                .arg("docs")
                .arg("-L")
                .arg("target/debug/deps")
                .env("RUST_LOG", "error");

            ctx.run_seq(
                "Book Test",
                vec![
                    ("lychee".to_string(), lychee),
                    ("cargo build".to_string(), build_cmd),
                    ("mdbook test".to_string(), test_cmd),
                ],
            )
            .await
        }
        "clean" => {
            let mut cmd = TokioCommand::new("mdbook");
            cmd.arg("clean").arg("docs");
            ctx.run(cmd, "Cleaning mdbook").await?;

            let _ = std::fs::remove_file("docs/src/rustdoc");
            Ok(())
        }
        "links" => {
            Box::pin(cmd_doc(ctx, "book")).await?;
            let mut cmd = TokioCommand::new("lychee");
            cmd.arg("--offline")
                .arg("--exclude-path")
                .arg("rustdoc")
                .arg("docs/book");
            ctx.run(cmd, "Checking links (offline)").await
        }
        "links-online" => {
            Box::pin(cmd_doc(ctx, "book")).await?;
            let mut cmd = TokioCommand::new("lychee");
            cmd.arg("--exclude-path")
                .arg("rustdoc")
                .arg("--exclude-path")
                .arg("fonts")
                .arg("docs/book");
            ctx.run(cmd, "Checking links (online)").await
        }
        "stats" => {
            let tested = count_code_blocks("docs/src", true)?;
            let total = count_code_blocks("docs/src", false)?;
            println!("{}/{} Code Blocks tested", tested, total);
            Ok(())
        }
        _ => {
            eprintln!("Unknown action: {}", action);
            eprintln!(
                "Options: api, api-full, book, serve, test, clean, links, links-online, stats"
            );
            anyhow::bail!("Unknown action: {}", action);
        }
    }
}

fn count_code_blocks(dir: &str, only_testable: bool) -> Result<usize> {
    use std::process::Command;

    let pattern = if only_testable { "```rust$" } else { "```rust" };
    let output = Command::new("grep")
        .arg("-r")
        .arg(pattern)
        .arg(dir)
        .output()?;

    let count = String::from_utf8_lossy(&output.stdout).lines().count();
    Ok(count)
}

// =============================================================================
// Coverage
// =============================================================================

async fn cmd_coverage(ctx: &Context, backend: &str) -> Result<()> {
    match backend {
        "inmemory" => run_coverage(ctx, None).await,
        "sqlite" => run_coverage(ctx, Some("sqlite")).await,
        "postgres" => run_coverage_postgres(ctx).await,
        "all" => {
            run_coverage(ctx, None).await?;
            std::fs::rename("coverage/lcov.info", "coverage/lcov-inmemory.info")?;

            run_coverage(ctx, Some("sqlite")).await?;
            std::fs::rename("coverage/lcov.info", "coverage/lcov-sqlite.info")?;

            run_coverage_postgres(ctx).await?;
            std::fs::rename("coverage/lcov.info", "coverage/lcov-postgres.info")?;

            println!("Merging coverage reports...");
            let mut cmd = TokioCommand::new("lcov");
            cmd.arg("-a")
                .arg("coverage/lcov-inmemory.info")
                .arg("-a")
                .arg("coverage/lcov-sqlite.info")
                .arg("-a")
                .arg("coverage/lcov-postgres.info")
                .arg("-o")
                .arg("coverage/lcov.info");
            ctx.run(cmd, "Merging coverage reports").await?;
            println!("Merged coverage report: coverage/lcov.info");
            Ok(())
        }
        "ignored" => {
            let mut cmd = TokioCommand::new("cargo");
            cmd.arg("tarpaulin")
                .arg("--workspace")
                .arg("--skip-clean")
                .arg("--all-features")
                .arg("--output-dir")
                .arg("coverage")
                .arg("--out")
                .arg("lcov")
                .arg("--engine")
                .arg("llvm")
                .arg("--ignored")
                .arg("--no-fail-fast");
            ctx.run(cmd, "Running coverage on ignored tests").await
        }
        _ => {
            eprintln!("Unknown backend: {}", backend);
            eprintln!("Options: inmemory, sqlite, postgres, all, ignored");
            anyhow::bail!("Unknown backend: {}", backend);
        }
    }
}

async fn run_coverage(ctx: &Context, backend: Option<&str>) -> Result<()> {
    let mut cmd = TokioCommand::new("cargo");

    if let Some(b) = backend {
        cmd.env("TEST_BACKEND", b);
    }

    cmd.arg("tarpaulin")
        .arg("--workspace")
        .arg("--skip-clean")
        .arg("--all-features")
        .arg("--output-dir")
        .arg("coverage")
        .arg("--out")
        .arg("lcov")
        .arg("--engine")
        .arg("llvm");

    let desc = match backend {
        Some(b) => format!("Running coverage ({})", b),
        None => "Running coverage".to_string(),
    };
    ctx.run(cmd, &desc).await
}

async fn run_coverage_postgres(ctx: &Context) -> Result<()> {
    let container_name = "eidetica-coverage-postgres";
    let db_name = "eidetica_test";
    let db_port = "54322";

    start_postgres_container(ctx, container_name, db_name, db_port).await?;

    let result = async {
        let mut cmd = TokioCommand::new("cargo");
        cmd.env("TEST_BACKEND", "postgres")
            .env(
                "TEST_POSTGRES_URL",
                format!("postgres://postgres@localhost:{}/{}", db_port, db_name),
            )
            .arg("tarpaulin")
            .arg("--workspace")
            .arg("--skip-clean")
            .arg("--all-features")
            .arg("--output-dir")
            .arg("coverage")
            .arg("--out")
            .arg("lcov")
            .arg("--engine")
            .arg("llvm");
        ctx.run(cmd, "Running coverage against PostgreSQL").await
    }
    .await;

    cleanup_container(container_name).await;
    result
}

// =============================================================================
// CI
// =============================================================================

async fn cmd_ci(ctx: &Context, mode: &str) -> Result<()> {
    match mode {
        "local" => {
            // Phase 1: Fix code (must run first, modifies files)
            cmd_fix(ctx).await?;

            // Phase 2: Lint, doc, and build in parallel (single StreamingRunner)
            let mut clippy = TokioCommand::new("cargo");
            clippy
                .arg("clippy")
                .arg("--workspace")
                .arg("--all-targets")
                .arg("--all-features")
                .arg("--")
                .arg("-D")
                .arg("warnings");

            let mut audit = TokioCommand::new("cargo");
            audit.arg("deny").arg("check");

            let mut doc = TokioCommand::new("cargo");
            doc.arg("doc")
                .arg("--workspace")
                .arg("--all-features")
                .arg("--no-deps");

            let mut build = TokioCommand::new("cargo");
            build
                .arg("build")
                .arg("--workspace")
                .arg("--all-targets")
                .arg("--all-features");

            ctx.run_group(
                "Check",
                vec![
                    ("clippy".to_string(), clippy),
                    ("audit".to_string(), audit),
                    ("doc".to_string(), doc),
                    ("build".to_string(), build),
                ],
            )
            .await?;

            // Phase 3: Test (needs build to complete)
            cmd_test(ctx, vec!["full".to_string()]).await
        }
        "full" => {
            let mut cmd = TokioCommand::new("act");
            cmd.arg("--workflows").arg(".github/workflows/rust.yml");
            ctx.run(cmd, "Running CI with act").await
        }
        "nix" => {
            // Build and check in parallel
            let mut build = TokioCommand::new("nix");
            build.arg("build");

            let mut check = TokioCommand::new("nix-fast-build");
            check.arg("--no-link").arg("--skip-cached");

            ctx.run_group(
                "Nix",
                vec![("build".to_string(), build), ("check".to_string(), check)],
            )
            .await
        }
        _ => {
            eprintln!("Unknown mode: {}", mode);
            eprintln!("Options: local, full, nix");
            anyhow::bail!("Unknown mode: {}", mode);
        }
    }
}

// =============================================================================
// Nix
// =============================================================================

async fn cmd_nix(ctx: &Context, action: &str) -> Result<()> {
    match action {
        "build" => {
            let mut cmd = TokioCommand::new("nix");
            cmd.arg("build");
            ctx.run(cmd, "Building with Nix").await
        }
        "check" => {
            let mut cmd = TokioCommand::new("nix-fast-build");
            cmd.arg("--no-link").arg("--skip-cached");
            ctx.run(cmd, "Checking Nix flake").await
        }
        "integration" => {
            // Run both integration tests in parallel
            let mut nixos_cmd = TokioCommand::new("nix");
            nixos_cmd
                .arg("build")
                .arg(".#integration-nixos")
                .arg("--no-link");

            let mut container_cmd = TokioCommand::new("nix");
            container_cmd
                .arg("build")
                .arg(".#integration-container")
                .arg("--no-link");

            ctx.run_group(
                "Integration",
                vec![
                    ("nixos".to_string(), nixos_cmd),
                    ("container".to_string(), container_cmd),
                ],
            )
            .await
        }
        "full" => {
            // Run check and both integration tests in parallel
            let mut check_cmd = TokioCommand::new("nix-fast-build");
            check_cmd.arg("--no-link").arg("--skip-cached");

            let mut nixos_cmd = TokioCommand::new("nix");
            nixos_cmd
                .arg("build")
                .arg(".#integration-nixos")
                .arg("--no-link");

            let mut container_cmd = TokioCommand::new("nix");
            container_cmd
                .arg("build")
                .arg(".#integration-container")
                .arg("--no-link");

            ctx.run_group(
                "Nix Full",
                vec![
                    ("check".to_string(), check_cmd),
                    ("nixos".to_string(), nixos_cmd),
                    ("container".to_string(), container_cmd),
                ],
            )
            .await
        }
        _ => {
            eprintln!("Unknown action: {}", action);
            eprintln!("Options: build, check, integration, full");
            anyhow::bail!("Unknown action: {}", action);
        }
    }
}

// =============================================================================
// Container
// =============================================================================

async fn cmd_container(ctx: &Context, r#type: &str) -> Result<()> {
    match r#type {
        "docker" => {
            let mut cmd = TokioCommand::new("docker");
            cmd.arg("build").arg("-t").arg("eidetica:dev").arg(".");
            ctx.run(cmd, "Building Docker image").await
        }
        "nix" => {
            let mut build_cmd = TokioCommand::new("nix");
            build_cmd.arg("build").arg(".#eidetica-image");
            ctx.run(build_cmd, "Building Nix image").await?;

            let mut load_cmd = TokioCommand::new("docker");
            load_cmd.arg("load").arg("-i").arg("./result");
            ctx.run(load_cmd, "Loading image into Docker").await
        }
        _ => {
            eprintln!("Unknown type: {}", r#type);
            eprintln!("Options: docker, nix");
            anyhow::bail!("Unknown type: {}", r#type);
        }
    }
}

// =============================================================================
// Bench
// =============================================================================

async fn cmd_bench(ctx: &Context) -> Result<()> {
    let mut cmd = TokioCommand::new("cargo");
    cmd.arg("bench").arg("--workspace");
    ctx.run(cmd, "Running benchmarks").await?;

    // Try to open report
    let _ = TokioCommand::new("xdg-open")
        .arg("target/criterion/report/index.html")
        .spawn();

    Ok(())
}

// =============================================================================
// Dev & Fix (group shortcuts)
// =============================================================================

async fn cmd_dev(ctx: &Context) -> Result<()> {
    cmd_build(ctx, BuildMode::Debug).await?;
    cmd_test(ctx, vec![]).await?;
    cmd_lint(ctx, vec!["clippy".to_string()]).await
}

async fn cmd_fix(ctx: &Context) -> Result<()> {
    let mut clippy_cmd = TokioCommand::new("cargo");
    clippy_cmd
        .arg("clippy")
        .arg("--workspace")
        .arg("--fix")
        .arg("--allow-dirty")
        .arg("--all-targets")
        .arg("--all-features")
        .arg("--allow-no-vcs")
        .arg("--")
        .arg("-D")
        .arg("warnings");

    let mut cargo_fmt = TokioCommand::new("cargo");
    cargo_fmt.arg("fmt").arg("--all");

    let mut alejandra = TokioCommand::new("alejandra");
    alejandra.arg(".");

    let mut prettier = TokioCommand::new("prettier");
    prettier
        .arg("--write")
        .arg(".")
        .arg("--log-level")
        .arg("warn");

    let mut typos = TokioCommand::new("typos");
    typos
        .arg("--write-changes")
        .arg("--config")
        .arg(".config/typos.toml");

    ctx.run_group(
        "Fix",
        vec![
            ("clippy --fix".to_string(), clippy_cmd),
            ("cargo fmt".to_string(), cargo_fmt),
            ("alejandra".to_string(), alejandra),
            ("prettier".to_string(), prettier),
            ("typos".to_string(), typos),
        ],
    )
    .await
}

// =============================================================================
// List & Completions
// =============================================================================

fn cmd_list() -> Result<()> {
    println!("Available commands:");
    println!();
    println!("  build [mode]        Build the project (debug/release)");
    println!(
        "  test [variant]      Run tests (sqlite/postgres/all-backends/doc/full/ignored/minimal/todo/<filter>)"
    );
    println!("  lint [tools...]     Run linters (clippy/audit/typos/udeps/min-versions/all)");
    println!("  fmt                 Format code (cargo fmt + alejandra + prettier + typos)");
    println!("  sanitize [targets]  Run sanitizers (miri/careful/asan/tsan/lsan/all)");
    println!(
        "  doc [action]        Documentation (api/api-full/book/serve/test/clean/links/links-online/stats)"
    );
    println!("  coverage [backend]  Code coverage (inmemory/sqlite/postgres/all/ignored)");
    println!("  ci [mode]           CI pipeline (local/full/nix)");
    println!("  nix [action]        Nix commands (build/check/integration/full)");
    println!("  container [type]    Build container (docker/nix)");
    println!("  bench               Run benchmarks");
    println!();
    println!("  dev                 Quick feedback: build + test + clippy");
    println!("  fix                 Auto-fix: clippy --fix + format");
    println!();
    println!("  list                Show this help");
    println!("  completions <shell> Generate shell completions");
    Ok(())
}

fn generate_completions(shell: Shell) -> Result<()> {
    let mut cmd = Cli::command();
    let name = cmd.get_name().to_string();
    generate(shell, &mut cmd, name, &mut io::stdout());
    Ok(())
}

// =============================================================================
// Docker helpers
// =============================================================================

async fn start_postgres_container(
    ctx: &Context,
    name: &str,
    db_name: &str,
    port: &str,
) -> Result<()> {
    if ctx.dry_run {
        println!("[dry-run] Would start PostgreSQL container: {}", name);
        return Ok(());
    }

    // Remove existing container if any
    let _ = TokioCommand::new("docker")
        .arg("rm")
        .arg("-f")
        .arg(name)
        .output()
        .await;

    println!("Starting PostgreSQL container...");
    let mut start_cmd = TokioCommand::new("docker");
    start_cmd
        .arg("run")
        .arg("-d")
        .arg("--name")
        .arg(name)
        .arg("-e")
        .arg(format!("POSTGRES_DB={}", db_name))
        .arg("-e")
        .arg("POSTGRES_HOST_AUTH_METHOD=trust")
        .arg("-p")
        .arg(format!("{}:5432", port))
        .arg("postgres:16-alpine");

    let output = start_cmd.output().await?;
    if !output.status.success() {
        anyhow::bail!("Failed to start PostgreSQL container");
    }

    // Wait for PostgreSQL to be ready
    println!("Waiting for PostgreSQL to be ready...");
    for i in 1..=30 {
        let ready_check = TokioCommand::new("docker")
            .arg("exec")
            .arg(name)
            .arg("pg_isready")
            .arg("-U")
            .arg("postgres")
            .arg("-d")
            .arg(db_name)
            .output()
            .await;

        if let Ok(output) = ready_check
            && output.status.success()
        {
            println!("PostgreSQL is ready!");
            return Ok(());
        }

        if i == 30 {
            cleanup_container(name).await;
            anyhow::bail!("Timed out waiting for PostgreSQL");
        }

        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }

    Ok(())
}

async fn cleanup_container(name: &str) {
    let _ = TokioCommand::new("docker")
        .arg("rm")
        .arg("-f")
        .arg(name)
        .output()
        .await;
}
