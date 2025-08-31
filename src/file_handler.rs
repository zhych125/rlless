//! File handling abstraction with memory mapping and compression support.
//!
//! This module provides the core file access functionality for rlless, including
//! memory-mapped file access for large files and transparent compression support.

use crate::error::Result;
use async_trait::async_trait;
use std::path::Path;

/// Supported compression formats for transparent file access
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompressionType {
    /// No compression - plain text file
    None,
    /// Gzip compression (.gz files)
    Gzip,
    /// Bzip2 compression (.bz2 files)
    Bzip2,
    /// XZ compression (.xz files)
    Xz,
}

/// Core trait for file access operations.
///
/// This trait abstracts file access to enable different implementations
/// (memory-mapped, streaming, compressed) while providing a consistent interface.
#[async_trait]
pub trait FileAccessor: Send + Sync {
    /// Read a range of bytes from the file
    async fn read_range(&self, start: u64, length: usize) -> Result<Vec<u8>>;
    
    /// Read a specific line by line number (0-based)
    async fn read_line(&self, line_number: u64) -> Result<String>;
    
    /// Get the total file size in bytes
    fn file_size(&self) -> u64;
    
    /// Get the total number of lines in the file (if available)
    fn total_lines(&self) -> Option<u64>;
}

/// Access strategy for different file handling approaches
#[derive(Debug)]
pub enum AccessStrategy {
    /// Memory-mapped file access for random access patterns
    MemoryMapped,
    /// Streaming access for sequential patterns or very large files
    Streaming,
    /// Hybrid approach combining both strategies
    Hybrid,
}

/// Detect compression type from file path and magic numbers
pub fn detect_compression(_path: &Path) -> Result<CompressionType> {
    // TODO: Implement in Task 3.2
    Ok(CompressionType::None)
}

/// Validate that a file path is accessible and suitable for processing
pub fn validate_file_path(_path: &Path) -> Result<()> {
    // TODO: Implement in Task 3.4
    Ok(())
}