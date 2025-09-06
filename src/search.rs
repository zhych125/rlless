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
use std::time::{Duration, Instant};
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
    /// Search for a pattern starting from a specific line
    ///
    /// # Arguments
    /// * `pattern` - Search pattern (string or regex depending on options)
    /// * `start_line` - Line number to start searching from (0-based, inclusive)
    /// * `options` - Search configuration options
    ///
    /// # Returns
    /// * Some(line_number) if pattern found
    /// * None if pattern not found before EOF
    ///
    /// # Performance
    /// * Target: <500ms for 40GB files
    /// * Uses SIMD optimization when available
    /// * Leverages result caching for repeated searches
    async fn search_from(
        &self,
        pattern: &str,
        start_line: u64,
        options: &SearchOptions,
    ) -> Result<Option<u64>>;

    /// Search for the previous occurrence of a pattern
    ///
    /// # Arguments
    /// * `pattern` - Search pattern (string or regex depending on options)
    /// * `start_line` - Line number to start searching from (0-based, exclusive)
    /// * `options` - Search configuration options
    ///
    /// # Returns
    /// * Some(line_number) if pattern found
    /// * None if pattern not found before beginning of file
    async fn search_prev(
        &self,
        pattern: &str,
        start_line: u64,
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

    /// Get search statistics and performance metrics
    fn get_stats(&self) -> SearchStats;

    /// Clear internal caches and reset state
    fn clear_cache(&self);
}

/// Search performance and usage statistics
#[derive(Debug, Clone, Default)]
pub struct SearchStats {
    /// Total number of searches performed
    pub total_searches: u64,
    /// Number of cache hits
    pub cache_hits: u64,
    /// Cache hit ratio (0.0 to 1.0) - computed on demand
    pub cache_hit_ratio: f64,
    /// Average search time in milliseconds
    pub avg_search_time_ms: f64,
    /// Number of patterns currently cached
    pub cached_patterns: usize,
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
    /// LRU cache for recent search results (line numbers)
    result_cache: RwLock<LruCache<(SearchCacheKey, u64), Option<u64>>>,
    /// Performance statistics
    stats: RwLock<SearchStats>,
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
            result_cache: RwLock::new(LruCache::new(
                NonZeroUsize::new(1000).unwrap(), // Cache up to 1000 recent results
            )),
            stats: RwLock::new(SearchStats::default()),
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
        start_line: u64,
        options: &SearchOptions,
    ) -> Result<Option<u64>> {
        let start_time = Instant::now();

        // Update stats
        {
            let mut stats = self.stats.write();
            stats.total_searches += 1;
        }

        // Check result cache first
        let cache_key = SearchCacheKey {
            pattern: pattern.to_string(),
            options: options.into(),
        };

        {
            let mut cache = self.result_cache.write();
            if let Some(cached_result) = cache.get(&(cache_key.clone(), start_line)) {
                // Update cache hit count
                {
                    let mut stats = self.stats.write();
                    stats.cache_hits += 1;
                }
                return Ok(*cached_result);
            }
        }

        // Get or create matcher
        let matcher = self.get_or_create_matcher(pattern, options)?;

        // Create search function for FileAccessor
        let search_fn = self.create_search_function(matcher);

        // Define the search operation
        let search_operation = async {
            self.file_accessor
                .find_next_match(start_line, &search_fn)
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

        // Cache the result
        {
            let mut cache = self.result_cache.write();
            cache.put((cache_key, start_line), search_result);
        }

        // Update performance stats
        {
            let mut stats = self.stats.write();
            let search_time_ms = start_time.elapsed().as_millis() as f64;
            let total = stats.total_searches as f64;
            stats.avg_search_time_ms =
                ((stats.avg_search_time_ms * (total - 1.0)) + search_time_ms) / total;
        }

        Ok(search_result)
    }

    async fn search_prev(
        &self,
        pattern: &str,
        start_line: u64,
        options: &SearchOptions,
    ) -> Result<Option<u64>> {
        let start_time = Instant::now();

        // Update stats
        {
            let mut stats = self.stats.write();
            stats.total_searches += 1;
        }

        // Check result cache first
        let cache_key = SearchCacheKey {
            pattern: pattern.to_string(),
            options: options.into(),
        };

        {
            let mut cache = self.result_cache.write();
            if let Some(cached_result) = cache.get(&(cache_key.clone(), start_line)) {
                // Update cache hit count
                {
                    let mut stats = self.stats.write();
                    stats.cache_hits += 1;
                }
                return Ok(*cached_result);
            }
        }

        // Get or create matcher
        let matcher = self.get_or_create_matcher(pattern, options)?;

        // Create search function for FileAccessor
        let search_fn = self.create_search_function(matcher);

        // Define the search operation
        let search_operation = async {
            self.file_accessor
                .find_prev_match(start_line, &search_fn)
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

        // Cache the result
        {
            let mut cache = self.result_cache.write();
            cache.put((cache_key, start_line), search_result);
        }

        // Update performance stats
        {
            let mut stats = self.stats.write();
            let search_time_ms = start_time.elapsed().as_millis() as f64;
            let total = stats.total_searches as f64;
            stats.avg_search_time_ms =
                ((stats.avg_search_time_ms * (total - 1.0)) + search_time_ms) / total;
        }

        Ok(search_result)
    }

    fn get_stats(&self) -> SearchStats {
        let stats = self.stats.read();
        let mut result = stats.clone();

        // Add current cache sizes
        result.cached_patterns = self.matcher_cache.read().len();

        // Compute cache hit ratio on demand
        result.cache_hit_ratio = if result.total_searches > 0 {
            result.cache_hits as f64 / result.total_searches as f64
        } else {
            0.0
        };

        result
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
        self.result_cache.write().clear();

        // Reset cache hits but keep other stats
        let mut stats = self.stats.write();
        stats.cache_hits = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file_handler::accessor::tests::MockFileAccessor;

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

        let line_number = result.unwrap();
        assert_eq!(line_number, 0);

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

        let line_number = result.unwrap();
        assert_eq!(line_number, 0);

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

        let line_number = result.unwrap();
        assert_eq!(line_number, 0);

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

        // Should find "box" as a whole word
        let result = engine.search_from("box", 0, &options).await.unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap(), 2);

        // Should NOT find "ox" as it's part of "fox"
        let result = engine.search_from("ox", 0, &options).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_search_prev() {
        let engine = create_test_engine();
        let options = SearchOptions::default();

        let result = engine.search_prev("jump", 4, &options).await.unwrap();
        assert!(result.is_some());

        let line_number = result.unwrap();
        assert_eq!(line_number, 3); // "How vexingly quick daft zebras jump!"
    }

    #[tokio::test]
    async fn test_search_caching() {
        let engine = create_test_engine();
        let options = SearchOptions::default();

        // First search
        let _result1 = engine.search_from("fox", 0, &options).await.unwrap();

        // Second search (should hit cache)
        let _result2 = engine.search_from("fox", 0, &options).await.unwrap();

        let stats = engine.get_stats();
        assert_eq!(stats.total_searches, 2);
        assert!(stats.cache_hit_ratio > 0.0);
        assert_eq!(stats.cached_patterns, 1);
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
