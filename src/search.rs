//! Search engine module with SIMD-optimized search capabilities
//!
//! This module provides a flexible search interface built on top of the ripgrep ecosystem
//! for maximum performance. It supports regex patterns, case sensitivity, whole word matching,
//! and provides efficient result caching.

use crate::error::{Result, RllessError};
use crate::file_handler::accessor::FileAccessor;
use async_trait::async_trait;
use grep_matcher::Matcher;
use grep_regex::{RegexMatcher, RegexMatcherBuilder};
use lru::LruCache;
use parking_lot::RwLock;
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::timeout;

/// Configuration options for search operations
#[derive(Debug, Clone, PartialEq)]
pub struct SearchOptions {
    /// Enable case-sensitive search
    pub case_sensitive: bool,
    /// Match whole words only
    pub whole_word: bool,
    /// Treat pattern as regex (true) or literal string (false)
    pub regex_mode: bool,
    /// Number of context lines to include in results
    pub context_lines: u32,
    /// Maximum time to spend on a single search operation (ReDoS protection)
    pub timeout: Option<Duration>,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            case_sensitive: false,
            whole_word: false,
            regex_mode: false,
            context_lines: 0,
            timeout: Some(Duration::from_secs(10)), // 10 second default timeout
        }
    }
}

/// Core trait for search engine implementations
///
/// This trait provides a unified interface for different search backends while maintaining
/// high performance through SIMD optimization and intelligent caching.
#[async_trait]
pub trait SearchEngine: Send + Sync {
    /// Search for a pattern starting from a specific byte position
    ///
    /// # Arguments
    /// * `pattern` - Search pattern (string or regex depending on options)
    /// * `start_byte` - Byte position to start searching from (0-based, inclusive)
    /// * `options` - Search configuration options
    ///
    /// # Returns
    /// * Some(byte_position) if pattern found
    /// * None if pattern not found before EOF
    ///
    /// # Performance
    /// * Target: <500ms for 40GB files
    /// * Uses SIMD optimization when available
    /// * Leverages line-based match caching for repeated searches
    async fn search_from(
        &self,
        pattern: &str,
        start_byte: u64,
        options: &SearchOptions,
    ) -> Result<Option<u64>>;

    /// Search for the previous occurrence of a pattern
    ///
    /// # Arguments
    /// * `pattern` - Search pattern (string or regex depending on options)
    /// * `start_byte` - Byte position to start searching from (0-based, exclusive)
    /// * `options` - Search configuration options
    ///
    /// # Returns
    /// * Some(byte_position) if pattern found
    /// * None if pattern not found before beginning of file
    async fn search_prev(
        &self,
        pattern: &str,
        start_byte: u64,
        options: &SearchOptions,
    ) -> Result<Option<u64>>;

    /// Get match ranges for a specific line
    ///
    /// # Arguments
    /// * `pattern` - Search pattern (string or regex depending on options)
    /// * `line` - Line content to search
    /// * `options` - Search configuration options
    ///
    /// # Returns
    /// * Vector of (start, end) byte ranges where matches occur in the line
    ///
    /// # Performance
    /// * Uses cached matcher for the pattern
    /// * SIMD-optimized matching
    fn get_line_matches(
        &self,
        pattern: &str,
        line: &str,
        options: &SearchOptions,
    ) -> Result<Vec<(usize, usize)>>;

    /// Clear internal caches and reset state
    fn clear_cache(&self);
}

/// Cache key for storing compiled search patterns and results
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct SearchCacheKey {
    pattern: String,
    options: SearchOptionsKey,
}

/// Hashable version of SearchOptions for caching
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct SearchOptionsKey {
    case_sensitive: bool,
    whole_word: bool,
    regex_mode: bool,
    context_lines: u32,
}

impl From<&SearchOptions> for SearchOptionsKey {
    fn from(options: &SearchOptions) -> Self {
        Self {
            case_sensitive: options.case_sensitive,
            whole_word: options.whole_word,
            regex_mode: options.regex_mode,
            context_lines: options.context_lines,
        }
    }
}

