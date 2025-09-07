//! Factory for creating FileAccessor instances.
//!
//! This module provides the FileAccessorFactory which creates AdaptiveFileAccessor instances
//! that automatically handle file size, compression detection, and platform optimization.

use crate::error::{Result, RllessError};
use crate::file_handler::adaptive::{AdaptiveFileAccessor, ByteSource};
use crate::file_handler::compression::{decompress_file, detect_compression, DecompressionResult};
use crate::file_handler::validation::validate_file_path;
use memmap2::Mmap;
use std::fs::File;
use std::io::Read;
use std::path::Path;

/// Factory for creating AdaptiveFileAccessor instances
///
/// This factory automatically selects the optimal internal strategy for AdaptiveFileAccessor
/// based on file characteristics. It handles validation, compression detection, and
/// strategy selection to provide the best performance for each file.
///
/// # Strategy Selection
/// - Files < 50MB: In-memory (`ByteSource::InMemory`)
/// - Files ≥ 50MB: Memory mapping (`ByteSource::MemoryMapped`)
/// - Compressed files: Automatic decompression with size-based strategy
///
/// # Validation
/// All files undergo validation before accessor creation:
/// - File existence and readability
/// - Reasonable file size (not empty, not >100GB)
/// - Proper file type (not directory)
pub struct FileAccessorFactory;

impl FileAccessorFactory {
    /// Size threshold for choosing between in-memory and memory-mapped strategies
    ///
    /// Files smaller than this threshold are loaded into memory (`ByteSource::InMemory`).
    /// Files larger than this threshold use memory mapping (`ByteSource::MemoryMapped`).
    const MEMORY_THRESHOLD: u64 = 50 * 1024 * 1024; // 50MB

    /// Create an AdaptiveFileAccessor with the optimal strategy for the given file
    ///
    /// # Arguments
    /// * `path` - Path to the file to open
    ///
    /// # Returns
    /// * `AdaptiveFileAccessor` - Configured with the appropriate `ByteSource` strategy
    ///
    /// # Process
    /// 1. Validate file (existence, permissions, reasonable size)
    /// 2. Detect and handle compression transparently
    /// 3. Select `ByteSource` strategy based on file size
    ///
    /// # Errors
    /// * File validation errors (non-existent, empty, too large, not readable)
    /// * Compression detection/decompression errors
    /// * Memory mapping failures
    pub async fn create(path: &Path) -> Result<AdaptiveFileAccessor> {
        // 1. Validate file first (existence, permissions, reasonable size)
        validate_file_path(path)?;

        // 2. Detect compression format
        let compression_type = detect_compression(path).await?;

        if compression_type.is_compressed() {
            // Handle compressed files
            match decompress_file(path, compression_type).await? {
                DecompressionResult::InMemory(data) => {
                    let file_size = data.len() as u64;
                    let source = ByteSource::InMemory(data);
                    Ok(AdaptiveFileAccessor::new(
                        source,
                        file_size,
                        path.to_path_buf(),
                    ))
                }
                DecompressionResult::TempFile(temp_file) => {
                    // Memory map the temp file
                    let temp_file_handle = temp_file
                        .reopen()
                        .map_err(|e| RllessError::file_error("Failed to reopen temp file", e))?;

                    let mmap = unsafe {
                        Mmap::map(&temp_file_handle).map_err(|e| {
                            RllessError::file_error("Failed to memory map temp file", e)
                        })?
                    };

                    let file_size = mmap.len() as u64;
                    let source = ByteSource::Compressed {
                        mmap,
                        _temp_file: temp_file,
                    };
                    Ok(AdaptiveFileAccessor::new(
                        source,
                        file_size,
                        path.to_path_buf(),
                    ))
                }
            }
        } else {
            // Handle uncompressed files - use size-based strategy
            let file = File::open(path).map_err(|e| {
                RllessError::file_error(format!("Failed to open file: {}", path.display()), e)
            })?;

            let metadata = file
                .metadata()
                .map_err(|e| RllessError::file_error("Failed to get file metadata", e))?;
            let file_size = metadata.len();

            if file_size < Self::MEMORY_THRESHOLD {
                // Small file: load into memory
                let mut content = Vec::new();
                let mut file = file;
                file.read_to_end(&mut content)
                    .map_err(|e| RllessError::file_error("Failed to read file", e))?;

                let source = ByteSource::InMemory(content);
                Ok(AdaptiveFileAccessor::new(
                    source,
                    file_size,
                    path.to_path_buf(),
                ))
            } else {
                // Large file: use memory mapping
                let mmap = unsafe {
                    Mmap::map(&file).map_err(|e| {
                        RllessError::file_error(
                            format!("Failed to memory map file: {}", path.display()),
                            e,
                        )
                    })?
                };

                let source = ByteSource::MemoryMapped(mmap);
                Ok(AdaptiveFileAccessor::new(
                    source,
                    file_size,
                    path.to_path_buf(),
                ))
            }
        }
    }

