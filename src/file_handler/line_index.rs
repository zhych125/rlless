//! SIMD-optimized line boundary detection
//!
//! This module provides the LineIndex structure that maintains an index of line boundaries
//! in a file, building the index lazily as lines are accessed. It uses memchr for
//! SIMD-optimized newline detection without any string caching overhead.

use memchr::memchr;

/// SIMD-optimized line boundary finder
///
/// This structure maintains an index of line boundaries in a file,
/// building the index lazily as lines are accessed. It uses memchr
/// for SIMD-optimized newline detection and provides zero-copy access
/// to line data via byte offsets.
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
}

impl LineIndex {
    /// Create a new empty line index
    pub fn new() -> Self {
        Self {
            line_offsets: vec![0], // First line starts at position 0
            indexed_to_byte: 0,
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
        // Special case: empty data means no lines exist
        if data.is_empty() {
            return;
        }

        // If we have already indexed enough lines, return early
        if target_line < self.indexed_line_count() {
            return;
        }

        let mut pos = self.indexed_to_byte as usize;

        // Use memchr for SIMD-optimized newline search
        while pos < data.len() {
            if let Some(newline_offset) = memchr(b'\n', &data[pos..]) {
                pos += newline_offset + 1; // Move past the newline
                self.line_offsets.push(pos as u64);

                // Check if we've indexed enough
                if target_line != u64::MAX && (self.line_offsets.len() - 1) as u64 > target_line {
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

    /// Get line byte range (start, end) for a specific line number
    ///
    /// # Arguments
    /// * `line_number` - The line number to get bounds for (0-based)
    ///
    /// # Returns
    /// * Some((start_byte, end_byte)) if line exists
    /// * None if line_number is beyond indexed content
    ///
    /// # Usage
    /// Used for zero-copy line extraction from file data
    pub fn get_line_range(&self, line_number: u64) -> Option<(u64, u64)> {
        let line_idx = line_number as usize;

        // Check if this line number exists
        if line_number >= self.indexed_line_count() {
            return None; // Line not indexed yet
        }

        let start = self.line_offsets[line_idx];
        let end = if line_idx + 1 < self.line_offsets.len() {
            // Not the last line - end is just before next line's newline
            self.line_offsets[line_idx + 1].saturating_sub(1)
        } else {
            // Last indexed line - end is current indexed position
            self.indexed_to_byte
        };

        Some((start, end))
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
        // If we haven't processed any content yet, there are no lines
        if self.indexed_to_byte == 0 {
            return 0;
        }

        // line_offsets.len() is always >= 1 (initialized with [0])
        // but let's be defensive and handle the empty case
        if self.line_offsets.is_empty() {
            return 0;
        }

        let newline_count = (self.line_offsets.len() - 1) as u64;

        // If we have indexed some content beyond the last newline, that's another line
        if self.line_offsets.len() <= 1 {
            // No newlines found, but we have content, so that's 1 line
            1
        } else {
            // Check if there's content after the last newline
            let last_newline_pos = *self.line_offsets.last().unwrap();
            if self.indexed_to_byte > last_newline_pos {
                newline_count + 1
            } else {
                newline_count
            }
        }
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
    fn test_get_line_range() {
        let mut index = LineIndex::new();
        let data = create_test_data(); // "line1\nline2\nline3\nline4\n"

        // Index some lines
        index.ensure_indexed_to(&data, 2);

        // Test first line
        let range = index.get_line_range(0);
        assert_eq!(range, Some((0, 5))); // "line1" without newline

        // Test second line
        let range = index.get_line_range(1);
        assert_eq!(range, Some((6, 11))); // "line2" without newline

        // Test non-existent line
        let range = index.get_line_range(99);
        assert_eq!(range, None);
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

        // Requesting line 0 with non-empty data should process content to establish line 0 exists
        index.ensure_indexed_to(&data, 0);
        assert_eq!(index.indexed_line_count(), 1); // One line found (content exists, no newlines)
        assert_eq!(index.indexed_byte_count(), data.len() as u64); // Entire file processed

        // Requesting line 1 should not find more lines
        index.ensure_indexed_to(&data, 1);
        assert_eq!(index.indexed_line_count(), 1); // Still only one line
        assert_eq!(index.indexed_byte_count(), data.len() as u64); // Still entire file processed
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
        assert_eq!(index.indexed_to_byte, 0);
    }
}
