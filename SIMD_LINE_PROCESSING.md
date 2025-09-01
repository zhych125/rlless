# SIMD Line Processing Architecture for rlless

This document outlines the architecture for efficient line boundary detection and parallel processing in rlless, based on ripgrep's proven approach.

## Crate Selection Strategy

### Primary: `memchr` - SIMD Line Finding
- **Purpose**: Finding individual newline bytes (`\n`) with SIMD optimization
- **Performance**: ~10GB/s throughput on modern CPUs with AVX2/SSE2
- **Use case**: Core line boundary detection, chunk splitting
- **Why primary**: Best performance for finding single bytes, battle-tested in ripgrep

### Complementary: `bstr` - Line Processing Utilities
- **Purpose**: Higher-level line operations built on memchr
- **Use cases**: 
  - `.lines_with_terminator()` - Clean line iteration with newline preservation
  - `.to_str_lossy()` - Graceful UTF-8 handling for invalid sequences
  - Line-oriented processing without manual boundary management
- **Why complementary**: Provides convenience and correctness on top of memchr's raw speed

### Optional: `bytecount` - Bulk Line Counting
- **Purpose**: SIMD-optimized counting of newlines when you only need totals
- **Use case**: Getting total line count without needing individual positions
- **When to use**: Only if frequent line counting is needed

## Dependencies

```toml
[dependencies]
memchr = "2.7"        # SIMD-optimized byte search
bstr = "1.9"          # Byte string utilities with line iteration  
memmap2 = "0.9"       # Memory mapping
parking_lot = "0.12"  # Fast RwLock for line index
rayon = "1.8"         # Future parallel processing

[features]
default = ["simd"]
simd = ["memchr/std"]  # Enable SIMD optimizations
```

## Architecture Overview

```
┌─────────────────────────────────────┐
│            Application              │
├─────────────────────────────────────┤
│      FileAccessor Trait             │  ← Single interface
├─────────────────────────────────────┤
│  FileAccessorFactory (Strategy)     │  ← Chooses implementation
├─────────────────────────────────────┤
│ ┌─────────────┐ ┌─────────────────┐ │
│ │MmapAccessor │ │BufferedAccessor │ │  ← Different strategies
│ └─────────────┘ └─────────────────┘ │
├─────────────────────────────────────┤
│        LineIndex (SIMD)             │  ← memchr for line finding
├─────────────────────────────────────┤
│    Compression Layer (Optional)     │  ← Transparent compression
├─────────────────────────────────────┤
│      File System / Memory Map       │  ← Storage layer
└─────────────────────────────────────┘
```

## Core Implementation

### 1. SIMD-Optimized Line Index

```rust
use memchr::{memchr, memrchr};
use parking_lot::RwLock;

/// SIMD-optimized line boundary finder
pub struct LineIndex {
    /// Cached line start positions (byte offsets)
    line_starts: Vec<u64>,
    /// How far we've indexed (byte position)
    indexed_to_byte: u64,
    /// How many lines we've indexed
    indexed_to_line: u64,
}

impl LineIndex {
    /// Find line boundaries using SIMD memchr
    pub fn index_up_to_line(&mut self, data: &[u8], target_line: u64) {
        if target_line <= self.indexed_to_line {
            return; // Already indexed
        }
        
        let mut pos = self.indexed_to_byte as usize;
        
        // Use memchr for SIMD-optimized newline search
        while pos < data.len() {
            match memchr(b'\n', &data[pos..]) {
                Some(relative_pos) => {
                    pos += relative_pos + 1;
                    self.line_starts.push(pos as u64);
                    self.indexed_to_line += 1;
                    
                    if self.indexed_to_line >= target_line {
                        break;
                    }
                }
                None => break,
            }
        }
        
        self.indexed_to_byte = pos as u64;
    }
    
    /// Get line boundaries for parallel chunk processing
    pub fn find_chunk_boundaries(&self, data: &[u8], chunk_size: usize) -> Vec<usize> {
        let mut boundaries = Vec::new();
        let mut pos = 0;
        
        while pos < data.len() {
            let target = (pos + chunk_size).min(data.len());
            
            // Find nearest line boundary using memchr
            // Search forward up to 1KB for a newline to avoid tiny chunks
            let search_end = (target + 1024).min(data.len());
            if let Some(newline_pos) = memchr(b'\n', &data[target..search_end]) {
                boundaries.push(target + newline_pos + 1);
                pos = target + newline_pos + 1;
            } else {
                boundaries.push(data.len());
                break;
            }
        }
        
        boundaries
    }
}
```

