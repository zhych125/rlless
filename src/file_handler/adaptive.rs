//! Adaptive file accessor that handles in-memory, memory-mapped, and compressed files
//!
//! This module provides a single implementation that adapts its internal strategy
//! based on file characteristics determined by the FileAccessorFactory.

use crate::error::Result;
use crate::file_handler::accessor::FileAccessor;
use async_trait::async_trait;
use memmap2::Mmap;
use std::path::Path;
use tempfile::NamedTempFile;

/// Internal byte source strategy for AdaptiveFileAccessor
#[derive(Debug)]
pub enum ByteSource {
    /// Content loaded entirely into memory (for files < 50MB)
    InMemory(Vec<u8>),
    /// Content accessed via memory mapping (for files ≥ 50MB)
    MemoryMapped(Mmap),
    /// Compressed file decompressed to temp file and memory-mapped
    /// The temp file is kept alive to prevent deletion
    Compressed {
        mmap: Mmap,
        _temp_file: NamedTempFile,
    },
}

impl ByteSource {
    /// Get the underlying bytes as a slice regardless of storage strategy
    fn as_bytes(&self) -> &[u8] {
        match self {
            ByteSource::InMemory(vec) => vec.as_slice(),
            ByteSource::MemoryMapped(mmap) => &mmap[..],
            ByteSource::Compressed { mmap, .. } => &mmap[..],
        }
    }

    /// Convert bytes to String
    fn bytes_to_string(&self, bytes: &[u8]) -> Result<String> {
        std::str::from_utf8(bytes)
            .map(|s| s.to_string())
            .map_err(|e| {
                crate::error::RllessError::file_error(
                    "Invalid UTF-8 in file",
                    std::io::Error::new(std::io::ErrorKind::InvalidData, e),
                )
            })
    }
}

/// Adaptive file accessor that uses different internal strategies
///
/// This accessor adapts its internal storage strategy (`ByteSource`) based on file
/// characteristics determined by the `FileAccessorFactory`. It provides a unified
/// interface regardless of whether the file is stored in memory, memory-mapped,
/// or decompressed from a compressed format.
#[derive(Debug)]
pub struct AdaptiveFileAccessor {
    pub(crate) source: ByteSource,
    file_size: u64,
    file_path: std::path::PathBuf,
}

impl AdaptiveFileAccessor {
    /// Create a new adaptive file accessor
    ///
    /// # Arguments
    /// * `source` - The internal byte source strategy to use
    /// * `file_size` - Size of the file content in bytes
    /// * `file_path` - Path to the original file
    pub fn new(source: ByteSource, file_size: u64, file_path: std::path::PathBuf) -> Self {
        Self {
            source,
            file_size,
            file_path,
        }
    }
}

#[async_trait]
impl FileAccessor for AdaptiveFileAccessor {
    async fn read_from_byte(&self, start_byte: u64, max_lines: usize) -> Result<Vec<String>> {
        let bytes = self.source.as_bytes();
        if start_byte as usize >= bytes.len() {
            return Ok(Vec::new());
        }

        let mut lines = Vec::new();
        let mut current_pos = start_byte as usize;
        let mut lines_read = 0;

        while lines_read < max_lines && current_pos < bytes.len() {
            // Find the end of the current line
            let line_end = memchr::memchr(b'\n', &bytes[current_pos..])
                .map(|pos| current_pos + pos)
                .unwrap_or(bytes.len());

            // Extract the line content (without newline)
            let line_bytes = &bytes[current_pos..line_end];
            let line_str = self.source.bytes_to_string(line_bytes)?;

            lines.push(line_str);
            lines_read += 1;

            // Move to the start of the next line
            current_pos = if line_end < bytes.len() {
                line_end + 1 // Skip the newline character
            } else {
                break; // End of file
            };
        }

        Ok(lines)
    }

