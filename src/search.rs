//! Search engine module with SIMD-optimized search capabilities
//!
//! This module provides a flexible search interface built on top of the ripgrep ecosystem
//! for maximum performance. It supports regex patterns, case sensitivity, whole word matching,
//! and provides efficient result caching.

use crate::error::{Result, RllessError};
use crate::file_handler::accessor::{FileAccessor, MatchInfo};
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
    /// * Some(MatchInfo) if pattern found
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
    ) -> Result<Option<MatchInfo>>;

    /// Search for the previous occurrence of a pattern
    ///
    /// # Arguments
    /// * `pattern` - Search pattern (string or regex depending on options)
    /// * `start_line` - Line number to start searching from (0-based, exclusive)
    /// * `options` - Search configuration options
    ///
    /// # Returns
    /// * Some(MatchInfo) if pattern found
    /// * None if pattern not found before beginning of file
    async fn search_prev(
        &self,
        pattern: &str,
        start_line: u64,
        options: &SearchOptions,
    ) -> Result<Option<MatchInfo>>;

    /// Search for all occurrences of a pattern in the file
    ///
    /// # Arguments
    /// * `pattern` - Search pattern (string or regex depending on options)
    /// * `options` - Search configuration options
    /// * `max_results` - Maximum number of results to return (None for unlimited)
    ///
    /// # Returns
    /// * Vector of all matches found
    ///
    /// # Performance
    /// * Uses streaming search to avoid memory explosion
    /// * Results are yielded as they're found
    async fn search_all(
        &self,
        pattern: &str,
        options: &SearchOptions,
        max_results: Option<usize>,
    ) -> Result<Vec<MatchInfo>>;

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
    /// Cache hit ratio (0.0 to 1.0)
    pub cache_hit_ratio: f64,
    /// Average search time in milliseconds
    pub avg_search_time_ms: f64,
    /// Number of patterns currently cached
    pub cached_patterns: usize,
    /// Total bytes searched
    pub total_bytes_searched: u64,
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
    /// LRU cache for recent search results
    result_cache: RwLock<LruCache<(SearchCacheKey, u64), Option<MatchInfo>>>,
    /// Performance statistics
    stats: RwLock<SearchStats>,
    /// Search operation timeouts
    #[allow(dead_code)] // Will be used for ReDoS protection
    timeout_duration: Duration,
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
            timeout_duration: Duration::from_secs(10),
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

    /// Add context lines to a search match
    async fn add_context(
        &self,
        mut search_match: MatchInfo,
        options: &SearchOptions,
    ) -> Result<MatchInfo> {
        if options.context_lines == 0 {
            return Ok(search_match);
        }

        let context_lines = options.context_lines as u64;

        // Add context before
        let context_start = search_match.line_number.saturating_sub(context_lines);
        if context_start < search_match.line_number {
            let before_lines = self
                .file_accessor
                .read_lines_range(context_start, search_match.line_number - context_start)
                .await?;
            search_match.context_before = before_lines
                .into_iter()
                .map(|cow| cow.into_owned())
                .collect();
        }

        // Add context after
        let after_lines = self
            .file_accessor
            .read_lines_range(search_match.line_number + 1, context_lines)
            .await?;
        search_match.context_after = after_lines
            .into_iter()
            .map(|cow| cow.into_owned())
            .collect();

        Ok(search_match)
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
    ) -> Result<Option<MatchInfo>> {
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
                // Update cache hit ratio
                {
                    let mut stats = self.stats.write();
                    let total = stats.total_searches as f64;
                    let hits = (stats.cache_hit_ratio * (total - 1.0)) + 1.0;
                    stats.cache_hit_ratio = hits / total;
                }
                return Ok(cached_result.clone());
            }
        }

        // Get or create matcher
        let matcher = self.get_or_create_matcher(pattern, options)?;

        // Create search function for FileAccessor
        let search_fn = self.create_search_function(matcher);

        // Define the search operation
        let search_operation = async {
            let result = self
                .file_accessor
                .find_next_match(start_line, &search_fn)
                .await?;

            // Add context to the match if found
            match result {
                Some(match_info) => {
                    let with_context = self.add_context(match_info, options).await?;
                    Result::<Option<MatchInfo>>::Ok(Some(with_context))
                }
                None => Result::<Option<MatchInfo>>::Ok(None),
            }
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
            cache.put((cache_key, start_line), search_result.clone());
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
    ) -> Result<Option<MatchInfo>> {
        let start_time = Instant::now();

        // Update stats
        {
            let mut stats = self.stats.write();
            stats.total_searches += 1;
        }

        // Get or create matcher
        let matcher = self.get_or_create_matcher(pattern, options)?;

        // Create search function for FileAccessor
        let search_fn = self.create_search_function(matcher);

        // Define the search operation
        let search_operation = async {
            let result = self
                .file_accessor
                .find_prev_match(start_line, &search_fn)
                .await?;

            // Add context to the match if found
            match result {
                Some(match_info) => {
                    let with_context = self.add_context(match_info, options).await?;
                    Result::<Option<MatchInfo>>::Ok(Some(with_context))
                }
                None => Result::<Option<MatchInfo>>::Ok(None),
            }
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

    async fn search_all(
        &self,
        pattern: &str,
        options: &SearchOptions,
        max_results: Option<usize>,
    ) -> Result<Vec<MatchInfo>> {
        let mut results = Vec::new();
        let mut current_line = 0;
        let max = max_results.unwrap_or(usize::MAX);

        while results.len() < max {
            if let Some(search_match) = self.search_from(pattern, current_line, options).await? {
                current_line = search_match.line_number + 1;
                results.push(search_match);
            } else {
                break; // No more matches
            }
        }

        Ok(results)
    }

    fn get_stats(&self) -> SearchStats {
        let stats = self.stats.read();
        let mut result = stats.clone();

        // Add current cache sizes
        result.cached_patterns = self.matcher_cache.read().len();

        result
    }

    fn clear_cache(&self) {
        self.matcher_cache.write().clear();
        self.result_cache.write().clear();

        // Reset cache hit ratio but keep other stats
        let mut stats = self.stats.write();
        stats.cache_hit_ratio = 0.0;
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

        let search_match = result.unwrap();
        assert_eq!(search_match.line_number, 0);
        assert_eq!(search_match.line_content, "The quick brown fox");
        assert_eq!(search_match.match_ranges, vec![(16, 19)]);
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

        let search_match = result.unwrap();
        assert_eq!(search_match.line_number, 0);
        assert_eq!(search_match.match_ranges, vec![(16, 19)]);
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

        let search_match = result.unwrap();
        assert_eq!(search_match.line_number, 0);
        assert_eq!(search_match.match_ranges, vec![(4, 9)]); // "quick"
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
        assert_eq!(result.unwrap().line_number, 2);

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

        let search_match = result.unwrap();
        assert_eq!(search_match.line_number, 3); // "How vexingly quick daft zebras jump!"
    }

    #[tokio::test]
    async fn test_search_all() {
        let engine = create_test_engine();
        let options = SearchOptions::default();

        let results = engine.search_all("qu", &options, None).await.unwrap();
        assert_eq!(results.len(), 3); // "qu" appears in lines 0, 2, and 3

        assert_eq!(results[0].line_number, 0);
        assert_eq!(results[1].line_number, 2);
        assert_eq!(results[2].line_number, 3);
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
}
