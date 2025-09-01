# File Accessor Implementation Design

This document provides the detailed implementation plan for Task 4 of Phase 1, implementing the memory mapping strategy with SIMD-optimized line finding for rlless.

## Architecture Overview

```
┌─────────────────────────────────────────┐
│         FileAccessorFactory             │  ← Decides strategy based on file size
├─────────────────────────────────────────┤
│ < 10MB                 │ ≥ 10MB        │
├────────────────────────┼────────────────┤
│ InMemoryFileAccessor   │ MmapFileAccessor│  ← Two implementations
├────────────────────────┴────────────────┤
│            FileAccessor Trait           │  ← Common interface
├─────────────────────────────────────────┤
│      LineIndex (SIMD-powered)          │  ← Shared line finding logic
└─────────────────────────────────────────┘
```

## Core Components

### 1. FileAccessor Trait (Updated)

```rust
// src/file_handler/accessor.rs

use async_trait::async_trait;
use crate::error::Result;

/// Information about a search match in the file
#[derive(Debug, Clone)]
pub struct MatchInfo {
    /// Line number where the match was found (0-based)
    pub line_number: u64,
    
    /// Byte offset from start of file where this line begins
    /// Useful for seeking directly to this position
    pub byte_offset: u64,
    
    /// Full content of the line containing the match
    /// Already converted to UTF-8 (lossy if needed)
    pub line_content: String,
    
    /// Character position within the line where match starts
    /// Used for highlighting in UI
    pub match_start: usize,
    
    /// Character position within the line where match ends
    /// Used for highlighting in UI
    pub match_end: usize,
}

/// Core trait for file access operations
/// 
/// This trait provides a unified interface for both small files (loaded into memory)
/// and large files (memory-mapped). All implementations must be thread-safe.
#[async_trait]
pub trait FileAccessor: Send + Sync {
    /// Read a single line by line number
    /// 
    /// # Arguments
    /// * `line_number` - 0-based line number to read
    /// 
    /// # Returns
    /// * The line content without trailing newline
    /// * Error if line_number is out of bounds
    /// 
    /// # Performance
    /// * InMemory: O(1) - direct index lookup
    /// * Mmap: O(1) after indexing, may trigger lazy indexing on first access
    /// 
    /// # Usage
    /// Used when user jumps to specific line or navigates with arrow keys
    async fn read_line(&self, line_number: u64) -> Result<String>;
    
    /// Read multiple consecutive lines efficiently
    /// 
    /// # Arguments
    /// * `start` - First line number to read (0-based)
    /// * `count` - Number of lines to read
    /// 
    /// # Returns
    /// * Vector of lines, may be shorter than `count` if EOF reached
    /// * Empty vector if `start` is beyond EOF
    /// 
    /// # Performance
    /// * Optimized for bulk reading (e.g., filling terminal screen)
    /// * Uses bstr for efficient line iteration
    /// 
    /// # Usage
    /// Used for initial screen fill, page up/down, showing context
    async fn read_lines_range(&self, start: u64, count: u64) -> Result<Vec<String>>;
    
    /// Find next occurrence of pattern searching forward from start_line
    /// 
    /// # Arguments
    /// * `start_line` - Line number to start searching from (inclusive)
    /// * `pattern` - String pattern to search for (substring match)
    /// 
    /// # Returns
    /// * Some(MatchInfo) if pattern found
    /// * None if pattern not found before EOF
    /// 
    /// # Performance
    /// * Searches incrementally, returns as soon as match found
    /// * Does not scan entire file
    /// 
    /// # Usage
    /// Used for 'n' (next) command in search, Find Next in UI
    async fn find_next_match(&self, start_line: u64, pattern: &str) -> Result<Option<MatchInfo>>;
    
    /// Find previous occurrence of pattern searching backward from start_line
    /// 
    /// # Arguments
    /// * `start_line` - Line number to start searching from (exclusive)
    /// * `pattern` - String pattern to search for (substring match)
    /// 
    /// # Returns
    /// * Some(MatchInfo) if pattern found
    /// * None if pattern not found before beginning of file
    /// 
    /// # Performance
    /// * Searches incrementally backward
    /// * May be slower than forward search due to access patterns
    /// 
    /// # Usage
    /// Used for 'N' (previous) command in search, Find Previous in UI
    async fn find_prev_match(&self, start_line: u64, pattern: &str) -> Result<Option<MatchInfo>>;
    
    /// Get the total file size in bytes
    /// 
    /// # Returns
    /// * File size in bytes
    /// 
    /// # Performance
    /// * O(1) - cached from file metadata
    /// 
    /// # Usage
    /// Used for progress indicators, statistics display
    fn file_size(&self) -> u64;
    
    /// Get total number of lines in file if known
    /// 
    /// # Returns
    /// * Some(count) if line count is known
    /// * None if not yet computed (large files with lazy indexing)
    /// 
    /// # Performance
    /// * InMemory: Always returns Some (computed at load time)
    /// * Mmap: Returns None initially, Some after sufficient indexing
    /// 
    /// # Usage
    /// Used for scroll bar positioning, jump to percentage, statistics
    fn total_lines(&self) -> Option<u64>;
    
    /// Check if this implementation supports parallel operations
    /// 
    /// # Returns
    /// * true if parallel operations are available
    /// 
    /// # Usage
    /// Used to determine if ParallelFileProcessor trait is available
    fn supports_parallel(&self) -> bool {
        false // Default: no parallel support
    }
}
```

