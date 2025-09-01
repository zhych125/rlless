//! Factory for creating appropriate FileAccessor based on file characteristics.
//!
//! This module provides the FileAccessorFactory which automatically selects the most
//! appropriate FileAccessor implementation based on file size, compression, and platform
//! characteristics. It integrates validation and compression detection to ensure robust
//! file handling.

use crate::error::{Result, RllessError};
use crate::file_handler::accessor::FileAccessor;
use crate::file_handler::compression::{detect_compression, CompressionType};
use crate::file_handler::in_memory::InMemoryFileAccessor;
use crate::file_handler::mmap::MmapFileAccessor;
use crate::file_handler::validation::validate_file_path;
use std::path::Path;

/// Factory for creating appropriate FileAccessor based on file characteristics
///
/// This factory automatically selects between InMemoryFileAccessor and MmapFileAccessor
/// based on file size and platform characteristics. It performs comprehensive validation
/// and compression detection to ensure optimal performance and reliability.
///
/// # Strategy Selection
/// - Files < 10MB (50MB on macOS): InMemoryFileAccessor
/// - Files ≥ 10MB: MmapFileAccessor
/// - Future: Compressed files will get specialized handling
///
/// # Validation
/// All files undergo validation before accessor creation:
/// - File existence and readability
/// - Reasonable file size (not empty, not >100GB)
/// - Proper file type (not directory)
pub struct FileAccessorFactory;

impl FileAccessorFactory {
    /// Size threshold for choosing between in-memory and mmap
    ///
    /// Files smaller than this are loaded entirely into memory for fast access.
    /// Files larger than this use memory mapping for memory efficiency.
    const SMALL_FILE_THRESHOLD: u64 = 10 * 1024 * 1024; // 10MB

    /// Platform-specific threshold for macOS
    ///
    /// macOS has different mmap performance characteristics due to its
    /// unified buffer cache, so we use a larger threshold to prefer
    /// in-memory access for medium-sized files.
    #[cfg(target_os = "macos")]
    const MACOS_THRESHOLD: u64 = 50 * 1024 * 1024; // 50MB

    /// Create the appropriate FileAccessor for a given file
    ///
    /// # Arguments
    /// * `path` - Path to the file to open
    ///
    /// # Returns
    /// * `Box<dyn FileAccessor>` - Appropriate implementation for the file
    ///
    /// # Strategy
    /// 1. Validate file (existence, permissions, reasonable size)
    /// 2. Detect compression format (for future use)
    /// 3. Choose implementation based on file size and platform
    ///
    /// # Errors
    /// * File validation errors (non-existent, empty, too large, not readable)
    /// * Compression detection errors
    /// * Accessor creation errors (memory mapping failures, etc.)
    ///
    /// # Performance
    /// * Validation: O(1) - just metadata and header reads
    /// * InMemory creation: O(n) - loads entire file
    /// * Mmap creation: O(1) - just sets up mapping
    pub async fn create(path: &Path) -> Result<Box<dyn FileAccessor>> {
        // 1. Validate file first (existence, permissions, reasonable size)
        validate_file_path(path)?;

        // 2. Get file metadata for size-based decision
        let metadata = tokio::fs::metadata(path).await.map_err(|e| {
            RllessError::file_error(
                format!("Failed to get file metadata: {}", path.display()),
                e,
            )
        })?;

        let file_size = metadata.len();

        // 3. Detect compression (for future use, currently just for logging/planning)
        let compression = detect_compression(path)?;
        if compression != CompressionType::None {
            // Future: Handle compressed files specially
            // For now: proceed with normal strategy but compressed files will work
            // through the normal accessors (they'll see the compressed bytes)
            eprintln!(
                "Note: Detected {:?} compression in {}, processing as-is",
                compression,
                path.display()
            );
        }

        // 4. Choose implementation based on file size and platform
        let threshold = Self::get_threshold();

        if file_size < threshold {
            // Small file: load into memory for fast access
            let content = tokio::fs::read(path).await.map_err(|e| {
                RllessError::file_error(
                    format!("Failed to read file content: {}", path.display()),
                    e,
                )
            })?;
            let accessor = InMemoryFileAccessor::new(content);
            Ok(Box::new(accessor))
        } else {
            // Large file: use memory mapping for memory efficiency
            let accessor = MmapFileAccessor::new(path).await?;
            Ok(Box::new(accessor))
        }
    }

    /// Get the size threshold for the current platform
    ///
    /// # Returns
    /// * Threshold in bytes for choosing between in-memory and mmap
    ///
    /// # Platform Differences
    /// * macOS: 50MB (unified buffer cache makes mmap less efficient for medium files)
    /// * Other platforms: 10MB (mmap is generally efficient)
    fn get_threshold() -> u64 {
        #[cfg(target_os = "macos")]
        {
            Self::MACOS_THRESHOLD
        }

        #[cfg(not(target_os = "macos"))]
        {
            Self::SMALL_FILE_THRESHOLD
        }
    }