/// Ripgrep-based search engine implementation
///
/// This implementation leverages the ripgrep ecosystem (grep-searcher, grep-matcher, grep-regex)
/// for SIMD-optimized search performance. It includes intelligent caching and ReDoS protection.
pub struct RipgrepEngine {
    /// File accessor for reading file content
    file_accessor: Arc<dyn FileAccessor>,
    /// LRU cache for compiled regex matchers
    matcher_cache: RwLock<LruCache<SearchCacheKey, Arc<RegexMatcher>>>,
}

impl RipgrepEngine {
    /// Create a new RipgrepEngine instance
    ///
    /// # Arguments
    /// * `file_accessor` - File accessor for reading content
    ///
    /// # Returns
    /// * New RipgrepEngine ready for high-performance search operations
    pub fn new(file_accessor: Arc<dyn FileAccessor>) -> Self {
        Self {
            file_accessor,
            matcher_cache: RwLock::new(LruCache::new(
                NonZeroUsize::new(100).unwrap(), // Cache up to 100 compiled patterns
            )),
        }
    }

    /// Create a search function compatible with FileAccessor API
    ///
    /// This is the key integration point - we create a closure that captures
    /// the compiled regex matcher and returns match ranges for FileAccessor.
    fn create_search_function(
        &self,
        matcher: Arc<RegexMatcher>,
    ) -> impl Fn(&str) -> Vec<(usize, usize)> + Send + Sync {
        move |line: &str| {
            let mut matches = Vec::new();
            let line_bytes = line.as_bytes();

            // Use grep-matcher to find all matches in the line
            let mut start_pos = 0;
            while start_pos < line_bytes.len() {
                if let Ok(Some(m)) = matcher.find_at(line_bytes, start_pos) {
                    matches.push((m.start(), m.end()));
                    start_pos = m.end().max(start_pos + 1); // Prevent infinite loop on zero-width matches
                } else {
                    break;
                }
            }

            matches
        }
    }

    /// Get or create a compiled regex matcher for the given pattern and options
    fn get_or_create_matcher(
        &self,
        pattern: &str,
        options: &SearchOptions,
    ) -> Result<Arc<RegexMatcher>> {
        let cache_key = SearchCacheKey {
            pattern: pattern.to_string(),
            options: options.into(),
        };

        // Try to get from cache first
        {
            let mut cache = self.matcher_cache.write();
            if let Some(matcher) = cache.get(&cache_key) {
                return Ok(matcher.clone());
            }
        }

        // Create new matcher
        let matcher = self.create_matcher(pattern, options)?;
        let matcher = Arc::new(matcher);

        // Store in cache
        {
            let mut cache = self.matcher_cache.write();
            cache.put(cache_key, matcher.clone());
        }

        Ok(matcher)
    }

    /// Create a new regex matcher with the specified options
    fn create_matcher(&self, pattern: &str, options: &SearchOptions) -> Result<RegexMatcher> {
        // Handle whole word matching
        let effective_pattern = if options.whole_word && !options.regex_mode {
            // For literal strings, wrap in word boundaries
            format!(r"\b{}\b", escape_regex(pattern))
        } else if options.whole_word && options.regex_mode {
            // For regex patterns, wrap in word boundaries
            format!(r"\b(?:{})\b", pattern)
        } else if !options.regex_mode {
            // For literal strings, escape regex special characters
            escape_regex(pattern)
        } else {
            // For regex patterns, use as-is
            pattern.to_string()
        };

        // Create matcher with case sensitivity configuration
        let mut builder = RegexMatcherBuilder::new();
        if !options.case_sensitive {
            builder.case_insensitive(true);
        }

        builder.build(&effective_pattern).map_err(|e| {
            RllessError::search_error(format!("Invalid regex pattern: {}", e), e.into())
        })
    }
}