### 2. LineIndex Structure

```rust
// src/file_handler/line_index.rs

use memchr::memchr;
use std::collections::VecDeque;

/// Cached line information for LRU cache
#[derive(Debug, Clone)]
struct CachedLine {
    /// Line number (0-based) for cache lookup
    line_number: u64,
    
    /// Cached line content (UTF-8 converted)
    /// Stored to avoid repeated UTF-8 conversion
    content: String,
    
    /// Byte position where this line starts in file
    /// Used for chunk operations
    byte_start: u64,
    
    /// Byte position where this line ends (before newline)
    /// Used for chunk operations
    byte_end: u64,
}

/// SIMD-optimized line boundary finder with intelligent caching
/// 
/// This structure maintains an index of line boundaries in a file,
/// building the index lazily as lines are accessed. It uses memchr
/// for SIMD-optimized newline detection.
#[derive(Debug)]
pub struct LineIndex {
    /// Byte offsets where each line starts
    /// 
    /// - line_offsets[0] = 0 (first line always starts at byte 0)
    /// - line_offsets[n] = byte position after nth newline
    /// - Length of this vector - 1 = number of indexed lines
    /// 
    /// Grows monotonically as more of the file is indexed
    line_offsets: Vec<u64>,
    
    /// How far into the file we've indexed (in bytes)
    /// 
    /// Everything before this position has been scanned for newlines.
    /// Used to resume indexing from where we left off.
    indexed_to_byte: u64,
    
    /// LRU cache of recently accessed lines
    /// 
    /// - Front = most recently used
    /// - Back = least recently used
    /// - Eviction happens from back when cache is full
    /// 
    /// Speeds up repeated access to same lines (common in scrolling)
    line_cache: VecDeque<CachedLine>,
    
    /// Maximum number of lines to keep in cache
    /// 
    /// Typically 100-1000 lines depending on available memory.
    /// Prevents unbounded memory growth.
    max_cache_size: usize,
}

impl LineIndex {
    /// Create a new empty line index
    pub fn new() -> Self {
        Self {
            line_offsets: vec![0], // First line starts at position 0
            indexed_to_byte: 0,
            line_cache: VecDeque::with_capacity(100),
            max_cache_size: 100,
        }
    }
    
    /// Create with custom cache size
    pub fn with_cache_size(max_cache_size: usize) -> Self {
        Self {
            line_offsets: vec![0],
            indexed_to_byte: 0,
            line_cache: VecDeque::with_capacity(max_cache_size),
            max_cache_size,
        }
    }
    
    /// Ensure we have indexed at least up to the target line
    /// 
    /// # Arguments
    /// * `data` - The file data to scan (either mmap or in-memory buffer)
    /// * `target_line` - Line number we need to have indexed
    /// 
    /// # Performance
    /// * Uses SIMD-optimized memchr for finding newlines
    /// * Only scans the portion of file not yet indexed
    /// * O(n) where n is bytes to scan, but only runs once per file region
    pub fn ensure_indexed_to(&mut self, data: &[u8], target_line: u64) {
        let current_lines = (self.line_offsets.len() - 1) as u64;
        
        if target_line <= current_lines {
            return; // Already indexed enough lines
        }
        
        let mut pos = self.indexed_to_byte as usize;
        
        // Use memchr for SIMD-optimized newline search
        while pos < data.len() {
            if let Some(newline_offset) = memchr(b'\n', &data[pos..]) {
                pos += newline_offset + 1; // Move past the newline
                self.line_offsets.push(pos as u64);
                
                // Check if we've indexed enough
                if self.line_offsets.len() > target_line as usize + 1 {
                    break;
                }
            } else {
                // No more newlines found, we've reached end of file
                break;
            }
        }
        
        self.indexed_to_byte = pos as u64;
    }
    
    /// Get a line from cache if available
    /// 
    /// # Returns
    /// * Some(content) if line is cached
    /// * None if line is not in cache
    /// 
    /// # Side Effects
    /// * Moves accessed line to front of cache (LRU update)
    pub fn get_cached_line(&mut self, line_number: u64) -> Option<String> {
        if let Some(pos) = self.line_cache.iter().position(|c| c.line_number == line_number) {
            // Move to front (mark as most recently used)
            let cached = self.line_cache.remove(pos).unwrap();
            let content = cached.content.clone();
            self.line_cache.push_front(cached);
            return Some(content);
        }
        None
    }
    
    /// Add a line to the cache
    /// 
    /// # Arguments
    /// * `line_number` - Line number to cache
    /// * `content` - Line content (already UTF-8 converted)
    /// * `byte_start` - Where this line starts in the file
    /// * `byte_end` - Where this line ends in the file
    /// 
    /// # Side Effects
    /// * May evict least recently used line if cache is full
    pub fn cache_line(&mut self, line_number: u64, content: String, byte_start: u64, byte_end: u64) {
        // Remove if already cached (to avoid duplicates)
        if let Some(pos) = self.line_cache.iter().position(|c| c.line_number == line_number) {
            self.line_cache.remove(pos);
        }
        
        // Add to front (most recently used)
        self.line_cache.push_front(CachedLine {
            line_number,
            content,
            byte_start,
            byte_end,
        });
        
        // Evict LRU if cache is too large
        while self.line_cache.len() > self.max_cache_size {
            self.line_cache.pop_back();
        }
    }
    
    /// Get line start positions for a range
    /// 
    /// # Returns
    /// * Slice of line offsets that have been indexed
    /// 
    /// # Usage
    /// Used for calculating line positions in chunk operations
    pub fn get_line_offsets(&self) -> &[u64] {
        &self.line_offsets
    }
    
    /// Check how many lines have been indexed
    /// 
    /// # Returns
    /// * Number of lines that have known positions
    pub fn indexed_line_count(&self) -> u64 {
        (self.line_offsets.len() - 1) as u64
    }
    
    /// Check how many bytes have been indexed
    /// 
    /// # Returns
    /// * Byte position up to which we've scanned for newlines
    pub fn indexed_byte_count(&self) -> u64 {
        self.indexed_to_byte
    }
}
```

