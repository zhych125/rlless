//! In-memory file accessor for small to medium files
//!
//! This module provides the InMemoryFileAccessor implementation that loads
//! entire file content into memory and provides zero-copy line access.

use crate::error::{Result, RllessError};
use crate::file_handler::accessor::{FileAccessor, MatchInfo};
use crate::file_handler::line_index::LineIndex;
use async_trait::async_trait;
use std::borrow::Cow;

/// Production in-memory file accessor for small to medium files
///
/// This implementation loads the entire file content into memory and provides
/// zero-copy line access by extracting lines directly from the content buffer.
#[derive(Debug)]
pub struct InMemoryFileAccessor {
    /// File content loaded into memory (source of truth)
    content: Vec<u8>,

    /// File size in bytes (cached for performance)
    file_size: u64,

    /// LineIndex for line boundary detection (no string caching)
    line_index: LineIndex,

    /// Total number of lines (computed during construction)
    total_lines: u64,
}

impl InMemoryFileAccessor {
    /// Create a new in-memory file accessor with zero-copy line extraction
    ///
    /// # Arguments
    /// * `content` - File content as bytes
    ///
    /// # Returns
    /// * New InMemoryFileAccessor instance ready for zero-copy access
    ///
    /// # Performance
    /// * O(n) - Scans entire content once to build line boundary index
    /// * All subsequent line operations are O(1) zero-copy extractions
    pub fn new(content: Vec<u8>) -> Self {
        let file_size = content.len() as u64;
        let mut line_index = LineIndex::new();

        // Eagerly index the entire content for line boundaries
        line_index.ensure_indexed_to(&content, u64::MAX);

        let total_lines = line_index.indexed_line_count();

        Self {
            content,
            file_size,
            line_index,
            total_lines,
        }
    }

    /// Extract line content directly from content buffer (zero-copy)
    ///
    /// # Arguments
    /// * `line_number` - 0-based line number
    ///
    /// # Returns
    /// * Line content as Cow<str> - zero-copy Cow::Borrowed if valid UTF-8
    /// * Error if line number is out of bounds
    ///
    /// # Performance
    /// * O(1) - Direct extraction using pre-computed line boundaries
    fn extract_line(&self, line_number: u64) -> Result<Cow<'_, str>> {
        // Bounds check first
        if line_number >= self.total_lines {
            return Err(RllessError::file_error(
                format!(
                    "Line number {} out of bounds (total lines: {})",
                    line_number, self.total_lines
                ),
                std::io::Error::new(std::io::ErrorKind::InvalidInput, "Line out of bounds"),
            ));
        }

        // Get line byte range from line index
        let (start, end) = self
            .line_index
            .get_line_range(line_number)
            .expect("Line range should exist for valid line number");

        // Extract bytes for this line
        let line_bytes = &self.content[start as usize..end as usize];

        // Zero-copy conversion from [u8] to &str if valid UTF-8
        // Uses Cow::Borrowed for valid UTF-8, Cow::Owned only if replacement needed
        Ok(String::from_utf8_lossy(line_bytes))
    }
}