/// Escape special regex characters in a literal string
///
/// This is a simple implementation to escape common regex metacharacters
/// for literal string matching.
fn escape_regex(s: &str) -> String {
    let mut escaped = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\\' | '^' | '$' | '.' | '[' | ']' | '|' | '(' | ')' | '?' | '*' | '+' | '{' | '}' => {
                escaped.push('\\');
                escaped.push(ch);
            }
            _ => escaped.push(ch),
        }
    }
    escaped
}

#[async_trait]
impl SearchEngine for RipgrepEngine {
    async fn search_from(
        &self,
        pattern: &str,
        start_byte: u64,
        options: &SearchOptions,
    ) -> Result<Option<u64>> {
        // Get or create matcher
        let matcher = self.get_or_create_matcher(pattern, options)?;

        // Create search function for FileAccessor
        let search_fn = self.create_search_function(matcher);

        // Define the search operation
        let search_operation = async {
            self.file_accessor
                .find_next_match(start_byte, &search_fn)
                .await
        };

        // Apply timeout if specified
        let search_result = if let Some(timeout_duration) = options.timeout {
            timeout(timeout_duration, search_operation)
                .await
                .map_err(|_| {
                    RllessError::search(format!(
                        "Search timeout after {:?}: pattern too complex",
                        timeout_duration
                    ))
                })?
        } else {
            search_operation.await
        }?;

        Ok(search_result)
    }

    async fn search_prev(
        &self,
        pattern: &str,
        start_byte: u64,
        options: &SearchOptions,
    ) -> Result<Option<u64>> {
        // Get or create matcher
        let matcher = self.get_or_create_matcher(pattern, options)?;

        // Create search function for FileAccessor
        let search_fn = self.create_search_function(matcher);

        // Define the search operation
        let search_operation = async {
            self.file_accessor
                .find_prev_match(start_byte, &search_fn)
                .await
        };

        // Apply timeout if specified
        let search_result = if let Some(timeout_duration) = options.timeout {
            timeout(timeout_duration, search_operation)
                .await
                .map_err(|_| {
                    RllessError::search(format!(
                        "Search timeout after {:?}: pattern too complex",
                        timeout_duration
                    ))
                })?
        } else {
            search_operation.await
        }?;

        Ok(search_result)
    }

    fn get_line_matches(
        &self,
        pattern: &str,
        line: &str,
        options: &SearchOptions,
    ) -> Result<Vec<(usize, usize)>> {
        // Get or create matcher for the pattern
        let matcher = self.get_or_create_matcher(pattern, options)?;

        // Use the same search function logic as FileAccessor integration
        let search_fn = self.create_search_function(matcher);

        // Apply the search function to the line
        Ok(search_fn(line))
    }

