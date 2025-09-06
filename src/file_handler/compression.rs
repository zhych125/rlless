//! Compression format detection and transparent decompression for file access.
//!
//! This module provides compression format detection using magic numbers (file signatures)
//! and transparent decompression support for common compression formats used with log files.

use crate::error::{Result, RllessError};
use crate::file_handler::{FileAccessor, InMemoryFileAccessor, MmapFileAccessor};
use async_compression::tokio::bufread::{BzDecoder, GzipDecoder, XzDecoder, ZstdDecoder};
use async_trait::async_trait;
use std::borrow::Cow;
use std::path::{Path, PathBuf};
use tempfile::NamedTempFile;
use tokio::fs::File;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader, BufWriter};

/// Supported compression formats for transparent file access
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompressionType {
    /// No compression - plain text file
    None,
    /// Gzip compression (.gz files)
    Gzip,
    /// Bzip2 compression (.bz2 files)
    Bzip2,
    /// XZ compression (.xz files)
    Xz,
    /// Zstandard compression (.zst, .zstd files)
    Zstd,
}

impl CompressionType {
    /// Get human-readable name for the compression type
    pub fn name(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Gzip => "gzip",
            Self::Bzip2 => "bzip2",
            Self::Xz => "xz",
            Self::Zstd => "zstd",
        }
    }

    /// Check if this type represents a compressed format
    pub fn is_compressed(&self) -> bool {
        !matches!(self, Self::None)
    }
}

/// Detect compression type from file path and magic numbers
///
/// This function reads the first few bytes of a file to identify compression format
/// based on magic numbers (file signatures). Falls back to extension-based detection
/// if magic numbers don't match.
///
/// # Magic Numbers Used
/// - Gzip: `1f 8b` (RFC 1952)
/// - Bzip2: `42 5a 68` ("BZh" with block size)
/// - XZ: `fd 37 7a 58 5a 00` (XZ format specification)
/// - Zstd: `28 b5 2f fd` (Zstandard frame format)
pub async fn detect_compression(path: &Path) -> Result<CompressionType> {
    // Try magic bytes first (most reliable)
    if let Ok(mut file) = File::open(path).await {
        let mut buffer = [0u8; 8];
        let bytes_read = file.read(&mut buffer).await.unwrap_or(0);

        if bytes_read >= 2 {
            if let Some(format) = detect_by_magic(&buffer[..bytes_read]) {
                return Ok(format);
            }
        }
    }

    // Fall back to extension-based detection
    if let Some(format) = detect_by_extension(path) {
        return Ok(format);
    }

    // No compression detected
    Ok(CompressionType::None)
}

/// Detect compression format from magic bytes
fn detect_by_magic(magic: &[u8]) -> Option<CompressionType> {
    if magic.len() < 2 {
        return None;
    }

    // Check magic numbers in order of common usage
    if magic.starts_with(&[0x1f, 0x8b]) {
        // Gzip magic number (RFC 1952)
        Some(CompressionType::Gzip)
    } else if magic.len() >= 3 && magic.starts_with(&[0x42, 0x5a, 0x68]) {
        // Bzip2 magic number "BZh"
        Some(CompressionType::Bzip2)
    } else if magic.len() >= 4 && magic.starts_with(&[0x28, 0xb5, 0x2f, 0xfd]) {
        // Zstd magic number
        Some(CompressionType::Zstd)
    } else if magic.len() >= 6 && magic.starts_with(&[0xfd, 0x37, 0x7a, 0x58, 0x5a, 0x00]) {
        // XZ magic number
        Some(CompressionType::Xz)
    } else {
        None
    }
}

/// Detect compression format from file extension
fn detect_by_extension(path: &Path) -> Option<CompressionType> {
    let ext = path.extension()?.to_str()?;
    match ext.to_lowercase().as_str() {
        "gz" => Some(CompressionType::Gzip),
        "bz2" => Some(CompressionType::Bzip2),
        "xz" => Some(CompressionType::Xz),
        "zst" | "zstd" => Some(CompressionType::Zstd),
        _ => None,
    }
}

/// FileAccessor implementation that handles compressed files transparently
pub struct CompressedFileAccessor {
    /// Inner accessor (either InMemory or Mmap)
    inner_accessor: Box<dyn FileAccessor>,
    /// Compression format of the original file
    compression_type: CompressionType,
    /// Path to the original compressed file
    original_path: PathBuf,
    /// Temporary file handle (kept alive for mmap)
    _temp_file: Option<NamedTempFile>,
    /// Size of the compressed file
    compressed_size: u64,
}