#[async_trait]
impl FileAccessor for InMemoryFileAccessor {
    async fn read_line(&self, line_number: u64) -> Result<Cow<'_, str>> {
        self.extract_line(line_number)
    }

    async fn read_lines_range(&self, start: u64, count: u64) -> Result<Vec<Cow<'_, str>>> {
        let mut lines =
            Vec::with_capacity(count.min(self.total_lines.saturating_sub(start)) as usize);

        let end_line = (start + count).min(self.total_lines);

        for line_num in start..end_line {
            lines.push(self.extract_line(line_num)?);
        }

        Ok(lines)
    }

    async fn find_next_match(&self, start_line: u64, pattern: &str) -> Result<Option<MatchInfo>> {
        for current_line in start_line..self.total_lines {
            let line_content = self.extract_line(current_line)?;

            if let Some(match_start) = line_content.find(pattern) {
                return Ok(Some(MatchInfo {
                    line_number: current_line,
                    byte_offset: self.line_index.get_line_offsets()[current_line as usize],
                    line_content: line_content.into_owned(), // Convert Cow to String for MatchInfo
                    match_start,
                    match_end: match_start + pattern.len(),
                }));
            }
        }

        Ok(None)
    }

    async fn find_prev_match(&self, start_line: u64, pattern: &str) -> Result<Option<MatchInfo>> {
        if start_line == 0 {
            return Ok(None);
        }

        for current_line in (0..start_line).rev() {
            let line_content = self.extract_line(current_line)?;

            if let Some(match_start) = line_content.find(pattern) {
                return Ok(Some(MatchInfo {
                    line_number: current_line,
                    byte_offset: self.line_index.get_line_offsets()[current_line as usize],
                    line_content: line_content.into_owned(), // Convert Cow to String for MatchInfo
                    match_start,
                    match_end: match_start + pattern.len(),
                }));
            }
        }

        Ok(None)
    }

    fn file_size(&self) -> u64 {
        self.file_size
    }

    fn total_lines(&self) -> Option<u64> {
        Some(self.total_lines)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_content() -> Vec<u8> {
        b"line1\nline2\nline3\nline4\n".to_vec()
    }

    fn create_complex_content() -> Vec<u8> {
        b"short\na longer line with content\n\nempty line above\nfinal line without newline"
            .to_vec()
    }

    #[test]
    fn test_new_in_memory_accessor() {
        let content = create_test_content();
        let file_size = content.len();
        let accessor = InMemoryFileAccessor::new(content);

        assert_eq!(accessor.file_size(), file_size as u64);
        assert_eq!(accessor.total_lines(), Some(4));
        assert_eq!(accessor.line_index.indexed_line_count(), 4);
    }

    #[tokio::test]
    async fn test_read_line() {
        let content = create_test_content();
        let accessor = InMemoryFileAccessor::new(content);

        let line0 = accessor.read_line(0).await.unwrap();
        assert_eq!(line0, "line1");

        let line2 = accessor.read_line(2).await.unwrap();
        assert_eq!(line2, "line3");

        let line3 = accessor.read_line(3).await.unwrap();
        assert_eq!(line3, "line4");
    }

    #[tokio::test]
    async fn test_read_line_out_of_bounds() {
        let content = create_test_content();
        let accessor = InMemoryFileAccessor::new(content);

        let result = accessor.read_line(999).await;
        assert!(result.is_err());

        let result = accessor.read_line(4).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_read_lines_range() {
        let content = create_test_content();
        let accessor = InMemoryFileAccessor::new(content);

        let lines = accessor.read_lines_range(1, 2).await.unwrap();
        assert_eq!(lines, vec!["line2", "line3"]);

        let lines = accessor.read_lines_range(3, 5).await.unwrap();
        assert_eq!(lines, vec!["line4"]);

        let lines = accessor.read_lines_range(10, 5).await.unwrap();
        assert!(lines.is_empty());
    }

    #[tokio::test]
    async fn test_find_next_match() {
        let content = b"error on line1\nno match here\nerror on line3\nfinal line\n".to_vec();
        let accessor = InMemoryFileAccessor::new(content);

        let match_info = accessor.find_next_match(0, "error").await.unwrap();
        assert!(match_info.is_some());

        let info = match_info.unwrap();
        assert_eq!(info.line_number, 0);
        assert_eq!(info.line_content, "error on line1");
        assert_eq!(info.match_start, 0);
        assert_eq!(info.match_end, 5);

        let match_info = accessor.find_next_match(1, "error").await.unwrap();
        assert!(match_info.is_some());
        assert_eq!(match_info.unwrap().line_number, 2);

        let match_info = accessor.find_next_match(0, "nonexistent").await.unwrap();
        assert!(match_info.is_none());
    }

    #[tokio::test]
    async fn test_find_prev_match() {
        let content = b"error on line1\nno match here\nerror on line3\nfinal line\n".to_vec();
        let accessor = InMemoryFileAccessor::new(content);

        let match_info = accessor.find_prev_match(4, "error").await.unwrap();
        assert!(match_info.is_some());
        assert_eq!(match_info.unwrap().line_number, 2);

        let match_info = accessor.find_prev_match(2, "error").await.unwrap();
        assert!(match_info.is_some());
        assert_eq!(match_info.unwrap().line_number, 0);

        let match_info = accessor.find_prev_match(0, "error").await.unwrap();
        assert!(match_info.is_none());
    }

    #[tokio::test]
    async fn test_total_lines_always_available() {
        let content = create_test_content();
        let accessor = InMemoryFileAccessor::new(content);

        assert_eq!(accessor.total_lines(), Some(4));

        let empty_accessor = InMemoryFileAccessor::new(Vec::new());
        assert_eq!(empty_accessor.total_lines(), Some(0));
    }

    #[tokio::test]
    async fn test_empty_content() {
        let accessor = InMemoryFileAccessor::new(Vec::new());

        assert_eq!(accessor.file_size(), 0);
        assert_eq!(accessor.total_lines(), Some(0));

        let result = accessor.read_line(0).await;
        assert!(result.is_err());

        let lines = accessor.read_lines_range(0, 1).await.unwrap();
        assert!(lines.is_empty());
    }

    #[tokio::test]
    async fn test_single_line_no_newline() {
        let content = b"single line without newline".to_vec();
        let accessor = InMemoryFileAccessor::new(content);

        assert_eq!(accessor.total_lines(), Some(1));

        let line = accessor.read_line(0).await.unwrap();
        assert_eq!(line, "single line without newline");

        let result = accessor.read_line(1).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_complex_content() {
        let content = create_complex_content();
        let accessor = InMemoryFileAccessor::new(content);

        assert_eq!(accessor.total_lines(), Some(5));

        let line0 = accessor.read_line(0).await.unwrap();
        assert_eq!(line0, "short");

        let line1 = accessor.read_line(1).await.unwrap();
        assert_eq!(line1, "a longer line with content");

        let line2 = accessor.read_line(2).await.unwrap();
        assert_eq!(line2, "");

        let line3 = accessor.read_line(3).await.unwrap();
        assert_eq!(line3, "empty line above");

        let line4 = accessor.read_line(4).await.unwrap();
        assert_eq!(line4, "final line without newline");
    }

    #[tokio::test]
    async fn test_all_lines_accessible() {
        let content = create_test_content();
        let accessor = InMemoryFileAccessor::new(content);

        // All lines should be accessible immediately (zero-copy extraction)
        for line_num in 0..accessor.total_lines().unwrap() {
            let result = accessor.read_line(line_num).await;
            assert!(result.is_ok(), "Failed to read line {}", line_num);
        }
    }

    #[tokio::test]
    async fn test_repeated_access_performance() {
        let content = create_test_content();
        let accessor = InMemoryFileAccessor::new(content);

        // Multiple accesses should be consistent and fast (zero-copy extraction)
        for _ in 0..100 {
            let line = accessor.read_line(1).await.unwrap();
            assert_eq!(line, "line2");
        }
    }
}
