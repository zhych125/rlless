//! Core file access abstraction.
//!
//! This module defines the fundamental FileAccessor trait that provides a consistent
//! interface for different file access implementations. The trait uses byte-based
//! navigation for optimal performance with large files.

use crate::error::Result;
use async_trait::async_trait;
use std::path::Path;
use std::sync::atomic::AtomicBool;

/// Core trait for file access operations using byte-based navigation
///
/// This trait provides a unified interface for both small files (loaded into memory)
/// and large files (memory-mapped) using byte positions instead of line numbers.
/// All implementations must be thread-safe.
#[async_trait]
pub trait FileAccessor: Send + Sync {
    /// Read lines starting from a specific byte position
    ///
    /// # Arguments
    /// * `start_byte` - Byte position to start reading from (0-based)
    /// * `max_lines` - Maximum number of lines to read
    ///
    /// # Returns
    /// * Vector of lines starting from the byte position
    /// * May be shorter than `max_lines` if EOF reached
    /// * Empty vector if `start_byte` is beyond EOF
    ///
    /// # Performance
    /// * InMemory: O(1) per line access
    /// * Mmap: Efficient seeking to byte position, then sequential read
    ///
    /// # Usage
    /// Used for viewport rendering, navigation (PageUp/Down, Go to End)
    async fn read_from_byte(&self, start_byte: u64, max_lines: usize) -> Result<Vec<String>>;

    /// Find next occurrence using a search function from byte position
    ///
    /// # Arguments
    /// * `start_byte` - Byte position to start searching from (inclusive)
    /// * `search_fn` - Function that returns match ranges for a given line
    ///
    /// # Returns
    /// * Some(byte_position) if matches found - byte position of line containing match
    /// * None if no matches found before EOF
    ///
    /// # Performance
    /// * Searches incrementally, returns as soon as match found
    ///
    /// # Usage
    /// Used for forward search (/, n command in less)
    async fn find_next_match(
        &self,
        start_byte: u64,
        search_fn: &(dyn for<'a> Fn(&'a str) -> Vec<(usize, usize)> + Send + Sync),
        cancel_flag: Option<&AtomicBool>,
    ) -> Result<Option<u64>>;

    /// Find previous occurrence using a search function searching backward from byte position
    ///
    /// # Arguments
    /// * `start_byte` - Byte position to start searching from (exclusive, searches backward from here)
    /// * `search_fn` - Function that returns match ranges for a given line
    ///
    /// # Returns
    /// * Some(byte_position) if matches found - byte position of line containing match
    /// * None if no matches found before beginning of file
    ///
    /// # Performance
    /// * Searches incrementally backward from start_byte
    ///
    /// # Usage
    /// Used for backward search (?, N command in less)
    async fn find_prev_match(
        &self,
        start_byte: u64,
        search_fn: &(dyn for<'a> Fn(&'a str) -> Vec<(usize, usize)> + Send + Sync),
        cancel_flag: Option<&AtomicBool>,
    ) -> Result<Option<u64>>;

    /// Get the total file size in bytes
    ///
    /// # Returns
    /// * File size in bytes (always known, O(1))
    ///
    /// # Performance
    /// * O(1) - cached from file metadata
    ///
    /// # Usage
    /// Used for progress indicators (percentage = current_byte / file_size)
    fn file_size(&self) -> u64;

    /// Get the file path for this accessor
    ///
    /// # Returns
    /// * Path to the file being accessed
    ///
    /// # Usage
    /// Used for display purposes, error messages, file operations
    fn file_path(&self) -> &Path;

    /// Calculate the last page byte position for "Go to End" functionality
    ///
    /// # Arguments
    /// * `max_lines` - Maximum number of lines to show on last page
    ///
    /// # Returns
    /// * Byte position where the last page should start
    /// * Returns 0 if file is smaller than one page
    ///
    /// # Usage
    /// Used for "Go to End" (G command in less) - ALWAYS works, even for 40GB files
    async fn last_page_start(&self, max_lines: usize) -> Result<u64>;

    /// Find the byte position for the next page
    ///
    /// # Arguments
    /// * `current_byte` - Current byte position
    /// * `lines_to_skip` - Number of lines to advance
    ///
    /// # Returns
    /// * Byte position where next page should start
    /// * Returns file_size if at EOF (couldn't complete full navigation)
    ///
    /// # Usage
    /// Used for PageDown navigation
    async fn next_page_start(&self, current_byte: u64, lines_to_skip: usize) -> Result<u64>;

    /// Find the byte position for the previous page
    ///
    /// # Arguments
    /// * `current_byte` - Current byte position
    /// * `lines_to_skip` - Number of lines to go back
    ///
    /// # Returns
    /// * Byte position where previous page should start
    /// * Returns 0 if already at beginning
    ///
    /// # Usage
    /// Used for PageUp navigation
    async fn prev_page_start(&self, current_byte: u64, lines_to_skip: usize) -> Result<u64>;
}
