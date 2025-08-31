//! Core file access abstraction and access strategy definitions.
//!
//! This module defines the fundamental FileAccessor trait that provides a consistent
//! interface for different file access implementations, and the AccessStrategy enum
//! that determines the most appropriate access pattern for different use cases.

use crate::error::Result;
use async_trait::async_trait;

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
        content: String,
        lines: Vec<String>,
        file_size: u64,
    }

    impl MockFileAccessor {
        /// Create a new mock accessor with the given content
        pub fn new(content: impl Into<String>) -> Self {
            let content = content.into();
            let lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();
            let file_size = content.len() as u64;

            Self {
                content,
                lines,
                file_size,
            }
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
        async fn read_range(&self, start: u64, length: usize) -> Result<Vec<u8>> {
            let start = start as usize;
            let end = (start + length).min(self.content.len());
            
            if start >= self.content.len() {
                return Ok(Vec::new());
            }
            
            Ok(self.content[start..end].as_bytes().to_vec())
        }

        async fn read_line(&self, line_number: u64) -> Result<String> {
            let line_idx = line_number as usize;
            
            if line_idx >= self.lines.len() {
                return Err(RllessError::file_error(
                    &format!("Line number {} out of bounds (total lines: {})", 
                        line_number, self.lines.len()),
                    std::io::Error::new(std::io::ErrorKind::InvalidInput, "Line out of bounds")
                ));
            }
            
            Ok(self.lines[line_idx].clone())
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
    async fn test_mock_accessor_read_range() {
        let content = "Hello, World!\nSecond line\nThird line\n";
        let accessor = MockFileAccessor::new(content);
        
        // Test reading from the beginning
        let range = accessor.read_range(0, 5).await.unwrap();
        assert_eq!(String::from_utf8(range).unwrap(), "Hello");
        
        // Test reading from the middle
        let range = accessor.read_range(7, 5).await.unwrap();
        assert_eq!(String::from_utf8(range).unwrap(), "World");
        
        // Test reading beyond end
        let range = accessor.read_range(100, 10).await.unwrap();
        assert!(range.is_empty());
        
        // Test reading partial range at end
        let content_len = content.len() as u64;
        let range = accessor.read_range(content_len - 3, 10).await.unwrap();
        assert_eq!(String::from_utf8(range).unwrap(), "ne\n");
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
            },
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
        let range = accessor.read_range(0, 10).await.unwrap();
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
}