### 2. Strategy Selection

```rust
pub struct FileAccessorFactory;

impl FileAccessorFactory {
    pub async fn create(path: &Path) -> Result<Box<dyn FileAccessor>> {
        let metadata = tokio::fs::metadata(path).await?;
        let file_size = metadata.len();
        
        let strategy = Self::select_strategy(file_size, path)?;
        
        match strategy {
            AccessStrategy::MemoryMapped => {
                let mmap_accessor = MmapFileAccessor::new(path).await?;
                Ok(Box::new(mmap_accessor))
            }
            AccessStrategy::Buffered => {
                let buffered_accessor = BufferedFileAccessor::new(path).await?;
                Ok(Box::new(buffered_accessor))
            }
            AccessStrategy::Compressed(compression_type) => {
                let compressed_accessor = CompressedFileAccessor::new(path, compression_type).await?;
                Ok(Box::new(compressed_accessor))
            }
        }
    }
    
    fn select_strategy(file_size: u64, path: &Path) -> Result<AccessStrategy> {
        // First check compression
        let compression_type = detect_compression(path)?;
        if !matches!(compression_type, CompressionType::None) {
            return Ok(AccessStrategy::Compressed(compression_type));
        }
        
        // For uncompressed files, choose based on ripgrep's heuristics
        if cfg!(target_os = "macos") && file_size > 100 * 1024 * 1024 {
            // On macOS, prefer buffered for large files (ripgrep's approach)
            Ok(AccessStrategy::Buffered)
        } else if file_size < 10 * 1024 * 1024 {
            // Small files: buffered is fine and has less overhead
            Ok(AccessStrategy::Buffered)
        } else {
            // Large files on non-macOS: memory mapping is beneficial
            Ok(AccessStrategy::MemoryMapped)
        }
    }
}

pub enum AccessStrategy {
    MemoryMapped,
    Buffered,
    Compressed(CompressionType),
}
```

### 3. Memory-Mapped Implementation

```rust
use memmap2::{Mmap, MmapOptions};
use bstr::ByteSlice;

pub struct MmapFileAccessor {
    mmap: Mmap,
    line_index: RwLock<LineIndex>,
    file_size: u64,
}

impl MmapFileAccessor {
    pub async fn new(path: &Path) -> Result<Self> {
        let file = File::open(path)?;
        let mmap = unsafe { MmapOptions::new().map(&file)? };
        
        // Advise kernel for our access pattern
        #[cfg(unix)]
        mmap.advise(memmap2::Advice::Sequential)?;
        
        Ok(Self {
            file_size: mmap.len() as u64,
            mmap,
            line_index: RwLock::new(LineIndex {
                line_starts: vec![0], // First line starts at byte 0
                indexed_to_byte: 0,
                indexed_to_line: 0,
            }),
        })
    }
}

#[async_trait]
impl FileAccessor for MmapFileAccessor {
    async fn read_line(&self, line_number: u64) -> Result<String> {
        // Lazy indexing with SIMD optimization
        {
            let mut index = self.line_index.write();
            index.index_up_to_line(&self.mmap[..], line_number + 1);
        }
        
        let index = self.line_index.read();
        if line_number >= index.line_starts.len() as u64 {
            return Err(RllessError::LineOutOfBounds);
        }
        
        let start = index.line_starts[line_number as usize] as usize;
        drop(index); // Release lock early
        
        // Find line end using SIMD memchr
        let end = memchr(b'\n', &self.mmap[start..])
            .map(|pos| start + pos)
            .unwrap_or(self.mmap.len());
        
        // Use bstr for graceful UTF-8 handling
        Ok(self.mmap[start..end].to_str_lossy().into_owned())
    }
    
    async fn read_lines_range(&self, start_line: u64, count: u64) -> Result<Vec<String>> {
        // Efficient range reading using bstr's line iterator
        let data = &self.mmap[..];
        
        // Use bstr for clean line iteration with terminator handling
        let lines: Vec<String> = data
            .lines_with_terminator()
            .skip(start_line as usize)
            .take(count as usize)
            .map(|line| line.to_str_lossy().trim_end().to_owned())
            .collect();
        
        Ok(lines)
    }
    
    fn file_size(&self) -> u64 {
        self.file_size
    }
    
    fn total_lines(&self) -> Option<u64> {
        // Could implement with bytecount for fast counting if needed
        None
    }
    
    // Future: Parallel processing support
    fn as_bytes(&self) -> Option<&[u8]> {
        Some(&self.mmap[..])
    }
}
```

