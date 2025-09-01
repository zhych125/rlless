//! Error types and handling infrastructure for rlless.
//!
//! This module provides a centralized error handling system using `thiserror` for
//! custom error types and `anyhow` for application-level error handling with context.
//!
//! ## Design Principles
//!
//! - **User-friendly messages**: Errors should provide actionable feedback
//! - **Context preservation**: Include relevant information for debugging  
//! - **Extensibility**: Easy to add new error variants as features grow
//! - **Consistency**: Standardized Result type across all modules

use std::path::PathBuf;
use thiserror::Error;

/// The main error type for rlless operations.
///
/// This enum covers all possible error conditions that can occur during
/// file handling, search operations, and UI interactions.
#[derive(Error, Debug)]
pub enum RllessError {
    /// File system related errors (file not found, permission denied, etc.)
    #[error("File operation failed: {message}")]
    FileError {
        message: String,
        #[source]
        source: std::io::Error,
    },

    /// File not found specifically (common case for user feedback)
    #[error("File not found: {path}")]
    FileNotFound { path: PathBuf },

    /// Path exists but is not a regular file
    #[error("Path is not a regular file: {path}")]
    NotAFile { path: PathBuf },

    /// Permission denied accessing file
    #[error("Permission denied accessing file: {path}")]
    PermissionDenied { path: PathBuf },

    /// Memory mapping related errors
    #[error("Memory mapping failed: {message}")]
    MemoryMappingError { message: String },

    /// Compression format detection or decompression errors
    #[error("Compression error: {message}")]
    CompressionError { message: String },

    /// Search operation errors
    #[error("Search operation failed: {message}")]
    SearchError { message: String },

    /// UI and terminal related errors
    #[error("UI operation failed: {message}")]
    UIError { message: String },

    /// Configuration related errors (for Phase 4)
    #[error("Configuration error: {message}")]
    ConfigError { message: String },

    /// Invalid command line arguments
    #[error("Invalid argument: {message}")]
    InvalidArgument { message: String },

    /// Generic error for cases not covered by specific variants
    #[error("Operation failed: {message}")]
    Other { message: String },
}

/// Standard Result type for rlless operations.
///
/// This type alias provides a consistent error handling interface across
/// all modules in the rlless codebase.
pub type Result<T> = std::result::Result<T, RllessError>;

impl RllessError {
    /// Create a FileError from an io::Error with additional context
    pub fn file_error(message: impl Into<String>, source: std::io::Error) -> Self {
        Self::FileError {
            message: message.into(),
            source,
        }
    }

    /// Create a MemoryMappingError with a descriptive message
    pub fn memory_mapping(message: impl Into<String>) -> Self {
        Self::MemoryMappingError {
            message: message.into(),
        }
    }

    /// Create a CompressionError with a descriptive message
    pub fn compression(message: impl Into<String>) -> Self {
        Self::CompressionError {
            message: message.into(),
        }
    }

    /// Create a SearchError with a descriptive message
    pub fn search(message: impl Into<String>) -> Self {
        Self::SearchError {
            message: message.into(),
        }
    }

    /// Create a UIError with a descriptive message
    pub fn ui(message: impl Into<String>) -> Self {
        Self::UIError {
            message: message.into(),
        }
    }

    /// Create a generic Other error with a descriptive message
    pub fn other(message: impl Into<String>) -> Self {
        Self::Other {
            message: message.into(),
        }
    }
}

// Automatic conversion from io::Error to RllessError
impl From<std::io::Error> for RllessError {
    fn from(err: std::io::Error) -> Self {
        match err.kind() {
            std::io::ErrorKind::NotFound => {
                // For NotFound, we lose the specific path context here,
                // but it can be added at the call site using file_not_found()
                Self::FileError {
                    message: "File not found".to_string(),
                    source: err,
                }
            }
            std::io::ErrorKind::PermissionDenied => Self::FileError {
                message: "Permission denied".to_string(),
                source: err,
            },
            _ => Self::FileError {
                message: "IO operation failed".to_string(),
                source: err,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_error_display_messages() {
        let path = PathBuf::from("/test/file.log");

        let file_not_found = RllessError::FileNotFound { path: path.clone() };
        assert_eq!(file_not_found.to_string(), "File not found: /test/file.log");

        let not_a_file = RllessError::NotAFile { path: path.clone() };
        assert_eq!(
            not_a_file.to_string(),
            "Path is not a regular file: /test/file.log"
        );

        let memory_error = RllessError::memory_mapping("Failed to map file");
        assert_eq!(
            memory_error.to_string(),
            "Memory mapping failed: Failed to map file"
        );
    }

    #[test]
    fn test_error_constructors() {
        let search_err = RllessError::search("Pattern not found");
        matches!(search_err, RllessError::SearchError { .. });

        let ui_err = RllessError::ui("Terminal resize failed");
        matches!(ui_err, RllessError::UIError { .. });

        let other_err = RllessError::other("Unknown error");
        matches!(other_err, RllessError::Other { .. });
    }

    #[test]
    fn test_io_error_conversion() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "File not found");
        let rlless_err: RllessError = io_err.into();

        match rlless_err {
            RllessError::FileError { message, .. } => {
                assert_eq!(message, "File not found");
            }
            _ => panic!("Expected FileError variant"),
        }
    }

    #[test]
    fn test_result_type_alias() {
        fn returns_result() -> Result<String> {
            Ok("success".to_string())
        }

        let result = returns_result();
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "success");
    }
}
