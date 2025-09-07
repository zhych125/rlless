//! File handling abstraction with memory mapping and compression support.
//!
//! This module provides the core file access functionality for rlless, including
//! memory-mapped file access for large files and transparent compression support.
//!
//! The module is organized into focused sub-modules:
//! - `accessor`: Core FileAccessor trait and access strategies
//! - `adaptive`: Adaptive file accessor supporting in-memory, mmap, and compressed files
//! - `compression`: Compression format detection and decompression utilities
//! - `validation`: File validation utilities

pub mod accessor;
pub mod adaptive;
pub mod compression;
pub mod factory;
pub mod validation;

// Re-export public API for convenient access
pub use accessor::FileAccessor;
pub use adaptive::AdaptiveFileAccessor;
pub use compression::{decompress_file, detect_compression, DecompressionResult};
pub use factory::FileAccessorFactory;
pub use validation::validate_file_path;
