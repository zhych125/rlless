//! Compression format detection for transparent file access.
//!
//! This module provides compression format detection using magic numbers (file signatures).
//! It supports the most common compression formats used with log files.

use crate::error::{Result, RllessError};
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

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
}

/// Detect compression type from file path and magic numbers
///
/// This function reads the first few bytes of a file to identify compression format
/// based on magic numbers (file signatures). This approach is reliable regardless
/// of file extensions and works with renamed or extension-less files.
///
/// # Magic Numbers Used
/// - Gzip: `1f 8b` (RFC 1952)
/// - Bzip2: `42 5a 68` ("BZh" with block size)
/// - XZ: `fd 37 7a 58 5a 00` (XZ format specification)
///
/// # Performance
/// Only reads the first 8 bytes of the file, making it efficient even for very large files.
pub fn detect_compression(path: &Path) -> Result<CompressionType> {
    let file = File::open(path)
        .map_err(|e| RllessError::file_error("Failed to open file for compression detection", e))?;
    
    let mut reader = BufReader::new(file);
    let mut buffer = [0u8; 8]; // Enough for longest magic number (XZ needs 6 bytes)
    
    let bytes_read = reader.read(&mut buffer)
        .map_err(|e| RllessError::file_error("Failed to read file header", e))?;
    
    // Handle empty or very small files
    if bytes_read < 2 {
        return Ok(CompressionType::None);
    }
    
    // Check magic numbers in order of common usage and performance
    if buffer.starts_with(&[0x1f, 0x8b]) {
        // Gzip magic number (RFC 1952)
        Ok(CompressionType::Gzip)
    } else if bytes_read >= 3 && buffer.starts_with(&[0x42, 0x5a, 0x68]) {
        // Bzip2 magic number "BZh" (most common block size)
        Ok(CompressionType::Bzip2)
    } else if bytes_read >= 6 && buffer.starts_with(&[0xfd, 0x37, 0x7a, 0x58, 0x5a, 0x00]) {
        // XZ magic number (XZ file format specification)
        Ok(CompressionType::Xz)
    } else {
        // No compression detected
        Ok(CompressionType::None)
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
        file.write_all(content).expect("Failed to write test content");
        file.flush().expect("Failed to flush test file");
        file
    }

    #[test]
    fn test_detect_gzip_compression() {
        // Gzip magic number: 1f 8b
        let gzip_header = [0x1f, 0x8b, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00];
        let test_file = create_test_file(&gzip_header);
        
        let result = detect_compression(test_file.path()).unwrap();
        assert_eq!(result, CompressionType::Gzip);
    }

    #[test]
    fn test_detect_bzip2_compression() {
        // Bzip2 magic number: "BZh" (42 5a 68)
        let bzip2_header = [0x42, 0x5a, 0x68, 0x39, 0x31, 0x41, 0x59, 0x26];
        let test_file = create_test_file(&bzip2_header);
        
        let result = detect_compression(test_file.path()).unwrap();
        assert_eq!(result, CompressionType::Bzip2);
    }

    #[test]
    fn test_detect_xz_compression() {
        // XZ magic number: fd 37 7a 58 5a 00
        let xz_header = [0xfd, 0x37, 0x7a, 0x58, 0x5a, 0x00, 0x00, 0x01];
        let test_file = create_test_file(&xz_header);
        
        let result = detect_compression(test_file.path()).unwrap();
        assert_eq!(result, CompressionType::Xz);
    }

    #[test]
    fn test_detect_no_compression() {
        // Plain text file
        let plain_content = b"This is just plain text content\nwith multiple lines\n";
        let test_file = create_test_file(plain_content);
        
        let result = detect_compression(test_file.path()).unwrap();
        assert_eq!(result, CompressionType::None);
    }

    #[test]
    fn test_detect_empty_file() {
        // Empty file should be detected as no compression
        let test_file = create_test_file(&[]);
        
        let result = detect_compression(test_file.path()).unwrap();
        assert_eq!(result, CompressionType::None);
    }

    #[test]
    fn test_detect_single_byte_file() {
        // Single byte file should be detected as no compression
        let test_file = create_test_file(&[0x42]);
        
        let result = detect_compression(test_file.path()).unwrap();
        assert_eq!(result, CompressionType::None);
    }

    #[test]
    fn test_detect_partial_magic_number() {
        // File that starts with partial gzip magic (only first byte)
        let partial_header = [0x1f, 0x00, 0x00, 0x00];
        let test_file = create_test_file(&partial_header);
        
        let result = detect_compression(test_file.path()).unwrap();
        assert_eq!(result, CompressionType::None);
    }

    #[test]
    fn test_detect_compression_file_not_found() {
        use std::path::PathBuf;
        
        let non_existent_path = PathBuf::from("/this/file/does/not/exist");
        let result = detect_compression(&non_existent_path);
        
        assert!(result.is_err());
        // Should be a file error
        match result.unwrap_err() {
            RllessError::FileError { .. } => {}, // Expected
            _ => panic!("Expected FileError for non-existent file"),
        }
    }

    #[test]
    fn test_compression_type_debug_display() {
        // Test that our enum derives work correctly
        assert_eq!(format!("{:?}", CompressionType::None), "None");
        assert_eq!(format!("{:?}", CompressionType::Gzip), "Gzip");
        assert_eq!(format!("{:?}", CompressionType::Bzip2), "Bzip2");
        assert_eq!(format!("{:?}", CompressionType::Xz), "Xz");
    }

    #[test]
    fn test_compression_type_equality() {
        // Test that equality works
        assert_eq!(CompressionType::Gzip, CompressionType::Gzip);
        assert_ne!(CompressionType::Gzip, CompressionType::Bzip2);
        
        // Test that copy works
        let compression = CompressionType::Xz;
        let copied = compression;
        assert_eq!(compression, copied);
    }
}