    /// Create AdaptiveFileAccessor with explicit strategy (for testing)
    ///
    /// Bypasses automatic strategy selection and forces a specific `ByteSource`.
    /// Useful for testing different strategies on the same file.
    ///
    /// # Arguments
    /// * `path` - Path to the file to open
    /// * `force_mmap` - If true, use `ByteSource::MemoryMapped`; if false, use `ByteSource::InMemory`
    ///
    /// # Returns
    /// * `AdaptiveFileAccessor` - Configured with the requested strategy
    #[cfg(test)]
    pub async fn create_with_strategy(
        path: &Path,
        force_mmap: bool,
    ) -> Result<AdaptiveFileAccessor> {
        // Always validate, even when forcing strategy
        validate_file_path(path)?;

        let file = File::open(path).map_err(|e| {
            RllessError::file_error(format!("Failed to open file: {}", path.display()), e)
        })?;

        let metadata = file
            .metadata()
            .map_err(|e| RllessError::file_error("Failed to get file metadata", e))?;
        let file_size = metadata.len();

        if force_mmap {
            // Force memory mapping
            let mmap = unsafe {
                Mmap::map(&file).map_err(|e| {
                    RllessError::file_error(
                        format!("Failed to memory map file: {}", path.display()),
                        e,
                    )
                })?
            };

            let source = ByteSource::MemoryMapped(mmap);
            Ok(AdaptiveFileAccessor::new(
                source,
                file_size,
                path.to_path_buf(),
            ))
        } else {
            // Force in-memory
            let mut content = Vec::new();
            let mut file = file;
            file.read_to_end(&mut content)
                .map_err(|e| RllessError::file_error("Failed to read file", e))?;

            let source = ByteSource::InMemory(content);
            Ok(AdaptiveFileAccessor::new(
                source,
                file_size,
                path.to_path_buf(),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file_handler::accessor::FileAccessor;
    use crate::file_handler::adaptive::ByteSource;
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use std::io::Write;
    use std::path::PathBuf;
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

        // Test basic functionality
        let lines = accessor.read_from_byte(0, 1).await.unwrap();
        assert_eq!(lines[0], "line1");

        // Verify it's using InMemory strategy
        match &accessor.source {
            ByteSource::InMemory(_) => {} // Expected
            _ => panic!("Small file should use InMemory variant"),
        }
    }

    #[tokio::test]
    async fn test_factory_creates_mmap_for_large_files() {
        // Create a file larger than threshold (60MB)
        let large_file = create_test_file_with_size(60 * 1024 * 1024);

        let accessor = FileAccessorFactory::create(large_file.path())
            .await
            .unwrap();

        // Verify it's using MemoryMapped strategy for large files
        match &accessor.source {
            ByteSource::MemoryMapped(_) => {} // Expected
            _ => panic!("Large file should use MemoryMapped variant"),
        }

        let file_size = accessor.file_size();
        assert_eq!(file_size, 60 * 1024 * 1024);
    }

    #[tokio::test]
    async fn test_factory_validates_file_before_creation() {
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

    #[test]
    fn test_factory_memory_threshold() {
        // Test that the threshold constant is as expected
        assert_eq!(FileAccessorFactory::MEMORY_THRESHOLD, 50 * 1024 * 1024);
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

        // Verify forced strategies
        match &mmap_accessor.source {
            ByteSource::MemoryMapped(_) => {} // Expected
            _ => panic!("Should be forced to MemoryMapped"),
        }

        match &memory_accessor.source {
            ByteSource::InMemory(_) => {} // Expected
            _ => panic!("Should be forced to InMemory"),
        }

        // Both should work and return same content
        let mmap_lines = mmap_accessor.read_from_byte(0, 1).await.unwrap();
        let memory_lines = memory_accessor.read_from_byte(0, 1).await.unwrap();
        assert_eq!(mmap_lines[0], memory_lines[0]);
        assert_eq!(mmap_lines[0], "line1");
    }

    #[tokio::test]
    async fn test_compression_detection_integration() {
        // Create actual compressed data
        let original_text = "line 1\nline 2\nline 3\nThis is a test file with multiple lines\n";

        // Create a real gzip compressed file
        let temp_file = NamedTempFile::new().unwrap();
        {
            let file = std::fs::File::create(temp_file.path()).unwrap();
            let mut encoder = GzEncoder::new(file, Compression::default());
            encoder.write_all(original_text.as_bytes()).unwrap();
            encoder.finish().unwrap();
        }

        // Factory should detect compression and decompress transparently
        let accessor = FileAccessorFactory::create(temp_file.path()).await.unwrap();

        // Should be able to read the decompressed content
        let lines = accessor.read_from_byte(0, 2).await.unwrap();
        assert_eq!(lines[0], "line 1");
        assert_eq!(lines[1], "line 2");

        // File size should be the uncompressed size
        assert!(accessor.file_size() > 0);
    }

    #[tokio::test]
    async fn test_boundary_file_sizes() {
        let threshold = FileAccessorFactory::MEMORY_THRESHOLD;

        // File just under threshold should use InMemory
        let small_file = create_test_file_with_size((threshold - 1) as usize);
        let small_accessor = FileAccessorFactory::create(small_file.path())
            .await
            .unwrap();
        match &small_accessor.source {
            ByteSource::InMemory(_) => {} // Expected
            _ => panic!("Small file should use InMemory variant"),
        }

        // File at threshold should use Mmap
        let large_file = create_test_file_with_size(threshold as usize);
        let large_accessor = FileAccessorFactory::create(large_file.path())
            .await
            .unwrap();
        match &large_accessor.source {
            ByteSource::MemoryMapped(_) => {} // Expected
            _ => panic!("Large file should use MemoryMapped variant"),
        }
        assert_eq!(large_accessor.file_size(), threshold);
    }
}