## Parallel Processing Design (Future)

### Chunk-Based Parallel Search

```rust
impl MmapFileAccessor {
    /// Future: Parallel search implementation
    pub fn parallel_search<F>(&self, pattern: F) -> Result<Vec<SearchMatch>> 
    where 
        F: Fn(&[u8]) -> Vec<SearchMatch> + Send + Sync,
    {
        let data = self.as_bytes().unwrap();
        let index = self.line_index.read();
        
        // Split into 64MB chunks aligned on line boundaries
        let boundaries = index.find_chunk_boundaries(data, 64 * 1024 * 1024);
        
        // Use rayon for parallel processing
        let results: Vec<SearchMatch> = boundaries
            .par_windows(2)
            .flat_map(|window| {
                let chunk = &data[window[0]..window[1]];
                pattern(chunk)
            })
            .collect();
        
        Ok(results)
    }
}

#[derive(Debug)]
pub struct SearchMatch {
    pub line_number: u64,
    pub byte_offset: u64,
    pub content: String,
}
```

### Parallel Line Counting

```rust
use bytecount;

impl MmapFileAccessor {
    /// Fast parallel line counting using SIMD
    pub fn count_total_lines(&self) -> u64 {
        if let Some(data) = self.as_bytes() {
            // Use bytecount for SIMD-optimized counting
            bytecount::count(data, b'\n') as u64
        } else {
            0
        }
    }
    
    /// Count lines in parallel chunks
    pub fn parallel_count_lines(&self) -> u64 {
        let data = self.as_bytes().unwrap();
        let chunk_size = 64 * 1024 * 1024; // 64MB chunks
        
        (0..data.len())
            .step_by(chunk_size)
            .into_par_iter()
            .map(|start| {
                let end = (start + chunk_size).min(data.len());
                bytecount::count(&data[start..end], b'\n') as u64
            })
            .sum()
    }
}
```

## Performance Characteristics

### Expected Performance
- **Line indexing**: ~10GB/s with SIMD (AVX2)
- **Line access**: O(1) after indexing
- **Memory overhead**: ~8 bytes per line for index
- **Startup time**: Near-instantaneous (lazy indexing)

### Benchmarking Strategy
```rust
#[cfg(test)]
mod benchmarks {
    use criterion::{criterion_group, criterion_main, Criterion};
    
    fn bench_line_finding(c: &mut Criterion) {
        let data = include_bytes!("../test_data/large_log.txt");
        
        c.bench_function("memchr_line_finding", |b| {
            b.iter(|| {
                let mut pos = 0;
                let mut count = 0;
                while let Some(newline_pos) = memchr(b'\n', &data[pos..]) {
                    pos += newline_pos + 1;
                    count += 1;
                }
                count
            });
        });
    }
    
    criterion_group!(benches, bench_line_finding);
    criterion_main!(benches);
}
```

## Migration Path

### Phase 1: Current Task 4 Implementation
- Implement basic MmapFileAccessor with lazy indexing using memchr
- Add bstr for convenient line iteration in read_lines_range
- Keep interface compatible with current FileAccessor trait

### Phase 2: Future Parallel Support
- Add optional parallel methods to FileAccessor trait
- Implement chunk-based parallel search
- Add rayon dependency and parallel algorithms

### Phase 3: Advanced Optimizations  
- Add bytecount for bulk line counting
- Implement memory pressure detection
- Add SIMD feature flags and runtime detection

## Key Design Decisions

### Why This Approach?
1. **memchr primary**: Raw SIMD speed for the critical path (finding newlines)
2. **bstr complementary**: Higher-level correctness and convenience
3. **Lazy indexing**: Don't pay upfront cost for large files
4. **Chunk alignment**: Essential for future parallel processing
5. **Platform awareness**: Different strategies for macOS vs Linux (ripgrep's wisdom)

### Trade-offs Made
- **Memory vs CPU**: Cache line positions for faster access
- **Complexity vs Performance**: SIMD optimization worth the dependency
- **Lazy vs Eager**: Index on-demand rather than upfront scanning  
- **Single-threaded first**: Build solid foundation before parallelization

## Future Considerations

### Potential Optimizations
- **Compressed line indices**: For very large files with many lines
- **Memory mapping advice**: More sophisticated madvise usage
- **NUMA awareness**: For very large multi-socket systems
- **Streaming hybrid**: Combine mmap with streaming for massive files

### Monitoring and Metrics
- Track line index cache hit rates
- Measure SIMD vs scalar performance on different CPUs  
- Monitor memory pressure and strategy selection effectiveness