### 3. InMemoryFileAccessor

```rust
// src/file_handler/in_memory.rs

use crate::error::{Result, RllessError};
use crate::file_handler::accessor::{FileAccessor, MatchInfo};
use async_trait::async_trait;
use bstr::ByteSlice;
use std::path::{Path, PathBuf};

/// File accessor for small files that loads entire content into memory
/// 
/// This implementation is used for files smaller than 10MB. It loads the
/// entire file into memory at creation time and pre-computes all line
/// boundaries for O(1) line access.
pub struct InMemoryFileAccessor {
    /// Complete file content in memory
    /// 
    /// Stored as bytes to handle files with mixed or invalid UTF-8.
    /// Conversion to UTF-8 happens on demand when reading lines.
    content: Vec<u8>,
    
    /// Pre-computed line start positions
    /// 
    /// Built once at load time since file is small.
    /// Provides O(1) access to any line.
    line_offsets: Vec<u64>,
    
    /// Total file size in bytes
    /// 
    /// Cached for quick access, equals content.len()
    file_size: u64,
    
    /// Path to the original file
    /// 
    /// Kept for error messages and debugging
    path: PathBuf,
}

impl InMemoryFileAccessor {
    /// Load a small file completely into memory
    /// 
    /// # Arguments
    /// * `path` - Path to the file to load
    /// 
    /// # Returns
    /// * Ok(accessor) if file loads successfully
    /// * Err if file cannot be read or is too large
    /// 
    /// # Performance
    /// * O(n) where n is file size
    /// * All work done upfront, subsequent access is O(1)
    pub async fn new(path: &Path) -> Result<Self> {
        // Read entire file into memory
        let content = tokio::fs::read(path).await
            .map_err(|e| RllessError::file_error("Failed to read file", e))?;
        
        let file_size = content.len() as u64;
        
        // Pre-compute all line offsets since file is small
        let line_offsets = Self::compute_line_offsets(&content);
        
        Ok(Self {
            content,
            line_offsets,
            file_size,
            path: path.to_owned(),
        })
    }
    
    /// Pre-compute all line boundaries in the file
    /// 
    /// # Arguments
    /// * `content` - The complete file content
    /// 
    /// # Returns
    /// * Vector of byte positions where each line starts
    /// 
    /// # Performance
    /// * O(n) where n is file size
    /// * Uses SIMD-optimized memchr for newline detection
    fn compute_line_offsets(content: &[u8]) -> Vec<u64> {
        let mut offsets = vec![0]; // First line starts at 0
        let mut pos = 0;
        
        // Use memchr for SIMD-optimized scanning
        while let Some(newline_pos) = memchr::memchr(b'\n', &content[pos..]) {
            pos += newline_pos + 1;
            if pos < content.len() {
                offsets.push(pos as u64);
            }
        }
        
        offsets
    }
    
    /// Helper to get line boundaries for a given line number
    fn get_line_bounds(&self, line_number: u64) -> Option<(usize, usize)> {
        if line_number >= self.line_offsets.len() as u64 {
            return None;
        }
        
        let start = self.line_offsets[line_number as usize] as usize;
        let end = if line_number + 1 < self.line_offsets.len() as u64 {
            // Next line exists, end at byte before it starts
            (self.line_offsets[line_number as usize + 1] - 1) as usize
        } else {
            // Last line, find newline or use file end
            memchr::memchr(b'\n', &self.content[start..])
                .map(|pos| start + pos)
                .unwrap_or(self.content.len())
        };
        
        Some((start, end))
    }
}

#[async_trait]
impl FileAccessor for InMemoryFileAccessor {
    async fn read_line(&self, line_number: u64) -> Result<String> {
        let (start, end) = self.get_line_bounds(line_number)
            .ok_or_else(|| RllessError::line_out_of_bounds(line_number))?;
        
        // Use bstr for safe UTF-8 conversion
        Ok(self.content[start..end].to_str_lossy().into_owned())
    }
    
    async fn read_lines_range(&self, start_line: u64, count: u64) -> Result<Vec<String>> {
        let mut lines = Vec::with_capacity(count.min(1000) as usize);
        
        for line_num in start_line..start_line + count {
            match self.get_line_bounds(line_num) {
                Some((start, end)) => {
                    lines.push(self.content[start..end].to_str_lossy().into_owned());
                }
                None => break, // Reached end of file
            }
        }
        
        Ok(lines)
    }
    
    async fn find_next_match(&self, start_line: u64, pattern: &str) -> Result<Option<MatchInfo>> {
        // Get starting position in bytes
        let start_byte = self.line_offsets.get(start_line as usize)
            .copied()
            .unwrap_or(self.content.len() as u64) as usize;
        
        // Use bstr for line iteration from start position
        for (relative_line, line) in self.content[start_byte..].lines().enumerate() {
            if let Some(match_pos) = line.find_str(pattern) {
                let line_number = start_line + relative_line as u64;
                let byte_offset = self.line_offsets.get(line_number as usize)
                    .copied()
                    .unwrap_or(0);
                
                return Ok(Some(MatchInfo {
                    line_number,
                    byte_offset,
                    line_content: line.to_str_lossy().into_owned(),
                    match_start: match_pos,
                    match_end: match_pos + pattern.len(),
                }));
            }
        }
        
        Ok(None)
    }
    
    async fn find_prev_match(&self, start_line: u64, pattern: &str) -> Result<Option<MatchInfo>> {
        if start_line == 0 {
            return Ok(None);
        }
        
        // Search backward from start_line - 1
        for line_number in (0..start_line).rev() {
            if let Some((start, end)) = self.get_line_bounds(line_number) {
                let line = &self.content[start..end];
                if let Some(match_pos) = line.find_str(pattern) {
                    return Ok(Some(MatchInfo {
                        line_number,
                        byte_offset: start as u64,
                        line_content: line.to_str_lossy().into_owned(),
                        match_start: match_pos,
                        match_end: match_pos + pattern.len(),
                    }));
                }
            }
        }
        
        Ok(None)
    }
    
    fn file_size(&self) -> u64 {
        self.file_size
    }
    
    fn total_lines(&self) -> Option<u64> {
        // We always know the total since we indexed everything upfront
        Some(self.line_offsets.len() as u64)
    }
}
```

