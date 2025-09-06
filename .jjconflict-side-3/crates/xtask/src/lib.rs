//! Xtask library - shared utilities for development automation
//!
//! This library provides shared utilities used by the xtask binary.
//! The main CLI is self-contained in main.rs.

// Re-export indicatif for external use
pub use indicatif;

// Streaming utilities for progress display
pub mod streaming;

/// Output symbols for consistent terminal display
pub mod output {
    pub const DRY_RUN: &str = "[dry-run]";
    pub const SUCCESS: &str = "✓";
    pub const FAILURE: &str = "✗";
}

/// Global options that apply to all task execution
#[derive(Debug, Clone)]
pub struct GlobalOptions {
    pub dry_run: bool,
    pub verbose: bool,
}
