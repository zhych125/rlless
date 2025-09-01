# Sequential vs Parallel File Access Design for rlless

This document outlines the architectural decision to separate sequential and parallel file access interfaces in rlless, optimizing for different use cases and performance characteristics.

## Design Philosophy

### Core Principle: Different Use Cases Require Different APIs

rlless is primarily an **interactive log viewer** where users navigate, search, and examine log files. The access patterns and performance requirements for interactive viewing are fundamentally different from batch processing operations.

### User Interaction Patterns Analysis

#### Interactive Log Viewing (99% of usage)
```rust
// What users actually do in rlless
viewer.jump_to_line(12045)?;           // Navigate to specific line
viewer.read_lines_around(12045, 50)?;  // Show context around current position
viewer.scroll_down(10)?;                // Navigate through file
viewer.search_from_current("ERROR")?;  // Find next occurrence
viewer.filter_visible("timeout")?;     // Filter current view
```

**Characteristics:**
- **Low latency required** - User expects immediate response
- **Small data volumes** - Usually <1000 lines visible at once
- **Stateful navigation** - Current position, search context matter
- **Frequent random access** - Jump to any line number
- **Responsive UI** - Cannot block interface

#### Batch Processing (1% of usage, power features)
```rust
// Advanced operations that benefit from parallelism  
processor.count_pattern_matches("ERROR")?;     // Analyze entire log
processor.find_all_matches("timeout")?;        // Build complete index
processor.export_filtered_lines(filter)?;     // Extract subset
processor.generate_statistics()?;              // Full file analysis
```

**Characteristics:**
- **High throughput required** - Process entire file efficiently
- **Large data volumes** - Process gigabytes of data
- **Stateless operations** - Each operation independent
- **Sequential processing acceptable** - User can wait for results
- **Resource intensive** - Can use all CPU cores

## Interface Design Strategy

### Sequential Interface: Optimized for Interactive Use

```rust
/// Primary trait for interactive log viewing
/// Optimized for low latency, random access, and user responsiveness
#[async_trait]
pub trait FileAccessor: Send + Sync {
    // === Core Navigation ===
    /// Read a specific line by number (0-based)
    /// Returns immediately using line index cache
    async fn read_line(&self, line_number: u64) -> Result<String>;
    
    /// Read a range of lines efficiently
    /// Optimized for showing context around current position
    async fn read_lines_range(&self, start: u64, count: u64) -> Result<Vec<String>>;
    
    // === Interactive Search ===
    /// Find next match starting from a line
    /// Returns quickly by scanning forward incrementally
    async fn find_next_match(&self, start_line: u64, pattern: &str) -> Result<Option<MatchInfo>>;
    
    /// Find previous match searching backward
    async fn find_prev_match(&self, start_line: u64, pattern: &str) -> Result<Option<MatchInfo>>;
    
    // === File Information ===
    /// Get total file size in bytes
    fn file_size(&self) -> u64;
    
    /// Get total line count if known (may be None for very large files)
    fn total_lines(&self) -> Option<u64>;
    
    /// Check if parallel processing is available
    fn supports_parallel(&self) -> bool { false }
}

#[derive(Debug, Clone)]
pub struct MatchInfo {
    pub line_number: u64,
    pub byte_offset: u64,
    pub line_content: String,
    pub match_start: usize,  // Character offset within line
    pub match_end: usize,
}
```

### Parallel Interface: Optimized for Batch Processing