### 4. MmapFileAccessor

```rust
// src/file_handler/mmap.rs

use crate::error::{Result, RllessError};
use crate::file_handler::accessor::{FileAccessor, MatchInfo};
use crate::file_handler::line_index::LineIndex;
use async_trait::async_trait;
use bstr::ByteSlice;
use memmap2::{Mmap, MmapOptions};
use parking_lot::RwLock;
use std::path::{Path, PathBuf};

/// File accessor for large files using memory mapping
/// 
/// This implementation is used for files 10MB and larger. It memory maps
/// the file and builds a line index lazily as lines are accessed.
pub struct MmapFileAccessor {
    /// Memory-mapped file data
    /// 
    /// The entire file is mapped into virtual memory but pages are only
    /// loaded into physical memory on access.
    mmap: Mmap,
    
    /// Lazy line index with caching
    /// 
    /// Protected by RwLock for thread-safe access.
    /// Write lock only needed when indexing new lines.
    line_index: RwLock<LineIndex>,
    
    /// Total file size in bytes
    /// 
    /// Cached for quick access
    file_size: u64,
    
    /// Path to the original file
    /// 
    /// Kept for error messages and potential re-opening
    path: PathBuf,
}

impl MmapFileAccessor {
    /// Create a memory-mapped accessor for a large file
    /// 
    /// # Arguments
    /// * `path` - Path to the file to map
    /// 
    /// # Returns
    /// * Ok(accessor) if file maps successfully
    /// * Err if file cannot be opened or mapped
    /// 
    /// # Performance
    /// * O(1) - Just sets up memory mapping, no scanning
    /// * Actual file data is loaded on demand by OS
    pub async fn new(path: &Path) -> Result<Self> {
        // Open file with tokio, then convert to std for mmap
        let file = tokio::fs::File::open(path).await
            .map_err(|e| RllessError::file_error("Failed to open file", e))?;
        
        let std_file = file.into_std().await;
        
        // Create memory mapping
        let mmap = unsafe {
            MmapOptions::new()
                .map(&std_file)
                .map_err(|e| RllessError::file_error("Failed to memory map file", e))?
        };
        
        // Advise kernel about our access pattern
        #[cfg(unix)]
        {
            // Sequential for initial scan, may change to random later
            mmap.advise(memmap2::Advice::Sequential)
                .map_err(|e| RllessError::file_error("Failed to set mmap advice", e))?;
        }
        
        let file_size = mmap.len() as u64;
        
        Ok(Self {
            mmap,
            line_index: RwLock::new(LineIndex::new()),
            file_size,
            path: path.to_owned(),
        })
    }
    
    /// Get line boundaries for a specific line
    /// 
    /// # Arguments
    /// * `line_number` - Line to get boundaries for
    /// 
    /// # Returns
    /// * Ok((start, end)) - Byte positions of line start and end
    /// * Err if line number is out of bounds
    /// 
    /// # Side Effects
    /// * May trigger line indexing if not yet indexed
    fn get_line_boundaries(&self, line_number: u64) -> Result<(usize, usize)> {
        // Ensure we have indexed up to this line
        {
            let mut index = self.line_index.write();
            index.ensure_indexed_to(&self.mmap[..], line_number);
        }
        
        let index = self.line_index.read();
        let line_offsets = index.get_line_offsets();
        
        // Check bounds
        if line_number >= line_offsets.len() as u64 - 1 {
            return Err(RllessError::line_out_of_bounds(line_number));
        }
        
        let start = line_offsets[line_number as usize] as usize;
        
        // Find end of line
        let end = if line_number + 1 < line_offsets.len() as u64 {
            // We know where next line starts
            (line_offsets[line_number as usize + 1] - 1) as usize
        } else {
            // Need to find the end by scanning
            drop(index); // Release lock before scanning
            
            memchr::memchr(b'\n', &self.mmap[start..])
                .map(|offset| start + offset)
                .unwrap_or(self.mmap.len())
        };
        
        Ok((start, end))
    }
}

#[async_trait]
impl FileAccessor for MmapFileAccessor {
    async fn read_line(&self, line_number: u64) -> Result<String> {
        // Check cache first
        {
            let mut index = self.line_index.write();
            if let Some(cached) = index.get_cached_line(line_number) {
                return Ok(cached);
            }
        }
        
        // Get line boundaries (may trigger indexing)
        let (start, end) = self.get_line_boundaries(line_number)?;
        
        // Extract line content with UTF-8 conversion
        let line_content = self.mmap[start..end].to_str_lossy().into_owned();
        
        // Cache the result
        {
            let mut index = self.line_index.write();
            index.cache_line(line_number, line_content.clone(), start as u64, end as u64);
        }
        
        Ok(line_content)
    }
    
    async fn read_lines_range(&self, start_line: u64, count: u64) -> Result<Vec<String>> {
        if count == 0 {
            return Ok(Vec::new());
        }
        
        // For small ranges, use individual reads (benefits from caching)
        if count <= 10 {
            let mut lines = Vec::with_capacity(count as usize);
            for line_num in start_line..start_line + count {
                match self.read_line(line_num).await {
                    Ok(line) => lines.push(line),
                    Err(_) => break, // End of file
                }
            }
            return Ok(lines);
        }
        
        // For larger ranges, use bulk processing
        let (range_start, _) = self.get_line_boundaries(start_line)?;
        
        // Ensure we have enough lines indexed
        {
            let mut index = self.line_index.write();
            index.ensure_indexed_to(&self.mmap[..], start_line + count);
        }
        
        // Calculate end position
        let index = self.line_index.read();
        let line_offsets = index.get_line_offsets();
        let end_line = (start_line + count).min(line_offsets.len() as u64 - 1);
        
        let range_end = if end_line < line_offsets.len() as u64 - 1 {
            line_offsets[end_line as usize] as usize
        } else {
            self.mmap.len()
        };
        
        drop(index);
        
        // Use bstr for efficient line iteration
        let range_data = &self.mmap[range_start..range_end];
        let lines: Vec<String> = range_data
            .lines()
            .take(count as usize)
            .map(|line| line.to_str_lossy().into_owned())
            .collect();
        
        Ok(lines)
    }
    
    async fn find_next_match(&self, start_line: u64, pattern: &str) -> Result<Option<MatchInfo>> {
        let mut current_line = start_line;
        
        // Search forward line by line
        loop {
            match self.read_line(current_line).await {
                Ok(line_content) => {
                    if let Some(match_start) = line_content.find(pattern) {
                        let byte_offset = self.get_line_boundaries(current_line)?.0 as u64;
                        
                        return Ok(Some(MatchInfo {
                            line_number: current_line,
                            byte_offset,
                            line_content,
                            match_start,
                            match_end: match_start + pattern.len(),
                        }));
                    }
                    current_line += 1;
                }
                Err(e) if e.is_line_out_of_bounds() => {
                    return Ok(None); // End of file
                }
                Err(e) => return Err(e),
            }
            
            // Safety limit for very large files
            if current_line > start_line + 1_000_000 {
                return Ok(None);
            }
        }
    }
    
    async fn find_prev_match(&self, start_line: u64, pattern: &str) -> Result<Option<MatchInfo>> {
        if start_line == 0 {
            return Ok(None);
        }
        
        let mut current_line = start_line - 1;
        
        // Search backward
        loop {
            match self.read_line(current_line).await {
                Ok(line_content) => {
                    if let Some(match_start) = line_content.find(pattern) {
                        let byte_offset = self.get_line_boundaries(current_line)?.0 as u64;
                        
                        return Ok(Some(MatchInfo {
                            line_number: current_line,
                            byte_offset,
                            line_content,
                            match_start,
                            match_end: match_start + pattern.len(),
                        }));
                    }
                    
                    if current_line == 0 {
                        break;
                    }
                    current_line -= 1;
                }
                Err(_) => break,
            }
        }
        
        Ok(None)
    }
    
    fn file_size(&self) -> u64 {
        self.file_size
    }
    
    fn total_lines(&self) -> Option<u64> {
        let index = self.line_index.read();
        
        // Only provide estimate if we've indexed significant portion
        if index.indexed_byte_count() > self.file_size / 2 {
            // Extrapolate based on lines found so far
            let lines_so_far = index.indexed_line_count();
            let bytes_indexed = index.indexed_byte_count();
            let estimated_total = lines_so_far * self.file_size / bytes_indexed;
            Some(estimated_total)
        } else {
            None // Not enough data to estimate reliably
        }
    }
    
    fn supports_parallel(&self) -> bool {
        true // Memory mapping enables efficient parallel access
    }
}
```