impl CompressedFileAccessor {
    /// Create a new CompressedFileAccessor
    ///
    /// For small compressed files (<10MB), decompresses to memory.
    /// For large files, decompresses to a temp file and mmaps it.
    pub async fn new(path: &Path, compression: CompressionType) -> Result<Self> {
        if compression == CompressionType::None {
            return Err(RllessError::file_error(
                "CompressedFileAccessor called with no compression",
                std::io::Error::new(std::io::ErrorKind::InvalidInput, "No compression type"),
            ));
        }

        let metadata = tokio::fs::metadata(path)
            .await
            .map_err(|e| RllessError::file_error("Failed to read compressed file metadata", e))?;
        let compressed_size = metadata.len();

        // Threshold for in-memory vs temp file decompression
        const MEMORY_THRESHOLD: u64 = 10_000_000; // 10MB

        let (inner_accessor, temp_file): (Box<dyn FileAccessor>, Option<NamedTempFile>) =
            if compressed_size < MEMORY_THRESHOLD {
                // Small file: decompress to memory
                let data = decompress_to_memory(path, compression).await?;
                let accessor = InMemoryFileAccessor::new(data, path.to_path_buf());
                (Box::new(accessor), None)
            } else {
                // Large file: decompress to temp file and mmap
                let temp_file = decompress_to_temp_file(path, compression).await?;
                let temp_path = temp_file.path();
                let accessor = MmapFileAccessor::new(temp_path).await?;
                (Box::new(accessor), Some(temp_file))
            };

        Ok(Self {
            inner_accessor,
            compression_type: compression,
            original_path: path.to_path_buf(),
            _temp_file: temp_file,
            compressed_size,
        })
    }

    /// Get the compression type
    pub fn compression_type(&self) -> CompressionType {
        self.compression_type
    }

    /// Get the compressed file size
    pub fn compressed_size(&self) -> u64 {
        self.compressed_size
    }

    /// Get the path to the original compressed file
    pub fn original_path(&self) -> &Path {
        &self.original_path
    }
}

#[async_trait]
impl FileAccessor for CompressedFileAccessor {
    async fn read_line(&self, line_number: u64) -> Result<Cow<'_, str>> {
        self.inner_accessor.read_line(line_number).await
    }

    async fn read_lines_range(&self, start: u64, count: u64) -> Result<Vec<Cow<'_, str>>> {
        self.inner_accessor.read_lines_range(start, count).await
    }

    async fn find_next_match(
        &self,
        start_line: u64,
        search_fn: &(dyn for<'a> Fn(&'a str) -> Vec<(usize, usize)> + Send + Sync),
    ) -> Result<Option<u64>> {
        self.inner_accessor
            .find_next_match(start_line, search_fn)
            .await
    }

    async fn find_prev_match(
        &self,
        start_line: u64,
        search_fn: &(dyn for<'a> Fn(&'a str) -> Vec<(usize, usize)> + Send + Sync),
    ) -> Result<Option<u64>> {
        self.inner_accessor
            .find_prev_match(start_line, search_fn)
            .await
    }

    fn file_size(&self) -> u64 {
        self.inner_accessor.file_size()
    }

    fn total_lines(&self) -> Option<u64> {
        self.inner_accessor.total_lines()
    }

    fn file_path(&self) -> &std::path::Path {
        &self.original_path
    }

    fn supports_parallel(&self) -> bool {
        self.inner_accessor.supports_parallel()
    }
}

/// Decompress a file entirely into memory
async fn decompress_to_memory(path: &Path, compression: CompressionType) -> Result<Vec<u8>> {
    let file = File::open(path)
        .await
        .map_err(|e| RllessError::file_error("Failed to open compressed file", e))?;
    let file = BufReader::new(file);

    let mut data = Vec::new();
    let mut decoder: Box<dyn AsyncRead + Unpin> = match compression {
        CompressionType::Gzip => Box::new(GzipDecoder::new(file)),
        CompressionType::Bzip2 => Box::new(BzDecoder::new(file)),
        CompressionType::Xz => Box::new(XzDecoder::new(file)),
        CompressionType::Zstd => Box::new(ZstdDecoder::new(file)),
        CompressionType::None => unreachable!("Should not decompress uncompressed files"),
    };

    decoder
        .read_to_end(&mut data)
        .await
        .map_err(|e| RllessError::file_error("Failed to decompress file", e))?;

    Ok(data)
}

