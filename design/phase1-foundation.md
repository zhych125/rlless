# Phase 1: Foundation & Core Infrastructure

**Phase Goal**: Establish the basic Rust project structure and fundamental infrastructure that all other components depend on.

**Priority**: P0 Essential  
**Tasks**: 1-4  
**Dependencies**: None  
**Estimated Duration**: 2-3 days  

## Overview

Phase 1 creates the foundational elements needed for rlless development. These tasks establish the project structure, error handling patterns, and basic file access infrastructure that enable all subsequent development work.

## Tasks

### **Task 1: Set up Rust project foundation with Cargo.toml and basic structure**

**Priority**: P0 Essential  
**Scope**: Project initialization, dependency setup, module structure  
**Inputs**: Design specifications, dependency requirements  
**Outputs**: Complete Cargo.toml, src/ directory structure, basic main.rs  

**Acceptance Criteria**:
- Cargo.toml with all P0 dependencies (ripgrep, ratatui, memmap2, tokio, crossterm, anyhow)
- Compression dependencies (flate2, bzip2, xz2) configured
- Rust 2024 edition with MSRV 1.75.0
- Basic module files created (app.rs, file_handler.rs, search.rs, ui.rs, etc.)
- Project compiles successfully with `cargo check`

**Dependencies**: None  
**Estimated Effort**: Simple (2-3 hours)  

**Technical Notes**: 
- Use modular structure without mod.rs files as specified
- Configure cargo features for compression and SIMD support
- Set up release profile with LTO optimization

**Implementation Details**:
```toml
# Cargo.toml structure needed
[profile.release]
lto = true
codegen-units = 1
panic = "abort"
strip = true

[features]
default = ["compression", "simd"]
compression = ["flate2", "bzip2", "xz2"]
simd = ["ripgrep/simd-accel"]
```

---

### **Task 2: Implement core error types and handling infrastructure**

**Priority**: P0 Essential  
**Scope**: Error types, error handling patterns, result types  
**Inputs**: Design error handling requirements  
**Outputs**: Complete error.rs module with comprehensive error types  

**Acceptance Criteria**:
- Custom error types for file operations, search operations, UI operations
- Integration with anyhow for ergonomic error handling
- Error context preservation for debugging
- Standardized Result types for all modules

**Dependencies**: Task 1  
**Estimated Effort**: Simple (3-4 hours)  

**Technical Notes**: 
- Design for extensibility; errors should provide actionable user feedback
- Use thiserror for derive macros
- Consider error codes for programmatic handling

**Implementation Details**:
```rust
// Core error types structure
#[derive(thiserror::Error, Debug)]
pub enum RllessError {
    #[error("File operation failed: {0}")]
    FileError(#[from] std::io::Error),
    
    #[error("Search operation failed: {0}")]
    SearchError(String),
    
    #[error("UI operation failed: {0}")]
    UIError(String),
}

pub type Result<T> = std::result::Result<T, RllessError>;
```

---

### **Task 3: Create file handler module with FileAccessor trait and compression detection**

**Priority**: P0 Essential  
**Scope**: FileAccessor trait, file type detection, basic file operations  
**Inputs**: File handling design specifications  
**Outputs**: file_handler.rs with FileAccessor trait and compression detection  

**Acceptance Criteria**:
- FileAccessor trait with async methods (read_range, read_line, file_size, total_lines)
- Automatic compression type detection (gzip, bzip2, xz, none)
- AccessStrategy enum with variants for different file access patterns
- File validation and error handling

**Dependencies**: Task 2  
**Estimated Effort**: Medium (6-8 hours)  

**Technical Notes**: 
- Focus on trait design; implementations come in subsequent tasks
- Magic number detection for compression formats
- Design for testability with mock implementations

**Implementation Details**:
```rust
#[async_trait]
pub trait FileAccessor: Send + Sync {
    async fn read_range(&self, start: u64, length: usize) -> Result<Vec<u8>>;
    async fn read_line(&self, line_number: u64) -> Result<String>;
    fn file_size(&self) -> u64;
    fn total_lines(&self) -> Option<u64>;
}

pub enum CompressionType {
    None,
    Gzip,
    Bzip2,
    Xz,
}

pub fn detect_compression(path: &Path) -> Result<CompressionType> {
    // Magic number detection implementation
}
```

---

### **Task 4: Implement memory mapping strategy for large files in file handler**

**Priority**: P0 Essential  
**Scope**: Memory mapping implementation, streaming fallback, hybrid approach  
**Inputs**: File access strategy from design  
**Outputs**: Memory mapping implementation with automatic fallback  

**Acceptance Criteria**:
- Memory-mapped file access for files that fit in virtual memory
- Automatic fallback to streaming for very large files
- Hybrid approach combining mmap and streaming as needed
- Memory usage stays under 100MB regardless of file size
- Works with files up to 40GB

**Dependencies**: Task 3  
**Estimated Effort**: Complex (10-12 hours)  

**Technical Notes**: 
- Critical for performance targets; requires careful memory management
- Consider system memory availability for strategy selection
- Implement graceful degradation for memory pressure

**Implementation Details**:
```rust
pub enum AccessStrategy {
    MemoryMapped(Mmap),
    Streaming(Box<dyn AsyncRead + Unpin>),
    Hybrid { 
        mmap: Mmap, 
        stream: Box<dyn AsyncRead + Unpin> 
    },
}

pub struct MmapFileAccessor {
    mmap: Mmap,
    file_size: u64,
    // Line index for efficient line access
    line_index: Option<Vec<u64>>,
}
```

## Phase 1 Success Criteria

By the end of Phase 1, the following should be complete:
- ✅ Rust project compiles and runs basic tests
- ✅ Error handling infrastructure works across modules
- ✅ File accessor trait design enables future implementations
- ✅ Memory mapping works efficiently for large files
- ✅ Foundation ready for core component development

## Next Steps

Upon completion of Phase 1, proceed to Phase 2 (Core Components) which will implement:
- Compression support integration
- ripgrep search engine integration  
- Terminal UI with ratatui
- Application core coordination

## Risk Considerations

**Critical Risks for Phase 1**:
- **Memory mapping complexity**: Test with various file sizes early
- **Trait design inflexibility**: Review with future use cases in mind
- **Cross-platform compatibility**: Test on all target platforms

**Mitigation Strategies**:
- Prototype memory mapping approach before full implementation
- Design traits with extensibility in mind
- Set up CI testing matrix early