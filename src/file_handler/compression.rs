//! Compression format detection and decompression utilities.
//!
//! This module provides compression format detection using magic numbers (file signatures)
//! and decompression utilities for common compression formats used with log files.

use crate::error::{Result, RllessError};
use async_compression::tokio::bufread::{BzDecoder, GzipDecoder, XzDecoder, ZstdDecoder};
use std::path::Path;
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

/// Decompression result that can be either in-memory or a temp file
pub enum DecompressionResult {
    /// Small file decompressed to memory
    InMemory(Vec<u8>),
    /// Large file decompressed to temp file
    TempFile(NamedTempFile),
}

/// Decompress a file using the appropriate strategy based on file size
///
/// # Strategy
/// - Files < 10MB compressed: decompress to memory
/// - Files ≥ 10MB compressed: decompress to temp file
pub async fn decompress_file(
    path: &Path,
    compression: CompressionType,
) -> Result<DecompressionResult> {
    if !compression.is_compressed() {
        return Err(RllessError::file_error(
            "decompress_file called with no compression",
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "No compression type"),
        ));
    }

    // Get compressed file size
    let metadata = tokio::fs::metadata(path)
        .await
        .map_err(|e| RllessError::file_error("Failed to get file metadata", e))?;
    let compressed_size = metadata.len();

    // Threshold for in-memory vs temp file decompression
    const MEMORY_THRESHOLD: u64 = 10_000_000; // 10MB compressed size

    if compressed_size < MEMORY_THRESHOLD {
        // Small compressed file: decompress to memory
        let data = decompress_to_memory(path, compression).await?;
        Ok(DecompressionResult::InMemory(data))
    } else {
        // Large compressed file: decompress to temp file
        let temp_file = decompress_to_temp_file(path, compression).await?;
        Ok(DecompressionResult::TempFile(temp_file))
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
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use std::io::Write;
    use tokio::io::AsyncReadExt;

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

    #[tokio::test]
    async fn test_decompress_file_small_file() {
        // Create a small gzipped test file
        let test_data = b"Hello, world!\nThis is a test file.\n";
        let temp_file = tempfile::NamedTempFile::new().unwrap();
        {
            let mut encoder = GzEncoder::new(
                std::fs::File::create(temp_file.path()).unwrap(),
                Compression::default(),
            );
            encoder.write_all(test_data).unwrap();
            encoder.finish().unwrap();
        }

        let result = decompress_file(temp_file.path(), CompressionType::Gzip)
            .await
            .unwrap();

        match result {
            DecompressionResult::InMemory(data) => {
                assert_eq!(data, test_data);
            }
            DecompressionResult::TempFile(_) => {
                panic!("Small file should be decompressed to memory");
            }
        }
    }

    #[tokio::test]
    async fn test_decompress_file_with_no_compression_fails() {
        let temp_file = tempfile::NamedTempFile::new().unwrap();
        tokio::fs::write(temp_file.path(), b"not compressed")
            .await
            .unwrap();

        let result = decompress_file(temp_file.path(), CompressionType::None).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_decompress_to_memory_gzip() {
        // Create a gzipped test file
        let test_data = b"Test content for gzip decompression";
        let temp_file = tempfile::NamedTempFile::new().unwrap();
        {
            let mut encoder = GzEncoder::new(
                std::fs::File::create(temp_file.path()).unwrap(),
                Compression::default(),
            );
            encoder.write_all(test_data).unwrap();
            encoder.finish().unwrap();
        }

        let result = decompress_to_memory(temp_file.path(), CompressionType::Gzip)
            .await
            .unwrap();
        assert_eq!(result, test_data);
    }

    #[tokio::test]
    async fn test_decompress_to_temp_file() {
        // Create a gzipped test file
        let test_data = b"Test content for temp file decompression";
        let compressed_file = tempfile::NamedTempFile::new().unwrap();
        {
            let mut encoder = GzEncoder::new(
                std::fs::File::create(compressed_file.path()).unwrap(),
                Compression::default(),
            );
            encoder.write_all(test_data).unwrap();
            encoder.finish().unwrap();
        }

        let temp_file = decompress_to_temp_file(compressed_file.path(), CompressionType::Gzip)
            .await
            .unwrap();

        // Read the temp file content
        let mut decompressed_content = Vec::new();
        let mut file = tokio::fs::File::open(temp_file.path()).await.unwrap();
        file.read_to_end(&mut decompressed_content).await.unwrap();

        assert_eq!(decompressed_content, test_data);
    }

    #[test]
    fn test_decompression_result_variants() {
        let data = vec![1, 2, 3];
        let temp_file = tempfile::NamedTempFile::new().unwrap();

        match DecompressionResult::InMemory(data.clone()) {
            DecompressionResult::InMemory(d) => assert_eq!(d, data),
            DecompressionResult::TempFile(_) => panic!("Wrong variant"),
        }

        match DecompressionResult::TempFile(temp_file) {
            DecompressionResult::TempFile(_) => {} // Success
            DecompressionResult::InMemory(_) => panic!("Wrong variant"),
        }
    }
}