/// Decompress a file to a temporary file
async fn decompress_to_temp_file(
    path: &Path,
    compression: CompressionType,
) -> Result<NamedTempFile> {
    let file = File::open(path)
        .await
        .map_err(|e| RllessError::file_error("Failed to open compressed file", e))?;
    let file = BufReader::new(file);

    // Create temp file
    let temp_file = NamedTempFile::new()
        .map_err(|e| RllessError::file_error("Failed to create temp file", e))?;
    let temp_path = temp_file.path().to_path_buf();

    // Open temp file for writing with buffering for better performance
    let temp_file_handle = tokio::fs::File::create(&temp_path)
        .await
        .map_err(|e| RllessError::file_error("Failed to open temp file for writing", e))?;
    let mut temp_writer = BufWriter::new(temp_file_handle);

    // Create decoder
    let mut decoder: Box<dyn AsyncRead + Unpin> = match compression {
        CompressionType::Gzip => Box::new(GzipDecoder::new(file)),
        CompressionType::Bzip2 => Box::new(BzDecoder::new(file)),
        CompressionType::Xz => Box::new(XzDecoder::new(file)),
        CompressionType::Zstd => Box::new(ZstdDecoder::new(file)),
        CompressionType::None => unreachable!("Should not decompress uncompressed files"),
    };

    // Use optimized copy operation instead of manual buffering
    // This uses tokio's internal optimizations and larger buffers
    tokio::io::copy(&mut decoder, &mut temp_writer)
        .await
        .map_err(|e| RllessError::file_error("Failed to decompress file", e))?;

    // Ensure all data is written to disk
    temp_writer
        .flush()
        .await
        .map_err(|e| RllessError::file_error("Failed to flush temp file", e))?;

    Ok(temp_file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_detect_gzip_magic() {
        let magic = [0x1f, 0x8b, 0x08, 0x00];
        assert_eq!(detect_by_magic(&magic), Some(CompressionType::Gzip));
    }

    #[test]
    fn test_detect_bzip2_magic() {
        let magic = [0x42, 0x5a, 0x68, 0x39];
        assert_eq!(detect_by_magic(&magic), Some(CompressionType::Bzip2));
    }

    #[test]
    fn test_detect_xz_magic() {
        let magic = [0xfd, 0x37, 0x7a, 0x58, 0x5a, 0x00];
        assert_eq!(detect_by_magic(&magic), Some(CompressionType::Xz));
    }

    #[test]
    fn test_detect_zstd_magic() {
        let magic = [0x28, 0xb5, 0x2f, 0xfd];
        assert_eq!(detect_by_magic(&magic), Some(CompressionType::Zstd));
    }

    #[test]
    fn test_detect_no_compression() {
        let magic = [0x00, 0x00, 0x00, 0x00];
        assert_eq!(detect_by_magic(&magic), None);
    }

    #[test]
    fn test_detect_by_extension() {
        assert_eq!(
            detect_by_extension(Path::new("file.gz")),
            Some(CompressionType::Gzip)
        );
        assert_eq!(
            detect_by_extension(Path::new("file.bz2")),
            Some(CompressionType::Bzip2)
        );
        assert_eq!(
            detect_by_extension(Path::new("file.xz")),
            Some(CompressionType::Xz)
        );
        assert_eq!(
            detect_by_extension(Path::new("file.zst")),
            Some(CompressionType::Zstd)
        );
        assert_eq!(
            detect_by_extension(Path::new("file.zstd")),
            Some(CompressionType::Zstd)
        );
        assert_eq!(detect_by_extension(Path::new("file.txt")), None);
    }

    #[test]
    fn test_compression_type_methods() {
        assert!(!CompressionType::None.is_compressed());
        assert!(CompressionType::Gzip.is_compressed());
        assert_eq!(CompressionType::Bzip2.name(), "bzip2");
        assert_eq!(CompressionType::Zstd.name(), "zstd");
    }

    #[tokio::test]
    async fn test_detect_compression_with_gzip_file() {
        // Create a test file with gzip magic bytes
        let temp_file = tempfile::NamedTempFile::new().unwrap();
        {
            let mut file = std::fs::File::create(temp_file.path()).unwrap();
            file.write_all(&[0x1f, 0x8b, 0x08, 0x00]).unwrap();
        }

        let result = detect_compression(temp_file.path()).await.unwrap();
        assert_eq!(result, CompressionType::Gzip);
    }

    #[tokio::test]
    async fn test_detect_compression_with_extension_fallback() {
        // Create a file with .gz extension but no magic bytes
        let temp_dir = tempfile::tempdir().unwrap();
        let file_path = temp_dir.path().join("test.gz");
        tokio::fs::write(&file_path, b"not compressed")
            .await
            .unwrap();

        let result = detect_compression(&file_path).await.unwrap();
        assert_eq!(result, CompressionType::Gzip);
    }
}
