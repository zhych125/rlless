//! File handling abstraction with memory mapping and compression support.
//!
//! This module provides the core file access functionality for rlless, including
//! memory-mapped file access for large files and transparent compression support.
//!
//! The module is organized into focused sub-modules:
//! - `accessor`: Core FileAccessor trait and access strategies
//! - `compression`: Compression format detection using magic numbers
//! - `in_memory`: In-memory file accessor with zero-copy line extraction
//! - `mmap`: Memory-mapped file accessor for large files with lazy indexing
//! - `validation`: File validation utilities
//! - `line_index`: SIMD-optimized line boundary detection

pub mod accessor;
pub mod compression;
pub mod in_memory;
pub mod line_index;
pub mod mmap;
pub mod validation;

// Re-export public API for convenient access
pub use accessor::{AccessStrategy, FileAccessor};
pub use compression::{detect_compression, CompressionType};
pub use in_memory::InMemoryFileAccessor;
pub use mmap::MmapFileAccessor;
pub use validation::validate_file_path;

// Re-export test utilities for integration tests
#[cfg(test)]
pub use accessor::tests::MockFileAccessor;