### 5. FileAccessorFactory

```rust
// src/file_handler/factory.rs

use crate::error::Result;
use crate::file_handler::accessor::FileAccessor;
use crate::file_handler::in_memory::InMemoryFileAccessor;
use crate::file_handler::mmap::MmapFileAccessor;
use std::path::Path;

/// Factory for creating appropriate FileAccessor based on file characteristics
pub struct FileAccessorFactory;

impl FileAccessorFactory {
    /// Size threshold for choosing between in-memory and mmap
    /// 
    /// Files smaller than this are loaded entirely into memory.
    /// Files larger than this use memory mapping.
    const SMALL_FILE_THRESHOLD: u64 = 10 * 1024 * 1024; // 10MB
    
    /// Platform-specific threshold for macOS
    /// 
    /// macOS has different mmap performance characteristics,
    /// so we use a larger threshold.
    #[cfg(target_os = "macos")]
    const MACOS_THRESHOLD: u64 = 50 * 1024 * 1024; // 50MB
    
    /// Create the appropriate FileAccessor for a given file
    /// 
    /// # Arguments
    /// * `path` - Path to the file to open
    /// 
    /// # Returns
    /// * Box<dyn FileAccessor> - Appropriate implementation
    /// 
    /// # Strategy
    /// - Files < 10MB (50MB on macOS): InMemoryFileAccessor
    /// - Files ≥ 10MB: MmapFileAccessor
    /// - Future: Compressed files will get special handling
    pub async fn create(path: &Path) -> Result<Box<dyn FileAccessor>> {
        // Get file metadata
        let metadata = tokio::fs::metadata(path).await
            .map_err(|e| RllessError::file_error("Failed to get file metadata", e))?;
        
        let file_size = metadata.len();
        
        // Choose threshold based on platform
        let threshold = Self::get_threshold();
        
        // Select implementation based on file size
        if file_size < threshold {
            // Small file: load into memory
            let accessor = InMemoryFileAccessor::new(path).await?;
            Ok(Box::new(accessor))
        } else {
            // Large file: use memory mapping
            let accessor = MmapFileAccessor::new(path).await?;
            Ok(Box::new(accessor))
        }
    }
    
    /// Get the size threshold for the current platform
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
    
    /// Create with explicit strategy (for testing)
    #[cfg(test)]
    pub async fn create_with_strategy(
        path: &Path,
        force_mmap: bool,
    ) -> Result<Box<dyn FileAccessor>> {
        if force_mmap {
            Ok(Box::new(MmapFileAccessor::new(path).await?))
        } else {
            Ok(Box::new(InMemoryFileAccessor::new(path).await?))
        }
    }
}
```

