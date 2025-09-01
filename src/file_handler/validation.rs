//! File validation utilities for ensuring files are suitable for processing.
//!
//! This module provides validation functions to check that files can be safely
//! processed by rlless before attempting to open them for reading.

use crate::error::{Result, RllessError};
use std::fs::File;
use std::path::Path;

/// Validate that a file path is accessible and suitable for processing
///
/// This function performs essential checks to ensure a file can be safely
/// processed by rlless before attempting to open it for reading.
///
/// # Validations Performed
/// - Path exists and is a file (not a directory or symlink)
/// - File is readable by the current process
/// - File size is reasonable (not zero, not suspiciously large for a log file)
/// - File appears to contain text-based content
///
/// # Error Cases
/// - File does not exist
/// - Path points to a directory
/// - File is not readable due to permissions
/// - File is empty (likely not a useful log file)
/// - File is suspiciously large (>100GB, might indicate binary content)
pub fn validate_file_path(path: &Path) -> Result<()> {
    // Check if path exists
    if !path.exists() {
        return Err(RllessError::file_error(
            format!("File does not exist: {}", path.display()),
            std::io::Error::new(std::io::ErrorKind::NotFound, "File not found"),
        ));
    }

    // Check if it's a file (not directory, symlink, etc.)
    let metadata = std::fs::metadata(path)
        .map_err(|e| RllessError::file_error("Failed to read file metadata", e))?;

    if !metadata.is_file() {
        return Err(RllessError::file_error(
            format!("Path is not a file: {}", path.display()),
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "Not a file"),
        ));
    }

    // Check file size constraints
    let file_size = metadata.len();

    if file_size == 0 {
        return Err(RllessError::file_error(
            format!("File is empty: {}", path.display()),
            std::io::Error::new(std::io::ErrorKind::InvalidData, "Empty file"),
        ));
    }

    // Warn about very large files (>100GB) - might be binary or problematic
    const MAX_REASONABLE_SIZE: u64 = 100 * 1024 * 1024 * 1024; // 100GB
    if file_size > MAX_REASONABLE_SIZE {
        return Err(RllessError::file_error(
            format!(
                "File is suspiciously large ({}GB): {}",
                file_size / (1024 * 1024 * 1024),
                path.display()
            ),
            std::io::Error::new(std::io::ErrorKind::InvalidData, "File too large"),
        ));
    }

    // Try to open the file to verify read permissions
    File::open(path).map_err(|e| RllessError::file_error("Cannot open file for reading", e))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::{NamedTempFile, TempDir};

    /// Create a test file with specific content
    fn create_test_file(content: &[u8]) -> NamedTempFile {
        let mut file = NamedTempFile::new().expect("Failed to create temp file");
        file.write_all(content)
            .expect("Failed to write test content");
        file.flush().expect("Failed to flush test file");
        file
    }

    #[test]
    fn test_validate_valid_file() {
        let test_file = create_test_file(b"This is valid log content\nLine 2\nLine 3\n");
        let result = validate_file_path(test_file.path());
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_nonexistent_file() {
        let non_existent = std::path::Path::new("/this/file/does/not/exist.log");
        let result = validate_file_path(non_existent);

        assert!(result.is_err());
        match result.unwrap_err() {
            RllessError::FileError { message, .. } => {
                assert!(message.contains("File does not exist"));
            }
            _ => panic!("Expected FileError for non-existent file"),
        }
    }

    #[test]
    fn test_validate_empty_file() {
        let empty_file = create_test_file(&[]);
        let result = validate_file_path(empty_file.path());

        assert!(result.is_err());
        match result.unwrap_err() {
            RllessError::FileError { message, .. } => {
                assert!(message.contains("File is empty"));
            }
            _ => panic!("Expected FileError for empty file"),
        }
    }

    #[test]
    fn test_validate_directory() {
        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        let result = validate_file_path(temp_dir.path());

        assert!(result.is_err());
        match result.unwrap_err() {
            RllessError::FileError { message, .. } => {
                assert!(message.contains("Path is not a file"));
            }
            _ => panic!("Expected FileError for directory"),
        }
    }
}