```rust
/// Extension trait for bulk operations that benefit from parallelism
/// These operations process the entire file and may take significant time
pub trait ParallelFileProcessor: Send + Sync {
    // === Pattern Analysis ===
    /// Count total occurrences of pattern in entire file
    /// Uses parallel chunking for maximum throughput
    fn count_matches(&self, pattern: &str) -> Result<u64>;
    
    /// Find all matches in the entire file
    /// Returns complete index for fast subsequent searches
    fn find_all_matches(&self, pattern: &str) -> Result<Vec<MatchInfo>>;
    
    /// Build regex matches across entire file
    fn find_all_regex_matches(&self, regex: &regex::Regex) -> Result<Vec<MatchInfo>>;
    
    // === Bulk Operations ===
    /// Filter entire file and return matching lines
    /// Useful for exporting subsets of large logs
    fn filter_lines<F>(&self, predicate: F) -> Result<Vec<FilteredLine>>
    where F: Fn(&str) -> bool + Send + Sync;
    
    /// Generate statistics about the entire file
    /// Line counts, patterns, error frequencies, etc.
    fn generate_statistics(&self) -> Result<FileStatistics>;
    
    /// Export filtered content to a new file
    /// Efficiently processes and writes large subsets
    fn export_filtered<F>(&self, output_path: &Path, predicate: F) -> Result<ExportStats>
    where F: Fn(&str) -> bool + Send + Sync;
}

#[derive(Debug)]
pub struct FilteredLine {
    pub original_line_number: u64,
    pub content: String,
}

#[derive(Debug)]
pub struct FileStatistics {
    pub total_lines: u64,
    pub total_bytes: u64,
    pub pattern_counts: HashMap<String, u64>,
    pub line_length_stats: LengthStats,
    pub processing_time: Duration,
}

#[derive(Debug)]
pub struct ExportStats {
    pub lines_processed: u64,
    pub lines_exported: u64,
    pub bytes_written: u64,
    pub processing_time: Duration,
}
```

## Implementation Strategy

### Single Implementation, Multiple Interfaces

```rust
/// Core implementation supporting both sequential and parallel access
pub struct MmapFileAccessor {
    mmap: Mmap,
    line_index: RwLock<LineIndex>,
    file_size: u64,
    path: PathBuf,
}

impl MmapFileAccessor {
    pub async fn new(path: &Path) -> Result<Self> {
        let file = File::open(path)?;
        let mmap = unsafe { MmapOptions::new().map(&file)? };
        
        #[cfg(unix)]
        mmap.advise(memmap2::Advice::Sequential)?;
        
        Ok(Self {
            file_size: mmap.len() as u64,
            mmap,
            line_index: RwLock::new(LineIndex::new()),
            path: path.to_owned(),
        })
    }
    
    /// Internal method for parallel chunk processing
    fn parallel_process<F, R>(&self, chunk_processor: F) -> Result<Vec<R>>
    where
        F: Fn(&[u8], usize) -> Vec<R> + Send + Sync,
        R: Send,
    {
        let chunks = self.create_line_aligned_chunks(64 * 1024 * 1024);
        
        chunks
            .par_iter()
            .flat_map(|chunk| {
                let chunk_data = &self.mmap[chunk.start..chunk.end];
                chunk_processor(chunk_data, chunk.start)
            })
            .collect()
    }
}

// Sequential interface implementation
#[async_trait]
impl FileAccessor for MmapFileAccessor {
    async fn read_line(&self, line_number: u64) -> Result<String> {
        // Optimized for single line access with line index
        self.index_up_to_line(line_number + 1).await;
        
        let index = self.line_index.read();
        if line_number >= index.line_starts.len() as u64 {
            return Err(RllessError::LineOutOfBounds);
        }
        
        let start = index.line_starts[line_number as usize] as usize;
        drop(index);
        
        let end = memchr(b'\n', &self.mmap[start..])
            .map(|pos| start + pos)
            .unwrap_or(self.mmap.len());
        
        Ok(self.mmap[start..end].to_str_lossy().into_owned())
    }
    
    async fn find_next_match(&self, start_line: u64, pattern: &str) -> Result<Option<MatchInfo>> {
        // Optimized for incremental search from current position
        let mut current_line = start_line;
        
        loop {
            match self.read_line(current_line).await {
                Ok(line) => {
                    if let Some(match_pos) = line.find(pattern) {
                        return Ok(Some(MatchInfo {
                            line_number: current_line,
                            byte_offset: self.get_line_byte_offset(current_line)?,
                            line_content: line,
                            match_start: match_pos,
                            match_end: match_pos + pattern.len(),
                        }));
                    }
                    current_line += 1;
                }
                Err(_) => return Ok(None), // End of file
            }
        }
    }
    
    fn supports_parallel(&self) -> bool {
        true
    }
}

// Parallel interface implementation  
impl ParallelFileProcessor for MmapFileAccessor {
    fn count_matches(&self, pattern: &str) -> Result<u64> {
        let pattern = pattern.to_owned();
        let total: u64 = self.parallel_process(|chunk_data, _offset| {
            chunk_data
                .lines()
                .map(|line| line.matches(&pattern).count() as u64)
                .sum::<u64>()
        })?.into_iter().sum();
        
        Ok(total)
    }
    
    fn find_all_matches(&self, pattern: &str) -> Result<Vec<MatchInfo>> {
        let pattern = pattern.to_owned();
        self.parallel_process(move |chunk_data, chunk_offset| {
            let mut matches = Vec::new();
            
            for (relative_line, line) in chunk_data.lines().enumerate() {
                if let Some(match_pos) = line.find(&pattern) {
                    matches.push(MatchInfo {
                        line_number: relative_line as u64, // Would need global line calculation
                        byte_offset: chunk_offset as u64,
                        line_content: line.to_owned(),
                        match_start: match_pos,
                        match_end: match_pos + pattern.len(),
                    });
                }
            }
            
            matches
        })
    }
    
    fn filter_lines<F>(&self, predicate: F) -> Result<Vec<FilteredLine>>
    where F: Fn(&str) -> bool + Send + Sync
    {
        self.parallel_process(move |chunk_data, _chunk_offset| {
            chunk_data
                .lines()
                .enumerate()
                .filter_map(|(relative_line, line)| {
                    if predicate(line) {
                        Some(FilteredLine {
                            original_line_number: relative_line as u64, // Global calculation needed
                            content: line.to_owned(),
                        })
                    } else {
                        None
                    }
                })
                .collect()
        })
    }
}
```

