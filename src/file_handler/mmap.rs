//! Memory-mapped file accessor for large files
//!
//! This module provides the MmapFileAccessor implementation that uses memory mapping
//! for efficient access to large files with lazy line indexing and zero-copy string extraction.

use crate::error::{Result, RllessError};
use crate::file_handler::accessor::{FileAccessor, MatchInfo};
use crate::file_handler::line_index::LineIndex;
use async_trait::async_trait;
use memmap2::Mmap;
use parking_lot::RwLock;
use std::borrow::Cow;
use std::fs::File;
use std::path::Path;

/// Memory-mapped file accessor for large files
///
/// This implementation uses memory mapping to provide efficient access to large files
/// without loading the entire content into memory. It builds line indices lazily
/// as lines are accessed and provides zero-copy string extraction directly from
/// the mapped memory.
#[derive(Debug)]
pub struct MmapFileAccessor {
    /// Memory-mapped file handle
    ///
    /// Provides direct access to file content via OS virtual memory system.
    /// Pages are loaded on demand as they are accessed.
    mmap: Mmap,

    /// File size in bytes (cached for performance)
    file_size: u64,

    /// LineIndex for lazy line boundary detection (thread-safe)
    ///
    /// Uses RwLock because ensure_indexed_to requires &mut self
    /// but FileAccessor trait methods only take &self.
    /// Multiple readers can access simultaneously, writers get exclusive access.
    line_index: RwLock<LineIndex>,
}

impl MmapFileAccessor {
    /// Create a new memory-mapped file accessor
    ///
    /// # Arguments
    /// * `file_path` - Path to the file to memory map
    ///
    /// # Returns
    /// * New MmapFileAccessor instance ready for zero-copy access
    ///
    /// # Performance
    /// * O(1) - Just sets up memory mapping, no file content scanning
    /// * Actual file pages are loaded by OS on first access
    pub async fn new(file_path: impl AsRef<Path>) -> Result<Self> {
        let file_path = file_path.as_ref();

        // Open file and get metadata
        let file = File::open(file_path).map_err(|e| {
            RllessError::file_error(format!("Failed to open file: {}", file_path.display()), e)
        })?;

        let file_size = file
            .metadata()
            .map_err(|e| RllessError::file_error("Failed to get file metadata", e))?
            .len();

        // Create memory mapping
        let mmap = unsafe {
            Mmap::map(&file).map_err(|e| {
                RllessError::file_error(
                    format!("Failed to memory map file: {}", file_path.display()),
                    e,
                )
            })?
        };

        // Advise kernel about our access pattern on Unix systems
        #[cfg(unix)]
        {
            // Start with sequential advice for potential initial scanning
            // This can be changed later based on access patterns
            if let Err(e) = mmap.advise(memmap2::Advice::Sequential) {
                // Non-fatal - log and continue
                eprintln!("Warning: Failed to set mmap advice: {}", e);
            }
        }

        Ok(Self {
            mmap,
            file_size,
            line_index: RwLock::new(LineIndex::new()),
        })
    }

    /// Extract line content directly from memory-mapped file (zero-copy)
    ///
    /// # Arguments
    /// * `line_number` - 0-based line number
    ///
    /// # Returns
    /// * Line content as Cow<str> - zero-copy Cow::Borrowed for valid UTF-8
    /// * Error if line number is out of bounds
    ///
    /// # Performance
    /// * O(1) - Direct extraction using pre-computed line boundaries
    /// * May trigger lazy indexing on first access to a region
    fn extract_line(&self, line_number: u64) -> Result<Cow<'_, str>> {
        // 1. Ensure we have indexed up to this line
        {
            let mut index = self.line_index.write();
            index.ensure_indexed_to(&self.mmap[..], line_number);
        }

        // 2. Get line byte range from line index
        let (start, end) = {
            let index = self.line_index.read();
            index.get_line_range(line_number).ok_or_else(|| {
                RllessError::file_error(
                    format!("Line number {} out of bounds", line_number),
                    std::io::Error::new(std::io::ErrorKind::InvalidInput, "Line out of bounds"),
                )
            })?
        };

        // 3. Direct slice into memory-mapped file (zero-copy!)
        let line_bytes = &self.mmap[start as usize..end as usize];

        // 4. Zero-copy UTF-8 conversion (Cow::Borrowed for valid UTF-8)
        Ok(String::from_utf8_lossy(line_bytes))
    }
}

