//! SIMD-optimized line boundary finder with intelligent caching
//!
//! This module provides the LineIndex structure that maintains an index of line boundaries
//! in a file, building the index lazily as lines are accessed. It uses memchr for
//! SIMD-optimized newline detection and includes an LRU cache for frequently accessed lines.

use memchr::memchr;
use std::collections::VecDeque;

/// Cached line information for LRU cache
#[derive(Debug, Clone)]
struct CachedLine {
    /// Line number (0-based) for cache lookup
    line_number: u64,

    /// Cached line content (UTF-8 converted)
    /// Stored to avoid repeated UTF-8 conversion
    content: String,

    /// Byte position where this line starts in file
    /// Used for chunk operations
    byte_start: u64,

    /// Byte position where this line ends (before newline)
    /// Used for chunk operations
    byte_end: u64,
}

/// SIMD-optimized line boundary finder with intelligent caching
///
/// This structure maintains an index of line boundaries in a file,
/// building the index lazily as lines are accessed. It uses memchr
/// for SIMD-optimized newline detection.
#[derive(Debug)]
pub struct LineIndex {
    /// Byte offsets where each line starts
    ///
    /// - line_offsets[0] = 0 (first line always starts at byte 0)
    /// - line_offsets[n] = byte position after nth newline
    /// - Length of this vector - 1 = number of indexed lines
    ///
    /// Grows monotonically as more of the file is indexed
    line_offsets: Vec<u64>,

    /// How far into the file we've indexed (in bytes)
    ///
    /// Everything before this position has been scanned for newlines.
    /// Used to resume indexing from where we left off.
    indexed_to_byte: u64,

    /// LRU cache of recently accessed lines
    ///
    /// - Front = most recently used
    /// - Back = least recently used
    /// - Eviction happens from back when cache is full
    ///
    /// Speeds up repeated access to same lines (common in scrolling)
    line_cache: VecDeque<CachedLine>,

    /// Maximum number of lines to keep in cache
    ///
    /// Typically 100-1000 lines depending on available memory.
    /// Prevents unbounded memory growth.
    max_cache_size: usize,
}

impl LineIndex {
    /// Create a new empty line index
    pub fn new() -> Self {
        Self {
            line_offsets: vec![0], // First line starts at position 0
            indexed_to_byte: 0,
            line_cache: VecDeque::with_capacity(100),
            max_cache_size: 100,
        }
    }

    /// Create with custom cache size
    pub fn with_cache_size(max_cache_size: usize) -> Self {
        Self {
            line_offsets: vec![0],
            indexed_to_byte: 0,
            line_cache: VecDeque::with_capacity(max_cache_size),
            max_cache_size,
        }
    }

    /// Ensure we have indexed at least up to the target line
    ///
    /// # Arguments
    /// * `data` - The file data to scan (either mmap or in-memory buffer)
    /// * `target_line` - Line number we need to have indexed
    ///
    /// # Performance
    /// * Uses SIMD-optimized memchr for finding newlines
    /// * Only scans the portion of file not yet indexed
    /// * O(n) where n is bytes to scan, but only runs once per file region
    pub fn ensure_indexed_to(&mut self, data: &[u8], target_line: u64) {
        let current_lines = (self.line_offsets.len() - 1) as u64;

        if target_line <= current_lines {
            return; // Already indexed enough lines
        }

        let mut pos = self.indexed_to_byte as usize;

        // Use memchr for SIMD-optimized newline search
        while pos < data.len() {
            if let Some(newline_offset) = memchr(b'\n', &data[pos..]) {
                pos += newline_offset + 1; // Move past the newline
                self.line_offsets.push(pos as u64);

                // Check if we've indexed enough
                if self.line_offsets.len() > target_line as usize + 1 {
                    break;
                }
            } else {
                // No more newlines found, we've reached end of file
                pos = data.len(); // Mark that we've processed the entire file
                break;
            }
        }

        self.indexed_to_byte = pos as u64;
    }

    /// Get a line from cache if available
    ///
    /// # Returns
    /// * Some(content) if line is cached
    /// * None if line is not in cache
    ///
    /// # Side Effects
    /// * Moves accessed line to front of cache (LRU update)
    pub fn get_cached_line(&mut self, line_number: u64) -> Option<String> {
        if let Some(pos) = self
            .line_cache
            .iter()
            .position(|c| c.line_number == line_number)
        {
            // Move to front (mark as most recently used)
            let cached = self.line_cache.remove(pos).unwrap();
            let content = cached.content.clone();
            self.line_cache.push_front(cached);
            return Some(content);
        }
        None
    }

