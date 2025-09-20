//! # rlless - High-Performance Terminal Log Viewer
//!
//! A fast, memory-efficient terminal log viewer designed to handle extremely large files
//! (40GB+) with SIMD-optimized search capabilities powered by ripgrep.
//!
//! ## Features
//!
//! - **Large File Support**: Memory-mapped file access for files up to 40GB+
//! - **SIMD-Optimized Search**: Fast pattern matching using ripgrep core libraries
//! - **Memory Efficient**: Maintains <100MB memory usage regardless of file size
//! - **Compression Support**: Transparent handling of gzip, bzip2, and xz formats
//! - **Terminal UI**: Familiar less-like navigation with responsive interface
//!
//! ## Architecture
//!
//! The library is organized into focused modules following modern Rust patterns:
//!
//! - [`error`] - Centralized error types and handling
//! - [`file_handler`] - File access abstraction with memory mapping
//! - [`search`] - Search engine integration with ripgrep
//! - [`render::ui`](crate::render::ui) - Terminal user interface components
//! - [`app`] - Application core and component coordination

// Core modules
pub mod error;
pub mod file_handler;

// Subsystems introduced by the refactor roadmap
pub mod input;
pub mod render;

// Core components
pub mod app;
pub mod search;

// Re-export commonly used types for convenience
pub use error::{Result, RllessError};

// Public API surface for external usage
pub use app::Application;
pub use file_handler::FileAccessor;
pub use search::{RipgrepEngine, SearchEngine, SearchOptions};

// Version information
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