    async fn find_next_match(
        &self,
        start_byte: u64,
        search_fn: &(dyn for<'a> Fn(&'a str) -> Vec<(usize, usize)> + Send + Sync),
    ) -> Result<Option<u64>> {
        let bytes = self.source.as_bytes();
        if start_byte as usize >= bytes.len() {
            return Ok(None);
        }

        let mut current_pos = start_byte as usize;

        while current_pos < bytes.len() {
            // Find the end of the current line
            let line_end = memchr::memchr(b'\n', &bytes[current_pos..])
                .map(|pos| current_pos + pos)
                .unwrap_or(bytes.len());

            // Extract the line content
            let line_bytes = &bytes[current_pos..line_end];
            if let Ok(line_str) = std::str::from_utf8(line_bytes) {
                let matches = search_fn(line_str);
                if !matches.is_empty() {
                    return Ok(Some(current_pos as u64));
                }
            }

            // Move to the start of the next line
            current_pos = if line_end < bytes.len() {
                line_end + 1
            } else {
                break;
            };
        }

        Ok(None)
    }

    async fn find_prev_match(
        &self,
        start_byte: u64,
        search_fn: &(dyn for<'a> Fn(&'a str) -> Vec<(usize, usize)> + Send + Sync),
    ) -> Result<Option<u64>> {
        let bytes = self.source.as_bytes();
        if start_byte == 0 {
            return Ok(None);
        }

        // Start from one byte before start_byte to exclude current line
        let mut search_pos = (start_byte as usize).min(bytes.len()).saturating_sub(1);

        // Search backward line by line
        loop {
            // Find the start of the line containing search_pos
            let line_start = if search_pos == 0 {
                0
            } else {
                // Look for newline before search_pos
                match memchr::memrchr(b'\n', &bytes[0..search_pos]) {
                    Some(newline_pos) => newline_pos + 1, // Start of line is after the newline
                    None => 0, // No newline found, this is the first line
                }
            };

            // search_pos should be at a newline, so it's the end of the line we want
            let line_end = search_pos;

            // Extract and check the line content
            let line_bytes = &bytes[line_start..line_end];
            if let Ok(line_str) = std::str::from_utf8(line_bytes) {
                let matches = search_fn(line_str);
                if !matches.is_empty() {
                    return Ok(Some(line_start as u64));
                }
            }

            // Move to search the previous line
            if line_start == 0 {
                return Ok(None); // No more lines to search
            }
            search_pos = line_start - 1; // Move to the byte before this line starts
        }
    }

    fn file_size(&self) -> u64 {
        self.file_size
    }

    fn file_path(&self) -> &Path {
        &self.file_path
    }

    async fn last_page_start(&self, max_lines: usize) -> Result<u64> {
        let bytes = self.source.as_bytes();
        if bytes.is_empty() || max_lines == 0 {
            return Ok(0);
        }

        let mut search_pos = bytes.len();

        // Skip trailing newline if present (it doesn't count as a line separator)
        if bytes.last() == Some(&b'\n') {
            search_pos = search_pos.saturating_sub(1);
        }

        // Find max_lines newline characters from the end
        for _ in 0..max_lines {
            match memchr::memrchr(b'\n', &bytes[0..search_pos]) {
                Some(newline_pos) => {
                    search_pos = newline_pos;
                }
                None => {
                    // We hit the start of the file without finding enough newlines
                    return Ok(0);
                }
            }
        }

        // Return position after the last found newline
        Ok((search_pos + 1) as u64)
    }

    async fn next_page_start(&self, current_byte: u64, lines_to_skip: usize) -> Result<u64> {
        let bytes = self.source.as_bytes();
        let mut pos = current_byte as usize;
        let mut lines_skipped = 0;

        while pos < bytes.len() && lines_skipped < lines_to_skip {
            // Find the next newline
            if let Some(newline_pos) = memchr::memchr(b'\n', &bytes[pos..]) {
                pos += newline_pos + 1; // Move past the newline
                lines_skipped += 1;
            } else {
                // No more newlines, we're at the end
                break;
            }
        }

        // If we couldn't complete the full skip due to EOF, return file_size
        if lines_skipped < lines_to_skip {
            Ok(self.file_size) // Return EOF indicator
        } else {
            Ok(pos as u64) // Return new position
        }
    }

    async fn prev_page_start(&self, current_byte: u64, lines_to_skip: usize) -> Result<u64> {
        let bytes = self.source.as_bytes();
        if current_byte == 0 || lines_to_skip == 0 {
            return Ok(0);
        }

        // Start from one byte before current_byte to exclude current line
        let mut search_pos = (current_byte as usize).saturating_sub(1);

        // Find lines_to_skip newlines going backward
        for _ in 0..lines_to_skip {
            match memchr::memrchr(b'\n', &bytes[0..search_pos]) {
                Some(newline_pos) => {
                    search_pos = newline_pos;
                }
                None => {
                    // We hit the start of the file without finding enough newlines
                    return Ok(0);
                }
            }
        }

        // Return position after the last found newline
        Ok((search_pos + 1) as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file_handler::factory::FileAccessorFactory;
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use std::io::Write;
    use tempfile::NamedTempFile;

    /// Create a temporary test file with known content
    fn create_test_file(content: &[u8]) -> NamedTempFile {
        let mut file = NamedTempFile::new().expect("Failed to create temp file");
        file.write_all(content).expect("Failed to write test data");
        file.flush().expect("Failed to flush test data");
        file
    }

    #[tokio::test]
    async fn test_adaptive_accessor_small_file() {
        let content = b"line1\nline2\nline3\n";
        let temp_file = create_test_file(content);

        let accessor = FileAccessorFactory::create(temp_file.path()).await.unwrap();

        assert_eq!(accessor.file_size(), content.len() as u64);
        assert_eq!(accessor.file_path(), temp_file.path());

        // Should use InMemory for small file
        match &accessor.source {
            ByteSource::InMemory(_) => {} // Expected
            _ => panic!("Small file should use InMemory variant"),
        }
    }

    #[tokio::test]
    async fn test_adaptive_accessor_read_from_byte() {
        let content = b"line1\nline2\nline3\n";
        let temp_file = create_test_file(content);
        let accessor = FileAccessorFactory::create(temp_file.path()).await.unwrap();

        // Read from beginning
        let lines = accessor.read_from_byte(0, 2).await.unwrap();
        assert_eq!(lines, vec!["line1", "line2"]);

        // Read from middle (byte 6 is start of "line2")
        let lines = accessor.read_from_byte(6, 2).await.unwrap();
        assert_eq!(lines, vec!["line2", "line3"]);

        // Read from end of file
        let lines = accessor.read_from_byte(100, 2).await.unwrap();
        assert!(lines.is_empty());

        // Read with limit
        let lines = accessor.read_from_byte(0, 1).await.unwrap();
        assert_eq!(lines, vec!["line1"]);
    }

    #[tokio::test]
    async fn test_adaptive_accessor_find_next_match() {
        let content = b"error line\nnormal line\nerror again\n";
        let temp_file = create_test_file(content);
        let accessor = FileAccessorFactory::create(temp_file.path()).await.unwrap();

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

        // Find first match
        let result = accessor.find_next_match(0, &error_search).await.unwrap();
        assert_eq!(result, Some(0));

        // Find second match
        let result = accessor.find_next_match(15, &error_search).await.unwrap();
        assert!(result.is_some());
        let byte_pos = result.unwrap();
        assert!(byte_pos > 15);

        // No match found
        let no_match_search = |_line: &str| Vec::new();
        let result = accessor.find_next_match(0, &no_match_search).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_adaptive_accessor_find_prev_match() {
        let content = b"error line\nnormal line\nerror again\n";
        let temp_file = create_test_file(content);
        let accessor = FileAccessorFactory::create(temp_file.path()).await.unwrap();

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

        // Find match searching backward from end
        let result = accessor.find_prev_match(100, &error_search).await.unwrap();
        assert!(result.is_some());

        // No match from beginning
        let result = accessor.find_prev_match(0, &error_search).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_adaptive_accessor_navigation_methods() {
        let content = b"line1\nline2\nline3\nline4\nline5\n";
        let temp_file = create_test_file(content);
        let accessor = FileAccessorFactory::create(temp_file.path()).await.unwrap();

        // Test last page start
        let last_pos = accessor.last_page_start(3).await.unwrap();
        assert_eq!(last_pos, 12); // Should start at "line3"

        // Test next page start - normal case
        let next_pos = accessor.next_page_start(0, 2).await.unwrap();
        assert_eq!(next_pos, 12); // Should be at start of "line3"

        // Test next page start - at end of file (should return file_size when can't complete skip)
        let end_pos = accessor.next_page_start(24, 2).await.unwrap(); // Start of "line5", try to skip 2 lines
        assert_eq!(end_pos, 30); // Should return file_size (30) since we can only skip 1 line, not 2

        // Test prev page start
        let prev_pos = accessor.prev_page_start(next_pos, 2).await.unwrap();
        assert_eq!(prev_pos, 0); // Should go back to start
    }

    #[tokio::test]
    async fn test_adaptive_accessor_empty_file() {
        let content = b"";
        let temp_file = create_test_file(content);

        // FileAccessorFactory should reject empty files during validation
        let result = FileAccessorFactory::create(temp_file.path()).await;
        assert!(result.is_err());

        let error = result.err().unwrap();
        match error {
            crate::error::RllessError::FileError { message, .. } => {
                assert!(message.contains("File is empty"));
            }
            _ => panic!("Expected FileError for empty file"),
        }
    }

    #[tokio::test]
    async fn test_adaptive_accessor_single_line_no_newline() {
        let content = b"single line without newline";
        let temp_file = create_test_file(content);
        let accessor = FileAccessorFactory::create(temp_file.path()).await.unwrap();

        let lines = accessor.read_from_byte(0, 5).await.unwrap();
        assert_eq!(lines, vec!["single line without newline"]);

        let last_pos = accessor.last_page_start(1).await.unwrap();
        assert_eq!(last_pos, 0);
    }

    #[tokio::test]
    async fn test_adaptive_accessor_next_page_start_edge_cases() {
        // Test file ending with newline
        let content = b"A\nB\nC\n";
        let temp_file = create_test_file(content);
        let accessor = FileAccessorFactory::create(temp_file.path()).await.unwrap();

        // Normal navigation
        let pos = accessor.next_page_start(0, 1).await.unwrap();
        assert_eq!(pos, 2); // Start of "B"

        let pos = accessor.next_page_start(2, 1).await.unwrap();
        assert_eq!(pos, 4); // Start of "C"

        // At last line - should return file_size since we can't skip a full line
        let pos = accessor.next_page_start(4, 1).await.unwrap();
        assert_eq!(pos, 6); // Returns file_size (6) indicating EOF
    }

    #[tokio::test]
    async fn test_adaptive_accessor_last_page_start_comprehensive() {
        // Test case: "A\nB\nC\nD\nE\n" (5 lines, ends with newline)
        let content = b"A\nB\nC\nD\nE\n";
        let temp_file = create_test_file(content);
        let accessor = FileAccessorFactory::create(temp_file.path()).await.unwrap();

        // Request 1 line: should get last line (E)
        let last_pos = accessor.last_page_start(1).await.unwrap();
        assert_eq!(last_pos, 8); // Start of "E"

        // Request 2 lines: should get D and E
        let last_pos = accessor.last_page_start(2).await.unwrap();
        assert_eq!(last_pos, 6); // Start of "D"

        // Request 3 lines: should get C, D, and E
        let last_pos = accessor.last_page_start(3).await.unwrap();
        assert_eq!(last_pos, 4); // Start of "C"

        // Request 5 lines: should get all lines from beginning
        let last_pos = accessor.last_page_start(5).await.unwrap();
        assert_eq!(last_pos, 0); // Start of "A"

        // Request more than available: should get all lines from beginning
        let last_pos = accessor.last_page_start(10).await.unwrap();
        assert_eq!(last_pos, 0);
    }

    #[tokio::test]
    async fn test_adaptive_accessor_compressed_file() {
        // Create a small compressed file
        let test_data = b"compressed line 1\ncompressed line 2\ncompressed line 3\n";
        let temp_file = NamedTempFile::new().unwrap();
        {
            let mut encoder = GzEncoder::new(
                std::fs::File::create(temp_file.path()).unwrap(),
                Compression::default(),
            );
            encoder.write_all(test_data).unwrap();
            encoder.finish().unwrap();
        }

        let accessor = FileAccessorFactory::create(temp_file.path()).await.unwrap();

        // Should use InMemory for small compressed file after decompression
        assert!(
            matches!(accessor.source, ByteSource::InMemory(_)),
            "expected in-memory accessor for small compressed file, found {:?}",
            accessor.source
        );

        // Test that decompression worked
        let lines = accessor.read_from_byte(0, 3).await.unwrap();
        assert_eq!(lines[0], "compressed line 1");
        assert_eq!(lines[1], "compressed line 2");
        assert_eq!(lines[2], "compressed line 3");
    }

    #[tokio::test]
    async fn test_adaptive_accessor_string_conversion() {
        let content = b"test line for borrowing\n";
        let temp_file = create_test_file(content);
        let accessor = FileAccessorFactory::create(temp_file.path()).await.unwrap();

        let lines = accessor.read_from_byte(0, 1).await.unwrap();

        // Verify we get the expected string
        assert_eq!(lines[0], "test line for borrowing");
    }

    #[test]
    fn test_byte_source_variants() {
        let vec_data = vec![65, 10, 66, 10]; // "A\nB\n"
        let in_memory = ByteSource::InMemory(vec_data);

        assert_eq!(in_memory.as_bytes(), &[65, 10, 66, 10]);

        let string_result = in_memory.bytes_to_string(&[65]).unwrap();
        assert_eq!(string_result, "A");
    }
}