    /// Add a line to the cache
    ///
    /// # Arguments
    /// * `line_number` - Line number to cache
    /// * `content` - Line content (already UTF-8 converted)
    /// * `byte_start` - Where this line starts in the file
    /// * `byte_end` - Where this line ends in the file
    ///
    /// # Side Effects
    /// * May evict least recently used line if cache is full
    pub fn cache_line(
        &mut self,
        line_number: u64,
        content: String,
        byte_start: u64,
        byte_end: u64,
    ) {
        // Remove if already cached (to avoid duplicates)
        if let Some(pos) = self
            .line_cache
            .iter()
            .position(|c| c.line_number == line_number)
        {
            self.line_cache.remove(pos);
        }

        // Add to front (most recently used)
        self.line_cache.push_front(CachedLine {
            line_number,
            content,
            byte_start,
            byte_end,
        });

        // Evict LRU if cache is too large
        while self.line_cache.len() > self.max_cache_size {
            self.line_cache.pop_back();
        }
    }

    /// Get line start positions for a range
    ///
    /// # Returns
    /// * Slice of line offsets that have been indexed
    ///
    /// # Usage
    /// Used for calculating line positions in chunk operations
    pub fn get_line_offsets(&self) -> &[u64] {
        &self.line_offsets
    }

    /// Check how many lines have been indexed
    ///
    /// # Returns
    /// * Number of lines that have known positions
    pub fn indexed_line_count(&self) -> u64 {
        (self.line_offsets.len() - 1) as u64
    }

    /// Check how many bytes have been indexed
    ///
    /// # Returns
    /// * Byte position up to which we've scanned for newlines
    pub fn indexed_byte_count(&self) -> u64 {
        self.indexed_to_byte
    }
}

impl Default for LineIndex {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Create test data with known line structure
    fn create_test_data() -> Vec<u8> {
        b"line1\nline2\nline3\nline4\n".to_vec()
    }

    /// Create test data with various line endings and content
    fn create_complex_test_data() -> Vec<u8> {
        b"short\na longer line with content\n\nempty line above\nfinal".to_vec()
    }

    #[test]
    fn test_new_line_index() {
        let index = LineIndex::new();
        assert_eq!(index.line_offsets, vec![0]);
        assert_eq!(index.indexed_to_byte, 0);
        assert_eq!(index.indexed_line_count(), 0);
        assert_eq!(index.max_cache_size, 100);
    }

    #[test]
    fn test_custom_cache_size() {
        let index = LineIndex::with_cache_size(50);
        assert_eq!(index.max_cache_size, 50);
        assert_eq!(index.line_cache.capacity(), 50);
    }

    #[test]
    fn test_ensure_indexed_to_basic() {
        let mut index = LineIndex::new();
        let data = create_test_data(); // "line1\nline2\nline3\nline4\n"

        // Index to cover line 1 (second line)
        index.ensure_indexed_to(&data, 1);

        // Should have indexed at least 2 lines (0 and 1)
        assert!(index.indexed_line_count() >= 2);

        // Check that we have the expected line starts
        let offsets = index.get_line_offsets();
        assert_eq!(offsets[0], 0); // Line 0 starts at beginning
        assert_eq!(offsets[1], 6); // Line 1 starts after "line1\n"
    }

    #[test]
    fn test_ensure_indexed_to_incremental() {
        let mut index = LineIndex::new();
        let data = create_test_data();

        // First call - index some lines
        index.ensure_indexed_to(&data, 0);
        let first_count = index.indexed_line_count();
        let first_byte_count = index.indexed_byte_count();

        // Second call - should not re-scan already indexed content
        index.ensure_indexed_to(&data, 0);
        assert_eq!(index.indexed_line_count(), first_count);
        assert_eq!(index.indexed_byte_count(), first_byte_count);

        // Request more lines
        index.ensure_indexed_to(&data, 3);
        assert!(index.indexed_line_count() >= 3);
    }

    #[test]
    fn test_cache_operations() {
        let mut index = LineIndex::with_cache_size(3);

        // Add lines to cache
        index.cache_line(0, "line1".to_string(), 0, 5);
        index.cache_line(1, "line2".to_string(), 6, 11);
        index.cache_line(2, "line3".to_string(), 12, 17);

        assert_eq!(index.line_cache.len(), 3);

        // Test cache hit
        let result = index.get_cached_line(1);
        assert_eq!(result, Some("line2".to_string()));

        // Line 1 should now be at front (most recently used)
        assert_eq!(index.line_cache.front().unwrap().line_number, 1);

        // Test cache miss
        let result = index.get_cached_line(99);
        assert_eq!(result, None);
    }

