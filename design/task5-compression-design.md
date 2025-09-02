# Task 5: Compression Support Design

## Overview
Add transparent compression support to rlless, allowing it to read compressed log files without manual extraction. This is critical for production use where logs are commonly compressed to save disk space.

## Design Goals
1. **Transparent Operation**: Users shouldn't need to know if a file is compressed
2. **Streaming Decompression**: Avoid loading entire decompressed file into memory
3. **Performance**: Maintain responsiveness even with large compressed files
4. **Extensibility**: Easy to add new compression formats
5. **Zero-Copy Where Possible**: Minimize memory allocations during decompression

## Architecture

### Core Components

```rust
// src/compression.rs - Main compression module

/// Supported compression formats
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CompressionType {
    None,
    Gzip,
    Bzip2,
    Xz,
    Zstd,
}

/// Trait for compression format adapters
#[async_trait]
pub trait CompressionAdapter: Send + Sync {
    /// Get the compression type
    fn compression_type(&self) -> CompressionType;
    
    /// Create a decoder for reading compressed data
    async fn create_decoder(&self, source: Box<dyn AsyncRead + Unpin + Send>) 
        -> Result<Box<dyn AsyncRead + Unpin + Send>>;
    
    /// Estimate uncompressed size (if possible)
    async fn estimate_uncompressed_size(&self, compressed_size: u64) -> Option<u64>;
    
    /// Check if format supports random access (most don't)
    fn supports_random_access(&self) -> bool {
        false
    }
}

/// Detection result with confidence level
pub struct CompressionDetection {
    pub format: CompressionType,
    pub confidence: DetectionConfidence,
}

pub enum DetectionConfidence {
    High,    // Magic bytes match
    Medium,  // Extension match only
    Low,     // Heuristic guess
}
```

### Detection Strategy

```rust
/// Enhanced compression detection
pub async fn detect_compression(path: &Path) -> Result<CompressionType> {
    // 1. Check magic bytes first (most reliable)
    let magic = read_magic_bytes(path).await?;
    if let Some(format) = detect_by_magic(&magic) {
        return Ok(format);
    }
    
    // 2. Fall back to extension
    if let Some(format) = detect_by_extension(path) {
        return Ok(format);
    }
    
    // 3. No compression detected
    Ok(CompressionType::None)
}

fn detect_by_magic(magic: &[u8]) -> Option<CompressionType> {
    match magic {
        [0x1f, 0x8b, ..] => Some(CompressionType::Gzip),
        [0x42, 0x5a, 0x68, ..] => Some(CompressionType::Bzip2),
        [0xfd, 0x37, 0x7a, 0x58, 0x5a, 0x00, ..] => Some(CompressionType::Xz),
        [0x28, 0xb5, 0x2f, 0xfd, ..] => Some(CompressionType::Zstd),
        _ => None,
    }
}

fn detect_by_extension(path: &Path) -> Option<CompressionType> {
    let ext = path.extension()?.to_str()?;
    match ext {
        "gz" => Some(CompressionType::Gzip),
        "bz2" => Some(CompressionType::Bzip2),
        "xz" => Some(CompressionType::Xz),
        "zst" | "zstd" => Some(CompressionType::Zstd),
        _ => None,
    }
}
```

### CompressedFileAccessor Implementation