    fn clear_cache(&self) {
        self.matcher_cache.write().clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Simple mock FileAccessor for testing
    struct MockFileAccessor {
        content: String,
        lines: Vec<String>,
    }

    impl MockFileAccessor {
        fn from_lines(lines: Vec<String>) -> Self {
            let content = lines.join("\n") + "\n";
            Self { content, lines }
        }

        fn find_line_at_byte(&self, byte_pos: u64) -> Option<usize> {
            let mut current_byte = 0u64;
            for (line_idx, line) in self.lines.iter().enumerate() {
                if byte_pos >= current_byte && byte_pos < current_byte + line.len() as u64 + 1 {
                    return Some(line_idx);
                }
                current_byte += line.len() as u64 + 1; // +1 for newline
            }
            None
        }

        fn byte_pos_of_line(&self, line_idx: usize) -> u64 {
            let mut byte_pos = 0u64;
            for i in 0..line_idx.min(self.lines.len()) {
                byte_pos += self.lines[i].len() as u64 + 1; // +1 for newline
            }
            byte_pos
        }
    }

    #[async_trait]
    impl FileAccessor for MockFileAccessor {
        async fn read_from_byte(&self, start_byte: u64, max_lines: usize) -> Result<Vec<String>> {
            if let Some(start_line) = self.find_line_at_byte(start_byte) {
                let end_line = (start_line + max_lines).min(self.lines.len());
                Ok(self.lines[start_line..end_line]
                    .iter()
                    .map(|s| s.clone())
                    .collect())
            } else {
                Ok(vec![])
            }
        }

        async fn find_next_match(
            &self,
            start_byte: u64,
            search_fn: &(dyn for<'a> Fn(&'a str) -> Vec<(usize, usize)> + Send + Sync),
        ) -> Result<Option<u64>> {
            let start_line = self.find_line_at_byte(start_byte).unwrap_or(0);

            for line_idx in start_line..self.lines.len() {
                let matches = search_fn(&self.lines[line_idx]);
                if !matches.is_empty() {
                    return Ok(Some(self.byte_pos_of_line(line_idx)));
                }
            }
            Ok(None)
        }

        async fn find_prev_match(
            &self,
            start_byte: u64,
            search_fn: &(dyn for<'a> Fn(&'a str) -> Vec<(usize, usize)> + Send + Sync),
        ) -> Result<Option<u64>> {
            let start_line = self
                .find_line_at_byte(start_byte)
                .unwrap_or(self.lines.len());

            for line_idx in (0..start_line).rev() {
                let matches = search_fn(&self.lines[line_idx]);
                if !matches.is_empty() {
                    return Ok(Some(self.byte_pos_of_line(line_idx)));
                }
            }
            Ok(None)
        }

        fn file_size(&self) -> u64 {
            self.content.len() as u64
        }

        fn file_path(&self) -> &std::path::Path {
            std::path::Path::new("mock_file.txt")
        }

        async fn last_page_start(&self, max_lines: usize) -> Result<u64> {
            if self.lines.len() <= max_lines {
                Ok(0)
            } else {
                let start_line = self.lines.len() - max_lines;
                Ok(self.byte_pos_of_line(start_line))
            }
        }

        async fn next_page_start(&self, current_byte: u64, lines_to_skip: usize) -> Result<u64> {
            if let Some(current_line) = self.find_line_at_byte(current_byte) {
                let next_line = (current_line + lines_to_skip).min(self.lines.len());
                Ok(self.byte_pos_of_line(next_line))
            } else {
                Ok(current_byte)
            }
        }