## Implementation Tasks

### Phase 1: Core Infrastructure (2-3 hours)
1. **Add dependencies to Cargo.toml**
   - memchr, bstr, parking_lot, memmap2
2. **Update FileAccessor trait in accessor.rs**
   - Add MatchInfo struct
   - Add async trait methods with documentation

### Phase 2: LineIndex Implementation (2 hours)
3. **Create line_index.rs module**
   - Implement LineIndex with SIMD support
   - Add caching logic
   - Write unit tests

### Phase 3: File Accessor Implementations (4 hours)
4. **Implement InMemoryFileAccessor**
   - Load file and pre-compute lines
   - Implement all trait methods
   - Add unit tests

5. **Implement MmapFileAccessor**
   - Set up memory mapping
   - Integrate with LineIndex
   - Implement all trait methods
   - Add unit tests

### Phase 4: Factory and Integration (1 hour)
6. **Create FileAccessorFactory**
   - Implement strategy selection
   - Platform-specific thresholds
   - Integration tests

### Phase 5: Testing and Benchmarking (3 hours)
7. **Comprehensive testing**
   - Unit tests for each component
   - Integration tests for factory
   - Edge cases and error conditions

8. **Performance benchmarks**
   - Line access latency
   - Search performance
   - Memory usage validation

## Performance Targets

