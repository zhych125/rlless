//! Core file access abstraction and access strategy definitions.
//!
//! This module defines the fundamental FileAccessor trait that provides a consistent
//! interface for different file access implementations, and the AccessStrategy enum
//! that determines the most appropriate access pattern for different use cases.

use crate::error::Result;
use async_trait::async_trait;
use std::borrow::Cow;

/// Information about a search match in the file
#[derive(Debug, Clone)]
pub struct MatchInfo {
    /// Line number where the match was found (0-based)
    pub line_number: u64,

    /// Byte offset from start of file where this line begins
    /// Useful for seeking directly to this position
    pub byte_offset: u64,

    /// Full content of the line containing the match
    /// Already converted to UTF-8 (lossy if needed)
    /// Owned string since MatchInfo is typically consumed for highlighting/display
    pub line_content: String,

    /// Character position within the line where match starts
    /// Used for highlighting in UI
    pub match_start: usize,

    /// Character position within the line where match ends
    /// Used for highlighting in UI
    pub match_end: usize,
}

/// Core trait for file access operations
///
/// This trait provides a unified interface for both small files (loaded into memory)
/// and large files (memory-mapped). All implementations must be thread-safe.
#[async_trait]
pub trait FileAccessor: Send + Sync {
    /// Read a single line by line number
    ///
    /// # Arguments
    /// * `line_number` - 0-based line number to read
    ///
    /// # Returns
    /// * The line content without trailing newline
    /// * Uses Cow for zero-copy when possible, caller decides whether to clone
    /// * Error if line_number is out of bounds
    ///
    /// # Performance
    /// * InMemory: O(1) - zero-copy via Cow::Borrowed
    /// * Mmap: O(1) after indexing, may trigger lazy indexing on first access
    ///
    /// # Usage
    /// Used when user jumps to specific line or navigates with arrow keys
    /// Use .as_ref() for &str, .into_owned() for String, .to_string() for guaranteed String
    async fn read_line(&self, line_number: u64) -> Result<Cow<'_, str>>;

    /// Read multiple consecutive lines efficiently
    ///
    /// # Arguments
    /// * `start` - First line number to read (0-based)
    /// * `count` - Number of lines to read
    ///
    /// # Returns
    /// * Vector of lines, may be shorter than `count` if EOF reached
    /// * Empty vector if `start` is beyond EOF
    /// * Uses Cow for zero-copy when possible, caller decides when to allocate
    ///
    /// # Performance
    /// * InMemory: O(1) per line - zero-copy via Cow::Borrowed
    /// * Optimized for bulk reading (e.g., filling terminal screen)
    ///
    /// # Usage
    /// Used for initial screen fill, page up/down, showing context
    /// Each Cow can be used with .as_ref() for &str or .into_owned() for String
    async fn read_lines_range(&self, start: u64, count: u64) -> Result<Vec<Cow<'_, str>>>;

    /// Find next occurrence of pattern searching forward from start_line
    ///
    /// # Arguments
    /// * `start_line` - Line number to start searching from (inclusive)
    /// * `pattern` - String pattern to search for (substring match)
    ///
    /// # Returns
    /// * Some(MatchInfo) if pattern found
    /// * None if pattern not found before EOF
    ///
    /// # Performance
    /// * Searches incrementally, returns as soon as match found
    /// * Does not scan entire file
    ///
    /// # Usage
    /// Used for 'n' (next) command in search, Find Next in UI
    async fn find_next_match(&self, start_line: u64, pattern: &str) -> Result<Option<MatchInfo>>;

    /// Find previous occurrence of pattern searching backward from start_line
    ///
    /// # Arguments
    /// * `start_line` - Line number to start searching from (exclusive)
    /// * `pattern` - String pattern to search for (substring match)
    ///
    /// # Returns
    /// * Some(MatchInfo) if pattern found
    /// * None if pattern not found before beginning of file
    ///
    /// # Performance
    /// * Searches incrementally backward
    /// * May be slower than forward search due to access patterns
    ///
    /// # Usage
    /// Used for 'N' (previous) command in search, Find Previous in UI
    async fn find_prev_match(&self, start_line: u64, pattern: &str) -> Result<Option<MatchInfo>>;

    /// Get the total file size in bytes
    ///
    /// # Returns
    /// * File size in bytes
    ///
    /// # Performance
    /// * O(1) - cached from file metadata
    ///
    /// # Usage
    /// Used for progress indicators, statistics display
    fn file_size(&self) -> u64;

