# CLAUDE.md - Development Guidelines for rlless

## Project Overview
rlless is a high-performance terminal log viewer for large files (40GB+) built with Rust 2024, featuring SIMD-optimized search via ripgrep and memory-efficient file handling.

## Core Development Principles

### 1. Modern Rust Module Management
- **No `mod.rs` files** - Use the modern Rust module system
- Each module should be self-contained in its own file
- Use `pub mod module_name;` in `lib.rs` or parent modules
- Clear module boundaries with well-defined public APIs
- Minimize cross-module dependencies

```rust
// Good: src/file_handler.rs
pub struct MmapFileAccessor { /* ... */ }
pub trait FileAccessor { /* ... */ }

// Good: src/lib.rs
pub mod file_handler;
pub mod search;
pub mod ui;
```

### 2. Keep Things Simple
- **Favor simplicity over cleverness** - Choose the most straightforward solution
- **MVP-first approach** - Implement core functionality before optimization
- **Avoid over-engineering** - Don't build features we don't need yet
- **Clear, readable code** over complex abstractions
- **Single responsibility** - Each function/struct should do one thing well

```rust
// Good: Simple, clear function
pub async fn read_line(&self, line_number: u64) -> Result<String> {
    // Straightforward implementation
}

// Avoid: Complex generic abstractions unless truly needed
```

### 3. Trait-Based Plugin Architecture
- **Use traits for extensibility** - All core components should be behind traits
- **Dependency injection** - Components should accept trait objects
- **Easy testing** - Traits enable mock implementations
- **Future-proof design** - New implementations can be added without breaking changes

```rust
// Core pattern to follow:
#[async_trait]
pub trait FileAccessor: Send + Sync {
    async fn read_range(&self, start: u64, length: usize) -> Result<Vec<u8>>;
    async fn read_line(&self, line_number: u64) -> Result<String>;
    fn file_size(&self) -> u64;
}

// Application uses trait objects:
pub struct Application {
    file_handler: Box<dyn FileAccessor>,
    search_engine: Box<dyn SearchEngine>,
    ui_manager: Box<dyn UIRenderer>,
}
```

### 4. Simple Unit Testing Strategy
- **Focus on unit tests** - 80% of testing effort should be unit tests
- **Test core functionality** - Public APIs, error conditions, edge cases
- **Keep tests simple** - No complex test frameworks or elaborate setups
- **Test behavior, not implementation** - Focus on what the code does, not how
- **Property-based testing** for complex algorithms using `proptest`

```rust
// Good: Simple, focused unit test
#[tokio::test]
async fn test_read_line_basic() {
    let accessor = create_test_accessor("line1\nline2\n");
    let result = accessor.read_line(0).await.unwrap();
    assert_eq!(result, "line1");
}

// Good: Edge case testing
#[tokio::test]
async fn test_read_line_out_of_bounds() {
    let accessor = create_test_accessor("line1\n");
    let result = accessor.read_line(999).await;
    assert!(result.is_err());
}

// Avoid: Overly complex test scenarios unless necessary
```

### 5. Module-Level Integration Testing Only
- **Integration tests at module boundaries** - Test how modules work together
- **No end-to-end integration complexity** - Keep integration focused
- **Mock external dependencies** - Use trait implementations for testing
- **Focus on public interfaces** - Test the contracts between modules

```rust
// Good: Module-level integration test
#[tokio::test]
async fn test_file_handler_with_search_engine() {
    let file_accessor = MmapFileAccessor::new(test_file_path()).await.unwrap();
    let search_engine = RipgrepEngine::new(Arc::new(file_accessor)).unwrap();
    
    let matches = search_engine.search("ERROR", SearchOptions::default()).await.unwrap();
    assert!(!matches.is_empty());
}
```

## Implementation Guidelines

### Error Handling
- Use `anyhow` for application-level error handling
- Custom error types only when needed for specific error handling logic
- Provide context with errors for debugging
- Fail fast and provide clear error messages

### Performance
- **Measure first, optimize second** - Don't assume performance bottlenecks
- **Profile actual usage** - Use realistic data patterns for testing
- **Benchmark critical paths** - File access and search are performance-critical
- **Memory efficiency** - Always consider memory usage, especially for large files

### Dependencies
- **Prefer established crates** - Use well-maintained, popular libraries
- **Minimize dependency count** - Only add dependencies that provide significant value
- **Pin major versions** - Avoid breaking changes from dependency updates

## Code Organization

### File Structure
```
src/
├── lib.rs              # Public API and module declarations
├── main.rs             # CLI entry point
├── app.rs              # Application core and coordination
├── cli.rs              # Command-line argument parsing
├── config.rs           # Configuration management
├── error.rs            # Error types and handling
├── file_handler.rs     # File access traits and implementations
├── search.rs           # Search engine traits and implementations
├── ui.rs               # Terminal UI traits and implementations
├── compression.rs      # Compression format support
├── buffer.rs           # Buffer management utilities
└── utils.rs            # Utility functions
```