## Performance Characteristics

### Sequential Interface Performance
| Operation | Expected Latency | Optimization |
|-----------|------------------|--------------|
| `read_line()` | <1ms | Line index cache, SIMD boundary detection |
| `read_lines_range()` | <10ms for 100 lines | Bulk memcpy, minimal string allocation |
| `find_next_match()` | <50ms typical | Incremental search, early termination |
| `find_prev_match()` | <50ms typical | Backward search with line index |

### Parallel Interface Performance  
| Operation | Expected Throughput | Resource Usage |
|-----------|-------------------|----------------|
| `count_matches()` | ~2GB/s | All CPU cores, minimal memory |
| `find_all_matches()` | ~1GB/s | All CPU cores, result collection overhead |
| `filter_lines()` | ~500MB/s | All CPU cores, string allocation heavy |
| `export_filtered()` | ~200MB/s | I/O bound, streaming output |

## Usage Examples

### Interactive Log Viewing Session
```rust
#[tokio::main]
async fn main() -> Result<()> {
    let accessor = FileAccessorFactory::create("large.log").await?;
    
    // User jumps to a specific line
    let line = accessor.read_line(50000).await?;
    println!("Line 50000: {}", line);
    
    // Show context around current position
    let context = accessor.read_lines_range(49990, 20).await?;
    display_lines_with_numbers(context, 49990);
    
    // User searches for next error
    if let Some(match_info) = accessor.find_next_match(50000, "ERROR").await? {
        println!("Found ERROR at line {}: {}", 
            match_info.line_number, match_info.line_content);
    }
    
    Ok(())
}
```

### Batch Processing Session
```rust
#[tokio::main] 
async fn main() -> Result<()> {
    let accessor = FileAccessorFactory::create("large.log").await?;
    
    // Check if parallel processing is available
    if accessor.supports_parallel() {
        let processor = accessor as &dyn ParallelFileProcessor;
        
        // Generate comprehensive statistics
        println!("Analyzing entire log file...");
        let stats = processor.generate_statistics()?;
        println!("Total lines: {}, Total errors: {}", 
            stats.total_lines, stats.pattern_counts.get("ERROR").unwrap_or(&0));
        
        // Export all error lines
        processor.export_filtered("errors.log", |line| line.contains("ERROR"))?;
        println!("Error lines exported to errors.log");
    }
    
    Ok(())
}
```