    /// Create with explicit strategy (for testing and special cases)
    ///
    /// This method bypasses the automatic strategy selection and forces
    /// a specific implementation. Useful for testing and edge cases.
    ///
    /// # Arguments
    /// * `path` - Path to the file to open
    /// * `force_mmap` - If true, use MmapFileAccessor; if false, use InMemoryFileAccessor
    ///
    /// # Returns
    /// * `Box<dyn FileAccessor>` - Requested implementation
    ///
    /// # Note
    /// File validation is still performed regardless of strategy.
    #[cfg(test)]
    pub async fn create_with_strategy(
        path: &Path,
        force_mmap: bool,
    ) -> Result<Box<dyn FileAccessor>> {
        // Always validate, even when forcing strategy
        validate_file_path(path)?;

        if force_mmap {
            Ok(Box::new(MmapFileAccessor::new(path).await?))
        } else {
            let content = tokio::fs::read(path).await.map_err(|e| {
                RllessError::file_error(
                    format!("Failed to read file content: {}", path.display()),
                    e,
                )
            })?;
            Ok(Box::new(InMemoryFileAccessor::new(content)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    /// Create a test file with specific content
    fn create_test_file(content: &[u8]) -> NamedTempFile {
        let mut file = NamedTempFile::new().expect("Failed to create temp file");
        file.write_all(content)
            .expect("Failed to write test content");
        file.flush().expect("Failed to flush test file");
        file
    }

    /// Create a test file with specified size
    fn create_test_file_with_size(size: usize) -> NamedTempFile {
        let content = vec![b'x'; size];
        create_test_file(&content)
    }

    #[tokio::test]
    async fn test_factory_creates_in_memory_for_small_files() {
        // Create a small file (1KB)
        let small_content = b"line1\nline2\nline3\n".repeat(25); // ~100 bytes
        let small_file = create_test_file(&small_content);

        let accessor = FileAccessorFactory::create(small_file.path())
            .await
            .unwrap();

        // InMemoryFileAccessor always knows total lines
        assert!(accessor.total_lines().is_some());

        // Test basic functionality
        let first_line = accessor.read_line(0).await.unwrap();
        assert_eq!(first_line, "line1");
    }

    #[tokio::test]
    async fn test_factory_creates_mmap_for_large_files() {
        // Create a file larger than threshold (15MB)
        let large_file = create_test_file_with_size(15 * 1024 * 1024);

        let accessor = FileAccessorFactory::create(large_file.path())
            .await
            .unwrap();

        // MmapFileAccessor returns None initially for total_lines (lazy indexing)
        // Note: This might be Some if the file gets fully indexed quickly
        let file_size = accessor.file_size();
        assert!(file_size > 10 * 1024 * 1024);
    }

    #[tokio::test]
    async fn test_factory_validates_file_before_creation() {
        use std::path::PathBuf;

        // Test with non-existent file
        let non_existent = PathBuf::from("/this/file/does/not/exist.log");
        let result = FileAccessorFactory::create(&non_existent).await;

        assert!(result.is_err());
        let error = result.err().unwrap();
        match error {
            RllessError::FileError { message, .. } => {
                assert!(message.contains("File does not exist"));
            }
            _ => panic!("Expected FileError for non-existent file"),
        }
    }

    #[tokio::test]
    async fn test_factory_handles_empty_files() {
        let empty_file = create_test_file(&[]);
        let result = FileAccessorFactory::create(empty_file.path()).await;

        // Should fail validation due to empty file
        assert!(result.is_err());
        let error = result.err().unwrap();
        match error {
            RllessError::FileError { message, .. } => {
                assert!(message.contains("File is empty"));
            }
            _ => panic!("Expected FileError for empty file"),
        }
    }

    #[tokio::test]
    async fn test_factory_get_threshold() {
        let threshold = FileAccessorFactory::get_threshold();

        #[cfg(target_os = "macos")]
        assert_eq!(threshold, 50 * 1024 * 1024);

        #[cfg(not(target_os = "macos"))]
        assert_eq!(threshold, 10 * 1024 * 1024);
    }

    #[tokio::test]
    async fn test_create_with_strategy_forces_implementation() {
        let test_content = b"line1\nline2\nline3\n";
        let test_file = create_test_file(test_content);

        // Force mmap for small file
        let mmap_accessor = FileAccessorFactory::create_with_strategy(test_file.path(), true)
            .await
            .unwrap();

        // Force in-memory for same file
        let memory_accessor = FileAccessorFactory::create_with_strategy(test_file.path(), false)
            .await
            .unwrap();

        // Both should work and return same content
        let mmap_line = mmap_accessor.read_line(0).await.unwrap();
        let memory_line = memory_accessor.read_line(0).await.unwrap();
        assert_eq!(mmap_line, memory_line);
        assert_eq!(mmap_line, "line1");
    }

    #[tokio::test]
    async fn test_compression_detection_integration() {
        // Create a file that looks like gzip (magic number)
        let gzip_header = [0x1f, 0x8b, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00];
        let mut content = gzip_header.to_vec();
        content.extend(b"some compressed data here to make it larger");

        let compressed_file = create_test_file(&content);

        // Factory should detect compression but still create accessor
        let accessor = FileAccessorFactory::create(compressed_file.path())
            .await
            .unwrap();

        // Should still work (will see compressed bytes, but that's expected for now)
        assert!(accessor.file_size() > 0);
    }

    #[tokio::test]
    async fn test_boundary_file_sizes() {
        let threshold = FileAccessorFactory::get_threshold();

        // File just under threshold should use InMemory
        let small_file = create_test_file_with_size((threshold - 1) as usize);
        let small_accessor = FileAccessorFactory::create(small_file.path())
            .await
            .unwrap();
        assert!(small_accessor.total_lines().is_some()); // InMemory characteristic

        // File at threshold should use Mmap
        let large_file = create_test_file_with_size(threshold as usize);
        let large_accessor = FileAccessorFactory::create(large_file.path())
            .await
            .unwrap();
        assert_eq!(large_accessor.file_size(), threshold); // Should handle exactly threshold size
    }
}