```rust
/// Wrapper that provides FileAccessor interface for compressed files
pub struct CompressedFileAccessor {
    // Strategy: Decompress to temp file for mmap efficiency
    temp_file: Option<NamedTempFile>,
    inner_accessor: Box<dyn FileAccessor>,
    compression_type: CompressionType,
    original_path: PathBuf,
    
    // Metadata
    compressed_size: u64,
    uncompressed_size: u64,
}

impl CompressedFileAccessor {
    pub async fn new(path: &Path, compression: CompressionType) -> Result<Self> {
        // For small files (<10MB compressed), decompress to memory
        // For large files, decompress to temp file and mmap
        let compressed_size = tokio::fs::metadata(path).await?.len();
        
        let (temp_file, inner_accessor) = if compressed_size < 10_000_000 {
            // Small file: decompress to memory
            let data = decompress_to_memory(path, compression).await?;
            let accessor = Box::new(InMemoryFileAccessor::from_bytes(data));
            (None, accessor as Box<dyn FileAccessor>)
        } else {
            // Large file: stream to temp file
            let temp = decompress_to_temp_file(path, compression).await?;
            let temp_path = temp.path().to_path_buf();
            let accessor = Box::new(MmapFileAccessor::new(&temp_path).await?);
            (Some(temp), accessor as Box<dyn FileAccessor>)
        };
        
        Ok(CompressedFileAccessor {
            temp_file,
            inner_accessor,
            compression_type: compression,
            original_path: path.to_path_buf(),
            compressed_size,
            uncompressed_size: inner_accessor.file_size(),
        })
    }
}

#[async_trait]
impl FileAccessor for CompressedFileAccessor {
    async fn read_range(&self, start: u64, length: usize) -> Result<Vec<u8>> {
        self.inner_accessor.read_range(start, length).await
    }
    
    async fn read_line(&self, line_number: u64) -> Result<Cow<'_, str>> {
        self.inner_accessor.read_line(line_number).await
    }
    
    // ... delegate other methods to inner_accessor
}
```

### Format-Specific Adapters

```rust
// src/compression/gzip.rs
pub struct GzipAdapter;

#[async_trait]
impl CompressionAdapter for GzipAdapter {
    async fn create_decoder(&self, source: Box<dyn AsyncRead + Unpin + Send>) 
        -> Result<Box<dyn AsyncRead + Unpin + Send>> {
        Ok(Box::new(async_compression::tokio::bufread::GzipDecoder::new(
            BufReader::new(source)
        )))
    }
    
    async fn estimate_uncompressed_size(&self, compressed_size: u64) -> Option<u64> {
        // Gzip typically achieves 10:1 for text logs
        Some(compressed_size * 10)
    }
}

// Similar implementations for Bzip2Adapter, XzAdapter, ZstdAdapter
```

### Integration with FileAccessorFactory

```rust
// Update FileAccessorFactory in file_handler.rs
impl FileAccessorFactory {
    pub async fn create(path: &Path) -> Result<Box<dyn FileAccessor>> {
        // Validate file first
        validate_file_path(path)?;
        
        // Check for compression
        let compression = detect_compression(path).await?;
        
        if compression != CompressionType::None {
            // Use CompressedFileAccessor for compressed files
            return Ok(Box::new(
                CompressedFileAccessor::new(path, compression).await?
            ));
        }
        
        // Existing logic for uncompressed files
        let metadata = tokio::fs::metadata(path).await?;
        let file_size = metadata.len();
        
        // Use existing threshold logic
        let threshold = Self::get_threshold();
        
        if file_size < threshold {
            Ok(Box::new(InMemoryFileAccessor::new(path).await?))
        } else {
            Ok(Box::new(MmapFileAccessor::new(path).await?))
        }
    }
}
```

## Implementation Strategy

### Phase 1: Core Infrastructure
1. Create `src/compression.rs` with traits and enums
2. Implement detection logic with magic bytes
3. Create `CompressedFileAccessor` with basic functionality

### Phase 2: Format Adapters
1. Add dependencies to Cargo.toml:
   - `async-compression = { version = "0.4", features = ["tokio", "gzip", "bzip2", "xz", "zstd"] }`
   - `tempfile = "3.8"`
2. Implement each adapter (GzipAdapter, etc.)
3. Add format-specific tests

### Phase 3: Integration
1. Update `FileAccessorFactory` to detect and handle compression
2. Ensure search works on compressed files
3. Add progress reporting for decompression

### Phase 4: Optimization
1. Implement streaming decompression for better memory usage
2. Add caching for frequently accessed compressed files
3. Consider parallel decompression for multi-core systems

## Memory Management Strategy