## Testing Strategy

### Sequential Interface Testing
```rust
#[cfg(test)]
mod sequential_tests {
    use super::*;
    
    #[tokio::test]
    async fn test_random_line_access() {
        let accessor = create_test_accessor().await;
        
        // Test random access performance
        let start = Instant::now();
        for _ in 0..1000 {
            let line_num = rand::random::<u64>() % 10000;
            let _line = accessor.read_line(line_num).await?;
        }
        let duration = start.elapsed();
        assert!(duration < Duration::from_millis(100)); // <0.1ms per access
    }
    
    #[tokio::test]
    async fn test_incremental_search() {
        let accessor = create_test_accessor().await;
        
        let mut current_line = 0;
        let mut matches_found = 0;
        
        while let Some(match_info) = accessor.find_next_match(current_line, "ERROR").await? {
            matches_found += 1;
            current_line = match_info.line_number + 1;
            
            if matches_found >= 10 { break; } // Limit test
        }
        
        assert!(matches_found > 0);
    }
}
```

### Parallel Interface Testing
```rust
#[cfg(test)]
mod parallel_tests {
    use super::*;
    
    #[test]
    fn test_parallel_count_accuracy() {
        let processor = create_test_processor();
        
        // Count using parallel algorithm
        let parallel_count = processor.count_matches("ERROR")?;
        
        // Verify with sequential count
        let sequential_count = count_matches_sequentially("ERROR")?;
        
        assert_eq!(parallel_count, sequential_count);
    }
    
    #[test]
    fn test_parallel_performance() {
        let processor = create_large_test_processor(); // >1GB test file
        
        let start = Instant::now();
        let matches = processor.find_all_matches("ERROR")?;
        let duration = start.elapsed();
        
        // Should be faster than sequential for large files
        assert!(duration < Duration::from_secs(30));
        assert!(matches.len() > 0);
    }
}
```

## Migration Path

### Phase 1: Sequential-First Implementation
1. **Implement `FileAccessor` trait** with focus on interactive performance
2. **Optimize line access** using SIMD line indexing
3. **Add incremental search** for responsive UI
4. **Test with real log files** to validate latency requirements

### Phase 2: Add Parallel Capabilities
1. **Implement `ParallelFileProcessor`** on same underlying implementation
2. **Add chunk-based processing** using rayon and memchr
3. **Optimize for throughput** rather than latency
4. **Add comprehensive testing** for accuracy and performance

### Phase 3: Advanced Features
1. **Smart interface selection** - automatically choose best interface for operation
2. **Hybrid operations** - start sequential, switch to parallel for large results
3. **Progress reporting** for long-running parallel operations
4. **Memory pressure adaptation** for very large files

## Design Benefits

### Clear Separation of Concerns
- **Sequential interface**: Optimized for user experience and responsiveness
- **Parallel interface**: Optimized for computational throughput and efficiency
- **No performance compromise**: Each interface can be optimized independently

### Future Flexibility
- **Easy to add new operations** to either interface without affecting the other
- **Clear testing strategy** with different performance expectations
- **Incremental implementation** - start with sequential, add parallel later

### User Experience
- **Predictable performance** - users know what to expect from each operation
- **No hidden costs** - parallel operations are explicitly different
- **Responsive UI** - interactive operations never block on background processing

## Key Design Decisions

### Why Separate Interfaces?
1. **Different performance profiles** require different optimization strategies
2. **Different resource requirements** - sequential is lightweight, parallel is heavy
3. **Different error handling** - interactive errors vs batch processing errors
4. **Clear testing boundaries** with distinct performance expectations

### Why Same Implementation?
1. **Code reuse** - underlying file access logic is shared
2. **Consistency** - same data source ensures consistent results
3. **Resource sharing** - line index cache benefits both interfaces
4. **Maintenance simplicity** - one implementation to optimize and debug

This design provides the foundation for both excellent interactive performance and powerful batch processing capabilities while maintaining clear architectural boundaries.