        async fn prev_page_start(&self, current_byte: u64, lines_to_skip: usize) -> Result<u64> {
            if let Some(current_line) = self.find_line_at_byte(current_byte) {
                let prev_line = current_line.saturating_sub(lines_to_skip);
                Ok(self.byte_pos_of_line(prev_line))
            } else {
                Ok(0)
            }
        }
    }

    fn create_test_engine() -> RipgrepEngine {
        let lines = vec![
            "The quick brown fox".to_string(),
            "jumps over the lazy dog".to_string(),
            "Pack my box with five dozen liquor jugs".to_string(),
            "How vexingly quick daft zebras jump!".to_string(),
        ];
        let accessor = Arc::new(MockFileAccessor::from_lines(lines));
        RipgrepEngine::new(accessor)
    }

    #[tokio::test]
    async fn test_basic_search() {
        let engine = create_test_engine();
        let options = SearchOptions::default();

        let result = engine.search_from("fox", 0, &options).await.unwrap();
        assert!(result.is_some());

        let byte_position = result.unwrap();
        assert_eq!(byte_position, 0); // "fox" found at start of first line

        // Test get_line_matches for highlight computation
        let match_ranges = engine
            .get_line_matches("fox", "The quick brown fox", &options)
            .unwrap();
        assert_eq!(match_ranges, vec![(16, 19)]);
    }

    #[tokio::test]
    async fn test_case_insensitive_search() {
        let engine = create_test_engine();
        let options = SearchOptions {
            case_sensitive: false,
            ..Default::default()
        };

        let result = engine.search_from("FOX", 0, &options).await.unwrap();
        assert!(result.is_some());

        let byte_position = result.unwrap();
        assert_eq!(byte_position, 0); // "FOX" found at start of first line (case insensitive)

        // Test case insensitive matching
        let match_ranges = engine
            .get_line_matches("FOX", "The quick brown fox", &options)
            .unwrap();
        assert_eq!(match_ranges, vec![(16, 19)]);
    }

    #[tokio::test]
    async fn test_regex_search() {
        let engine = create_test_engine();
        let options = SearchOptions {
            regex_mode: true,
            ..Default::default()
        };

        let result = engine.search_from(r"qu\w+k", 0, &options).await.unwrap();
        assert!(result.is_some());

        let byte_position = result.unwrap();
        assert_eq!(byte_position, 0); // "quick" pattern found at start of first line

        // Test regex matching
        let match_ranges = engine
            .get_line_matches(r"qu\w+k", "The quick brown fox", &options)
            .unwrap();
        assert_eq!(match_ranges, vec![(4, 9)]); // "quick"
    }

    #[tokio::test]
    async fn test_whole_word_search() {
        let engine = create_test_engine();
        let options = SearchOptions {
            whole_word: true,
            ..Default::default()
        };

        // Should find "box" as a whole word in line 2 ("Pack my box with five dozen liquor jugs")
        let result = engine.search_from("box", 0, &options).await.unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap(), 44); // Line 2 starts at byte 44

        // Should NOT find "ox" as it's part of "fox"
        let result = engine.search_from("ox", 0, &options).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_search_prev() {
        let engine = create_test_engine();
        let options = SearchOptions::default();

        // Search backward from near end of file for "jump" - should find in line 2
        let result = engine.search_prev("jump", 100, &options).await.unwrap();
        assert!(result.is_some());

        let byte_position = result.unwrap();
        assert_eq!(byte_position, 20); // Line 2 "jumps over the lazy dog" starts at byte 20
    }

    #[tokio::test]
    async fn test_search_caching() {
        let engine = create_test_engine();
        let options = SearchOptions::default();

        // First search
        let result1 = engine.search_from("fox", 0, &options).await.unwrap();

        // Second search (should use cached regex matcher)
        let result2 = engine.search_from("fox", 0, &options).await.unwrap();

        // Both searches should return the same result
        assert_eq!(result1, result2);
        assert!(result1.is_some());
    }

    #[tokio::test]
    async fn test_invalid_regex() {
        let engine = create_test_engine();
        let options = SearchOptions {
            regex_mode: true,
            ..Default::default()
        };

        let result = engine.search_from("[invalid", 0, &options).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_search_no_timeout() {
        let engine = create_test_engine();

        // Create options with no timeout
        let options = SearchOptions {
            timeout: None,
            ..Default::default()
        };

        // This search should complete successfully
        let result = engine.search_from("fox", 0, &options).await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_some());
    }

    #[test]
    fn test_get_line_matches() {
        let engine = create_test_engine();
        let options = SearchOptions::default();

        // Test single match
        let matches = engine
            .get_line_matches("fox", "The quick brown fox", &options)
            .unwrap();
        assert_eq!(matches, vec![(16, 19)]);

        // Test multiple matches
        let test_line = "The quick brown fox jumps over the quiet dog";
        let matches = engine.get_line_matches("qu", test_line, &options).unwrap();
        // Let's find where "qu" actually appears:
        // "The quick brown fox jumps over the quiet dog"
        //      ^^                              ^^
        //      4-6                             35-37
        assert_eq!(matches, vec![(4, 6), (35, 37)]); // "qu" in "quick" and "quiet"

        // Test no matches
        let matches = engine
            .get_line_matches("xyz", "The quick brown fox", &options)
            .unwrap();
        assert!(matches.is_empty());

        // Test regex mode
        let regex_options = SearchOptions {
            regex_mode: true,
            ..Default::default()
        };
        let matches = engine
            .get_line_matches(r"\b\w{5}\b", "The quick brown fox jumps", &regex_options)
            .unwrap();
        assert_eq!(matches, vec![(4, 9), (10, 15), (20, 25)]); // "quick", "brown", "jumps"
    }
}