### Small Files (<10MB compressed)
- Decompress entirely to memory
- Use `InMemoryFileAccessor` for best performance
- Trade memory for speed

### Large Files (≥10MB compressed)
- Stream decompress to temp file (NOT mmap the compressed file)
- Use `MmapFileAccessor` on the UNCOMPRESSED temp file
- Clean up temp file on drop
- Flow: compressed file → stream decompress → temp file → mmap temp file

### Huge Files (>1GB compressed)
- Same as large files (decompress to temp, then mmap)
- Consider progress reporting during decompression
- Future: chunked decompression for visible region only

## Threading Strategy

### Current Design: Single-threaded
- Use `async-compression` with tokio async I/O
- Simple implementation, adequate for most cases
- Decompression happens in single thread with async streaming

### Future Optimization: Multi-threaded
- **zstd**: Native multi-threading support via `NbWorkers` parameter
- **gzip**: Could use external `pigz` for parallel decompression
- **bzip2**: Block-based format allows parallel decompression
- **xz**: Multi-stream files can be parallelized

### When to Consider Multi-threading
- Files >100MB compressed with slow decompression (bzip2, xz)
- Multi-core systems with CPU to spare
- Initial decompression is bottleneck (measure first!)

```rust
// Example: Future multi-threaded zstd support
pub struct ZstdAdapter {
    threads: Option<u32>, // None = single-threaded
}

impl ZstdAdapter {
    pub fn with_threads(threads: u32) -> Self {
        Self { threads: Some(threads) }
    }
}
```

## Error Handling

```rust
#[derive(Error, Debug)]
pub enum CompressionError {
    #[error("Unsupported compression format: {0:?}")]
    UnsupportedFormat(CompressionType),
    
    #[error("Corrupted compressed file: {0}")]
    CorruptedFile(String),
    
    #[error("Decompression failed: {0}")]
    DecompressionFailed(#[from] std::io::Error),
    
    #[error("Temp file creation failed: {0}")]
    TempFileError(#[from] tempfile::Error),
}
```

## Testing Strategy

### Unit Tests
```rust
#[cfg(test)]
mod tests {
    #[test]
    fn test_magic_byte_detection() {
        assert_eq!(detect_by_magic(&[0x1f, 0x8b]), Some(CompressionType::Gzip));
        assert_eq!(detect_by_magic(&[0x42, 0x5a, 0x68]), Some(CompressionType::Bzip2));
    }
    
    #[tokio::test]
    async fn test_gzip_decompression() {
        let compressed = create_test_gzip_file();
        let accessor = CompressedFileAccessor::new(&compressed, CompressionType::Gzip).await.unwrap();
        assert_eq!(accessor.read_line(0).await.unwrap(), "test line");
    }
}
```

### Integration Tests
1. Test with real compressed log files
2. Verify search works on compressed files
3. Test memory usage stays within bounds
4. Test corrupted file handling

### Performance Tests
1. Benchmark decompression speed
2. Measure memory usage during decompression
3. Test with various compression ratios

## Dependencies

Add to `Cargo.toml`:
```toml
[dependencies]
async-compression = { version = "0.4", features = ["tokio", "gzip", "bzip2", "xz", "zstd", "all-algorithms"] }
tempfile = "3.8"
tokio = { version = "1.35", features = ["fs", "io-util"] }
```

## Open Questions

1. **Progressive Decompression**: Should we implement partial decompression for better initial response time?
2. **Caching Strategy**: Should we cache decompressed temp files between sessions?
3. **Multi-threaded Decompression**: Is parallel decompression worth the complexity?
4. **Compression Level Detection**: Should we detect and report compression ratio?

## Success Criteria

- ✅ All common compression formats supported (gzip, bzip2, xz, zstd)
- ✅ Transparent operation - users don't need to know file is compressed
- ✅ Memory usage stays under 100MB even for large compressed files
- ✅ Decompression start time <100ms
- ✅ Search works seamlessly on compressed files
- ✅ Graceful handling of corrupted compressed files