### InMemoryFileAccessor (files < 10MB)
- **Load time**: < 50ms
- **Line access**: < 0.01ms
- **Search**: < 10ms for full file
- **Memory usage**: File size + ~10% overhead

### MmapFileAccessor (files ≥ 10MB)
- **Open time**: < 1ms
- **First line access**: < 5ms (includes indexing)
- **Cached line access**: < 0.1ms
- **Search**: < 100ms per million lines
- **Memory usage**: ~8 bytes per indexed line + 100 line cache

## Testing Strategy

### Unit Tests
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;
    
    #[tokio::test]
    async fn test_small_file_reads() {
        // Create test file
        let file = create_test_file("line1\nline2\nline3\n");
        let accessor = InMemoryFileAccessor::new(file.path()).await.unwrap();
        
        // Test line access
        assert_eq!(accessor.read_line(0).await.unwrap(), "line1");
        assert_eq!(accessor.read_line(1).await.unwrap(), "line2");
        
        // Test range read
        let lines = accessor.read_lines_range(0, 2).await.unwrap();
        assert_eq!(lines, vec!["line1", "line2"]);
    }
    
    #[tokio::test]
    async fn test_large_file_lazy_indexing() {
        // Create large test file
        let file = create_large_test_file(1_000_000); // 1M lines
        let accessor = MmapFileAccessor::new(file.path()).await.unwrap();
        
        // First access should trigger indexing
        let start = Instant::now();
        let line = accessor.read_line(1000).await.unwrap();
        assert!(start.elapsed() < Duration::from_millis(10));
        
        // Second access should be cached
        let start = Instant::now();
        let line = accessor.read_line(1000).await.unwrap();
        assert!(start.elapsed() < Duration::from_millis(1));
    }
}
```

### Integration Tests
```rust
#[tokio::test]
async fn test_factory_selects_correct_implementation() {
    // Small file
    let small_file = create_test_file_with_size(5 * 1024 * 1024); // 5MB
    let accessor = FileAccessorFactory::create(small_file.path()).await.unwrap();
    assert!(accessor.total_lines().is_some()); // InMemory always knows total
    
    // Large file
    let large_file = create_test_file_with_size(20 * 1024 * 1024); // 20MB
    let accessor = FileAccessorFactory::create(large_file.path()).await.unwrap();
    assert!(accessor.supports_parallel()); // Mmap supports parallel
}
```

## Success Criteria

- ✅ All trait methods implemented for both accessors
- ✅ SIMD line finding via memchr
- ✅ Lazy indexing for large files
- ✅ LRU cache for frequently accessed lines
- ✅ Consistent interface regardless of file size
- ✅ Performance targets met
- ✅ Memory usage within bounds
- ✅ All tests passing

This design provides a solid foundation for Phase 1 Task 4, with clear separation of concerns and optimal performance for both small and large files.