### Testing Structure
```
tests/
├── integration/
│   ├── file_tests.rs   # File handling integration tests
│   ├── search_tests.rs # Search integration tests
│   └── ui_tests.rs     # UI integration tests
└── fixtures/           # Test data files

benches/
├── file_access.rs      # File access benchmarks
├── search_performance.rs # Search performance benchmarks
└── memory_usage.rs     # Memory usage benchmarks
```

## What NOT to Do

### ❌ Over-Engineering
- Don't create complex type hierarchies unless needed
- Don't abstract everything - some concrete types are fine
- Don't build configuration systems until we need them
- Don't optimize prematurely

### ❌ Complex Testing
- Don't write integration tests that span the entire application
- Don't create elaborate test frameworks or fixtures
- Don't test private implementation details
- Don't mock everything - use real implementations when simple

### ❌ Anti-Patterns
- No God objects - keep structs focused and small
- No deep inheritance hierarchies - favor composition
- No global state - use dependency injection
- No premature abstraction - start concrete, abstract when patterns emerge

## Performance Targets

These targets guide our implementation decisions:

- **File Opening**: <2 seconds for 40GB files
- **Search Response**: <500ms for 40GB files  
- **Memory Usage**: <100MB regardless of file size
- **Navigation**: <50ms response time
- **Startup**: <100ms from CLI to interactive

## Development Workflow

1. **Start with traits** - Define interfaces before implementations
2. **Implement MVP functionality** - Core features first
3. **Add tests as you go** - Don't defer testing
4. **Measure performance** - Benchmark critical paths
5. **Iterate and improve** - Refine based on actual usage

## Questions to Ask Before Adding Complexity

1. **Do we actually need this?** - Is it required for the MVP?
2. **Is there a simpler way?** - Can we solve it with less code?
3. **Can we defer this?** - Is this a future enhancement?
4. **Does this follow the trait pattern?** - Can other implementations plug in?
5. **How do we test this?** - Is it testable with simple unit tests?

## Success Criteria

- ✅ All modules have clear, single responsibilities
- ✅ Trait boundaries enable easy testing and future extensions
- ✅ Unit tests cover core functionality with simple, readable tests
- ✅ Performance targets are met with straightforward implementations
- ✅ Code is maintainable and easy to understand

Remember: **Simple, working code is better than complex, clever code.**

## Phase 1 Implementation Tasks

### Task 1: Project Foundation Setup
- **1.1**: Create Cargo.toml with P0 dependencies and Rust 2024 configuration
  - Essential deps: ripgrep, ratatui, memmap2, tokio, crossterm, anyhow
  - Compression: flate2, bzip2, xz2
  - Dev deps: thiserror for error handling
- **1.2**: Set up src/ module structure following modern Rust patterns
  - No mod.rs files - each module is self-contained
- **1.3**: Create lib.rs with public module declarations
  - Clean public API surface
- **1.4**: Create basic main.rs CLI entry point
  - Minimal CLI to verify project structure

### Task 2: Error Infrastructure  
- **2.1**: Define core error types using thiserror in error.rs
  - File, Search, UI error variants
  - Clear error messages for users
- **2.2**: Implement error context and Result type alias
  - Standardized Result<T> across all modules
- **2.3**: Add basic error unit tests
  - Test error creation and message formatting

### Task 3: File Handler Trait Design
- **3.1**: Design FileAccessor trait with async methods
  - Focus on clean interface design first
  - read_range, read_line, file_size, total_lines methods
- **3.2**: Implement compression detection using magic numbers
  - Simple, reliable detection without external dependencies
- **3.3**: Create AccessStrategy enum for different file access patterns
  - MemoryMapped, Streaming, Hybrid variants
- **3.4**: Add file validation and basic unit tests
  - Test trait design with mock implementations

### Task 4: Memory Mapping Implementation
- **4.1**: Implement MmapFileAccessor with memory mapping
  - Core memory mapping functionality
  - Handle large files efficiently
- **4.2**: Add line indexing for efficient line access
  - Simple line boundary detection and caching
- **4.3**: Implement memory pressure detection and fallback strategy
  - Automatic strategy selection based on system resources
- **4.4**: Add comprehensive unit tests for memory mapping
  - Test various file sizes and edge cases

### Integration Checkpoint
- **Verify**: All modules compile and basic tests pass
- **Validate**: Core traits enable future implementations
- **Test**: Memory mapping works with test files up to 1GB

### Architectural Decisions Made

#### Error Strategy
- Use `thiserror` for custom errors with good derive support
- `anyhow` for application-level error handling with context
- Custom `Result<T>` type alias for consistency

#### File Access Strategy
- Trait-first design enables testing and future extensions
- Memory mapping as primary strategy with streaming fallback
- Line indexing for efficient random access to large files

#### Module Boundaries
- `error.rs` - Centralized error types, used by all modules
- `file_handler.rs` - File access abstraction, core performance component  
- Clear separation of concerns, minimal cross-dependencies

#### Testing Strategy
- Unit tests for each module's public interface
- Mock implementations via traits for isolated testing
- Property-based testing for file access edge cases