    /// Get total number of lines in file if known
    ///
    /// # Returns
    /// * Some(count) if line count is known
    /// * None if not yet computed (large files with lazy indexing)
    ///
    /// # Performance
    /// * InMemory: Always returns Some (computed at load time)
    /// * Mmap: Returns None initially, Some after sufficient indexing
    ///
    /// # Usage
    /// Used for scroll bar positioning, jump to percentage, statistics
    fn total_lines(&self) -> Option<u64>;

    /// Check if this implementation supports parallel operations
    ///
    /// # Returns
    /// * true if parallel operations are available
    ///
    /// # Usage
    /// Used to determine if ParallelFileProcessor trait is available
    fn supports_parallel(&self) -> bool {
        false // Default: no parallel support
    }
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

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::error::RllessError;

    /// Mock implementation of FileAccessor for testing
    ///
    /// This implementation stores content in memory and simulates file operations
    /// without requiring actual file I/O, making tests fast and reliable.
    #[derive(Debug)]
    pub struct MockFileAccessor {
        lines: Vec<String>,
        file_size: u64,
    }

    impl MockFileAccessor {
        /// Create a new mock accessor with the given content
        pub fn new(content: impl Into<String>) -> Self {
            let content = content.into();
            let lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();
            let file_size = content.len() as u64;

            Self { lines, file_size }
        }

        /// Create a mock accessor with specific lines
        pub fn from_lines(lines: Vec<String>) -> Self {
            let content = lines.join("\n");
            if !content.is_empty() {
                Self::new(content + "\n")
            } else {
                Self::new(content)
            }
        }
    }

    #[async_trait]
    impl FileAccessor for MockFileAccessor {
        async fn read_line(&self, line_number: u64) -> Result<Cow<'_, str>> {
            let line_idx = line_number as usize;

            if line_idx >= self.lines.len() {
                return Err(RllessError::file_error(
                    &format!(
                        "Line number {} out of bounds (total lines: {})",
                        line_number,
                        self.lines.len()
                    ),
                    std::io::Error::new(std::io::ErrorKind::InvalidInput, "Line out of bounds"),
                ));
            }

            Ok(Cow::Borrowed(&self.lines[line_idx]))
        }

        async fn read_lines_range(&self, start: u64, count: u64) -> Result<Vec<Cow<'_, str>>> {
            let start_idx = start as usize;
            let end_idx = (start_idx + count as usize).min(self.lines.len());

            if start_idx >= self.lines.len() {
                return Ok(Vec::new());
            }