    #[test]
    fn test_cache_eviction() {
        let mut index = LineIndex::with_cache_size(2);

        // Fill cache to capacity
        index.cache_line(0, "line0".to_string(), 0, 5);
        index.cache_line(1, "line1".to_string(), 6, 11);
        assert_eq!(index.line_cache.len(), 2);

        // Add one more - should evict least recently used
        index.cache_line(2, "line2".to_string(), 12, 17);
        assert_eq!(index.line_cache.len(), 2);

        // line0 should be evicted, line1 and line2 should remain
        assert!(index.get_cached_line(0).is_none());
        assert!(index.get_cached_line(1).is_some());
        assert!(index.get_cached_line(2).is_some());
    }

    #[test]
    fn test_cache_duplicate_handling() {
        let mut index = LineIndex::with_cache_size(3);

        // Add a line
        index.cache_line(0, "line0".to_string(), 0, 5);
        assert_eq!(index.line_cache.len(), 1);

        // Add the same line again with different content
        index.cache_line(0, "updated line0".to_string(), 0, 5);
        assert_eq!(index.line_cache.len(), 1);

        // Should have the updated content
        let result = index.get_cached_line(0);
        assert_eq!(result, Some("updated line0".to_string()));
    }

    #[test]
    fn test_complex_line_structure() {
        let mut index = LineIndex::new();
        let data = create_complex_test_data();

        index.ensure_indexed_to(&data, 10); // Index many lines (more than exist)

        // Verify line positions
        let offsets = index.get_line_offsets();
        assert!(offsets.len() >= 2); // Should have found some lines
        assert_eq!(offsets[0], 0); // First line always starts at 0

        // Verify we can access beyond the actual line count without panic
        index.ensure_indexed_to(&data, 100);
        // Should have processed the entire file
        assert_eq!(index.indexed_byte_count(), data.len() as u64);
    }

    #[test]
    fn test_empty_file() {
        let mut index = LineIndex::new();
        let data = Vec::new();

        index.ensure_indexed_to(&data, 0);
        assert_eq!(index.indexed_line_count(), 0);
        assert_eq!(index.indexed_byte_count(), 0);
        assert_eq!(index.get_line_offsets(), &[0]);
    }

    #[test]
    fn test_single_line_no_newline() {
        let mut index = LineIndex::new();
        let data = b"single line without newline".to_vec();

        // Requesting line 0 doesn't require processing since line 0 always exists
        index.ensure_indexed_to(&data, 0);
        assert_eq!(index.indexed_line_count(), 0); // Still no indexed lines beyond the initial state

        // Now request a line beyond what exists - this will cause processing
        index.ensure_indexed_to(&data, 1);
        assert_eq!(index.indexed_byte_count(), data.len() as u64); // Now it processes the entire file
        assert_eq!(index.indexed_line_count(), 0); // Still no newlines found
    }

    #[test]
    fn test_lines_ending_with_newline() {
        let mut index = LineIndex::new();
        let data = b"line1\nline2\n".to_vec();

        index.ensure_indexed_to(&data, 5);
        let offsets = index.get_line_offsets();

        // Should have: [0, 6, 12]
        // - Line 0 starts at 0
        // - Line 1 starts at 6 (after "line1\n")
        // - Position 12 is after "line2\n" (end of file)
        assert_eq!(offsets.len(), 3);
        assert_eq!(offsets[0], 0);
        assert_eq!(offsets[1], 6);
        assert_eq!(offsets[2], 12);
        assert_eq!(index.indexed_line_count(), 2);
    }

    #[test]
    fn test_get_line_offsets_immutable() {
        let index = LineIndex::new();
        let offsets = index.get_line_offsets();
        assert_eq!(offsets, &[0]);
    }

    #[test]
    fn test_debug_implementation() {
        let index = LineIndex::new();
        let debug_str = format!("{:?}", index);
        assert!(debug_str.contains("LineIndex"));
        assert!(debug_str.contains("line_offsets"));
    }

    #[test]
    fn test_default_implementation() {
        let index = LineIndex::default();
        assert_eq!(index.line_offsets, vec![0]);
        assert_eq!(index.max_cache_size, 100);
    }
}