#[async_trait]
impl FileAccessor for MmapFileAccessor {
    async fn read_line(&self, line_number: u64) -> Result<Cow<'_, str>> {
        self.extract_line(line_number)
    }

    async fn read_lines_range(&self, start: u64, count: u64) -> Result<Vec<Cow<'_, str>>> {
        if count == 0 {
            return Ok(Vec::new());
        }

        let mut lines = Vec::with_capacity(count.min(1000) as usize);

        for line_num in start..start + count {
            match self.extract_line(line_num) {
                Ok(line) => lines.push(line),
                Err(_) => break, // EOF reached
            }
        }

        Ok(lines)
    }

    async fn find_next_match(
        &self,
        start_line: u64,
        search_fn: &(dyn for<'a> Fn(&'a str) -> Vec<(usize, usize)> + Send + Sync),
    ) -> Result<Option<MatchInfo>> {
        let mut current_line = start_line;

        // Search forward line by line
        loop {
            match self.extract_line(current_line) {
                Ok(line_content) => {
                    let match_ranges = search_fn(&line_content);
                    if !match_ranges.is_empty() {
                        // Get byte offset for this line
                        let byte_offset = {
                            let index = self.line_index.read();
                            let offsets = index.get_line_offsets();
                            if current_line < offsets.len() as u64 {
                                offsets[current_line as usize]
                            } else {
                                0 // Fallback, should not happen
                            }
                        };

                        return Ok(Some(MatchInfo {
                            line_number: current_line,
                            byte_offset,
                            line_content: line_content.into_owned(), // MatchInfo needs owned String
                            match_ranges,
                            context_before: Vec::new(),
                            context_after: Vec::new(),
                        }));
                    }
                    current_line += 1;
                }
                Err(_) => return Ok(None), // EOF reached
            }

            // Safety limit for very large files to prevent infinite loops
            if current_line > start_line + 1_000_000 {
                return Ok(None);
            }
        }
    }

    async fn find_prev_match(
        &self,
        start_line: u64,
        search_fn: &(dyn for<'a> Fn(&'a str) -> Vec<(usize, usize)> + Send + Sync),
    ) -> Result<Option<MatchInfo>> {
        if start_line == 0 {
            return Ok(None);
        }

        // Search backward from start_line - 1
        for current_line in (0..start_line).rev() {
            match self.extract_line(current_line) {
                Ok(line_content) => {
                    let match_ranges = search_fn(&line_content);
                    if !match_ranges.is_empty() {
                        // Get byte offset for this line
                        let byte_offset = {
                            let index = self.line_index.read();
                            let offsets = index.get_line_offsets();
                            if current_line < offsets.len() as u64 {
                                offsets[current_line as usize]
                            } else {
                                0 // Fallback
                            }
                        };

                        return Ok(Some(MatchInfo {
                            line_number: current_line,
                            byte_offset,
                            line_content: line_content.into_owned(), // MatchInfo needs owned String
                            match_ranges,
                            context_before: Vec::new(),
                            context_after: Vec::new(),
                        }));
                    }
                }
                Err(_) => continue, // Skip lines that can't be read
            }
        }

        Ok(None)
    }

    fn file_size(&self) -> u64 {
        self.file_size
    }

    fn total_lines(&self) -> Option<u64> {
        // For lazy/large files, we don't precompute total lines
        // Return None until we need to scan the entire file
        // This is different from InMemoryFileAccessor which always knows the total
        None
    }

    fn supports_parallel(&self) -> bool {
        // Currently not implemented - would require careful design around
        // shared LineIndex updates and thread-safe line range caching
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    /// Create a temporary test file with known content
    fn create_test_file(content: &str) -> NamedTempFile {
        let mut file = NamedTempFile::new().expect("Failed to create temp file");
        file.write_all(content.as_bytes())
            .expect("Failed to write test data");
        file.flush().expect("Failed to flush test data");
        file
    }

    /// Create a test file with specific content
    fn create_test_content() -> &'static str {
        "line1\nline2\nline3\nline4\n"
    }

    /// Create complex test content with various line characteristics
    fn create_complex_content() -> &'static str {
        "short\na longer line with content\n\nempty line above\nfinal line without newline"
    }

    #[tokio::test]
    async fn test_new_mmap_accessor() {
        let content = create_test_content();
        let temp_file = create_test_file(content);

        let accessor = MmapFileAccessor::new(temp_file.path()).await.unwrap();

        assert_eq!(accessor.file_size(), content.len() as u64);
        assert_eq!(accessor.total_lines(), None); // Lazy loading
        assert!(!accessor.supports_parallel());
    }

    #[tokio::test]
    async fn test_read_line() {
        let content = create_test_content();
        let temp_file = create_test_file(content);
        let accessor = MmapFileAccessor::new(temp_file.path()).await.unwrap();

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
        let temp_file = create_test_file(content);
        let accessor = MmapFileAccessor::new(temp_file.path()).await.unwrap();

        let result = accessor.read_line(999).await;
        assert!(result.is_err());

        let result = accessor.read_line(4).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_read_lines_range() {
        let content = create_test_content();
        let temp_file = create_test_file(content);
        let accessor = MmapFileAccessor::new(temp_file.path()).await.unwrap();

        let lines = accessor.read_lines_range(1, 2).await.unwrap();
        assert_eq!(lines, vec!["line2", "line3"]);

        let lines = accessor.read_lines_range(3, 5).await.unwrap();
        assert_eq!(lines, vec!["line4"]);

        let lines = accessor.read_lines_range(10, 5).await.unwrap();
        assert!(lines.is_empty());
    }

    #[tokio::test]
    async fn test_find_next_match() {
        let content = "error on line1\nno match here\nerror on line3\nfinal line\n";
        let temp_file = create_test_file(content);
        let accessor = MmapFileAccessor::new(temp_file.path()).await.unwrap();

        // Create a search function for "error"
        let error_search = |line: &str| {
            let mut matches = Vec::new();
            let mut start = 0;
            while let Some(pos) = line[start..].find("error") {
                let match_start = start + pos;
                let match_end = match_start + "error".len();
                matches.push((match_start, match_end));
                start = match_end;
            }
            matches
        };

        let match_info = accessor.find_next_match(0, &error_search).await.unwrap();
        assert!(match_info.is_some());

        let info = match_info.unwrap();
        assert_eq!(info.line_number, 0);
        assert_eq!(info.line_content, "error on line1");
        assert_eq!(info.match_ranges, vec![(0, 5)]);

        let match_info = accessor.find_next_match(1, &error_search).await.unwrap();
        assert!(match_info.is_some());
        assert_eq!(match_info.unwrap().line_number, 2);

        let no_match_search = |_line: &str| Vec::new();
        let match_info = accessor.find_next_match(0, &no_match_search).await.unwrap();
        assert!(match_info.is_none());
    }

    #[tokio::test]
    async fn test_find_prev_match() {
        let content = "error on line1\nno match here\nerror on line3\nfinal line\n";
        let temp_file = create_test_file(content);
        let accessor = MmapFileAccessor::new(temp_file.path()).await.unwrap();

        // Create a search function for "error"
        let error_search = |line: &str| {
            let mut matches = Vec::new();
            let mut start = 0;
            while let Some(pos) = line[start..].find("error") {
                let match_start = start + pos;
                let match_end = match_start + "error".len();
                matches.push((match_start, match_end));
                start = match_end;
            }
            matches
        };

        let match_info = accessor.find_prev_match(4, &error_search).await.unwrap();
        assert!(match_info.is_some());
        assert_eq!(match_info.unwrap().line_number, 2);

        let match_info = accessor.find_prev_match(2, &error_search).await.unwrap();
        assert!(match_info.is_some());
        assert_eq!(match_info.unwrap().line_number, 0);

        let match_info = accessor.find_prev_match(0, &error_search).await.unwrap();
        assert!(match_info.is_none());
    }

    #[tokio::test]
    async fn test_empty_file() {
        let temp_file = create_test_file("");
        let accessor = MmapFileAccessor::new(temp_file.path()).await.unwrap();

        assert_eq!(accessor.file_size(), 0);
        assert_eq!(accessor.total_lines(), None);

        let result = accessor.read_line(0).await;
        assert!(result.is_err());

        let lines = accessor.read_lines_range(0, 1).await.unwrap();
        assert!(lines.is_empty());
    }

    #[tokio::test]
    async fn test_single_line_no_newline() {
        let content = "single line without newline";
        let temp_file = create_test_file(content);
        let accessor = MmapFileAccessor::new(temp_file.path()).await.unwrap();

        assert_eq!(accessor.total_lines(), None); // Lazy loading

        let line = accessor.read_line(0).await.unwrap();
        assert_eq!(line, "single line without newline");

        let result = accessor.read_line(1).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_complex_content() {
        let content = create_complex_content();
        let temp_file = create_test_file(content);
        let accessor = MmapFileAccessor::new(temp_file.path()).await.unwrap();

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
    async fn test_repeated_access() {
        let content = create_test_content();
        let temp_file = create_test_file(content);
        let accessor = MmapFileAccessor::new(temp_file.path()).await.unwrap();

        // Multiple accesses should be consistent (zero-copy efficiency)
        for _ in 0..10 {
            let line = accessor.read_line(1).await.unwrap();
            assert_eq!(line, "line2");
        }
    }

    #[tokio::test]
    async fn test_concurrent_access() {
        let content = create_test_content();
        let temp_file = create_test_file(content);
        let accessor = std::sync::Arc::new(MmapFileAccessor::new(temp_file.path()).await.unwrap());

        // Test concurrent read access
        let handles: Vec<_> = (0..10)
            .map(|i| {
                let accessor = accessor.clone();
                tokio::spawn(async move {
                    let line = accessor.read_line(i % 4).await.unwrap();
                    assert!(line.starts_with("line"));
                })
            })
            .collect();

        for handle in handles {
            handle.await.unwrap();
        }
    }
}