            Ok(self.lines[start_idx..end_idx]
                .iter()
                .map(|s| Cow::Borrowed(s.as_str()))
                .collect())
        }

        async fn find_next_match(
            &self,
            start_line: u64,
            pattern: &str,
        ) -> Result<Option<MatchInfo>> {
            for (i, line) in self.lines.iter().enumerate().skip(start_line as usize) {
                if let Some(match_start) = line.find(pattern) {
                    return Ok(Some(MatchInfo {
                        line_number: i as u64,
                        byte_offset: 0, // Mock doesn't track byte offsets
                        line_content: line.clone(),
                        match_start,
                        match_end: match_start + pattern.len(),
                    }));
                }
            }
            Ok(None)
        }

        async fn find_prev_match(
            &self,
            start_line: u64,
            pattern: &str,
        ) -> Result<Option<MatchInfo>> {
            if start_line == 0 {
                return Ok(None);
            }

            for i in (0..start_line as usize).rev() {
                if let Some(line) = self.lines.get(i) {
                    if let Some(match_start) = line.find(pattern) {
                        return Ok(Some(MatchInfo {
                            line_number: i as u64,
                            byte_offset: 0, // Mock doesn't track byte offsets
                            line_content: line.clone(),
                            match_start,
                            match_end: match_start + pattern.len(),
                        }));
                    }
                }
            }
            Ok(None)
        }

        fn file_size(&self) -> u64 {
            self.file_size
        }

        fn total_lines(&self) -> Option<u64> {
            Some(self.lines.len() as u64)
        }
    }

    #[test]
    fn test_access_strategy_debug() {
        // Test that AccessStrategy can be debugged (important for logging)
        let strategy = AccessStrategy::MemoryMapped;
        let debug_str = format!("{:?}", strategy);
        assert_eq!(debug_str, "MemoryMapped");
    }

    #[tokio::test]
    async fn test_mock_accessor_read_lines_range() {
        let lines = vec![
            "First line".to_string(),
            "Second line".to_string(),
            "Third line".to_string(),
        ];
        let accessor = MockFileAccessor::from_lines(lines.clone());

        // Test reading from the beginning
        let range = accessor.read_lines_range(0, 2).await.unwrap();
        assert_eq!(range, vec!["First line", "Second line"]);

        // Test reading from the middle
        let range = accessor.read_lines_range(1, 2).await.unwrap();
        assert_eq!(range, vec!["Second line", "Third line"]);

        // Test reading beyond end
        let range = accessor.read_lines_range(10, 5).await.unwrap();
        assert!(range.is_empty());

        // Test reading partial range at end
        let range = accessor.read_lines_range(2, 5).await.unwrap();
        assert_eq!(range, vec!["Third line"]);
    }

    #[tokio::test]
    async fn test_mock_accessor_read_line() {
        let lines = vec![
            "First line".to_string(),
            "Second line".to_string(),
            "Third line".to_string(),
        ];
        let accessor = MockFileAccessor::from_lines(lines.clone());

        // Test reading valid lines
        for (i, expected_line) in lines.iter().enumerate() {
            let result = accessor.read_line(i as u64).await.unwrap();
            assert_eq!(result, *expected_line);
        }

        // Test reading out of bounds
        let result = accessor.read_line(10).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            RllessError::FileError { message, .. } => {
                assert!(message.contains("out of bounds"));
            }
            _ => panic!("Expected FileError for line out of bounds"),
        }
    }

    #[tokio::test]
    async fn test_mock_accessor_file_size_and_lines() {
        let content = "Line 1\nLine 2\nLine 3\n";
        let accessor = MockFileAccessor::new(content);

        assert_eq!(accessor.file_size(), content.len() as u64);
        assert_eq!(accessor.total_lines(), Some(3));
    }

    #[tokio::test]
    async fn test_mock_accessor_empty_content() {
        let accessor = MockFileAccessor::new("");

        assert_eq!(accessor.file_size(), 0);
        assert_eq!(accessor.total_lines(), Some(0));

        // Reading from empty content should return empty results
        let range = accessor.read_lines_range(0, 10).await.unwrap();
        assert!(range.is_empty());

        let result = accessor.read_line(0).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_mock_accessor_single_line() {
        let accessor = MockFileAccessor::new("Single line without newline");

        let line = accessor.read_line(0).await.unwrap();
        assert_eq!(line, "Single line without newline");
        assert_eq!(accessor.total_lines(), Some(1));

        // Second line should fail
        let result = accessor.read_line(1).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_mock_accessor_find_next_match() {
        let lines = vec![
            "First line with pattern".to_string(),
            "Second line".to_string(),
            "Third line with pattern again".to_string(),
        ];
        let accessor = MockFileAccessor::from_lines(lines);

        // Test finding from beginning
        let result = accessor.find_next_match(0, "pattern").await.unwrap();
        assert!(result.is_some());
        let match_info = result.unwrap();
        assert_eq!(match_info.line_number, 0);
        assert_eq!(match_info.line_content, "First line with pattern");
        assert_eq!(match_info.match_start, 16);
        assert_eq!(match_info.match_end, 23);

        // Test finding from middle
        let result = accessor.find_next_match(1, "pattern").await.unwrap();
        assert!(result.is_some());
        let match_info = result.unwrap();
        assert_eq!(match_info.line_number, 2);
        assert_eq!(match_info.line_content, "Third line with pattern again");

        // Test not found
        let result = accessor.find_next_match(0, "nonexistent").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_mock_accessor_find_prev_match() {
        let lines = vec![
            "First line with pattern".to_string(),
            "Second line".to_string(),
            "Third line with pattern again".to_string(),
        ];
        let accessor = MockFileAccessor::from_lines(lines);

        // Test finding backward from end
        let result = accessor.find_prev_match(3, "pattern").await.unwrap();
        assert!(result.is_some());
        let match_info = result.unwrap();
        assert_eq!(match_info.line_number, 2);
        assert_eq!(match_info.line_content, "Third line with pattern again");

        // Test finding backward from middle
        let result = accessor.find_prev_match(2, "pattern").await.unwrap();
        assert!(result.is_some());
        let match_info = result.unwrap();
        assert_eq!(match_info.line_number, 0);

        // Test not found from beginning
        let result = accessor.find_prev_match(0, "pattern").await.unwrap();
        assert!(result.is_none());
    }
}
