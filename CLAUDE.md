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
- **Module size management**: When a module file becomes too large (>500 lines), refactor into a directory structure:
  - Create a directory with the module name
  - Break the large module into focused sub-modules within that directory
  - The original module file becomes an import/re-export hub
  - Maintain clean public API by selective re-exports

```rust
// Good: src/file_handler.rs (when small)
pub struct MmapFileAccessor { /* ... */ }
pub trait FileAccessor { /* ... */ }

// Good: src/lib.rs
pub mod file_handler;
pub mod search;
pub mod ui;

// Good: Large module refactoring example
// When src/file_handler.rs becomes >500 lines, refactor to:

// src/file_handler.rs (becomes import/re-export hub)
pub mod accessor;
pub mod compression;
pub mod memory_mapping;
pub mod validation;

// Re-export public API
pub use accessor::{FileAccessor, AccessStrategy};
pub use compression::{CompressionType, detect_compression};
pub use memory_mapping::MmapFileAccessor;
pub use validation::validate_file_path;

// src/file_handler/accessor.rs
pub trait FileAccessor { /* trait definition */ }
pub enum AccessStrategy { /* enum definition */ }

// src/file_handler/compression.rs  
pub enum CompressionType { /* compression types */ }
pub fn detect_compression() { /* detection logic */ }

// src/file_handler/memory_mapping.rs
pub struct MmapFileAccessor { /* implementation */ }

// src/file_handler/validation.rs
pub fn validate_file_path() { /* validation logic */ }
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

### ❌ Quick and Dirty Fixes
- **Never implement hacky workarounds** just to get past compiler or linter errors
- **Don't comment out code** or add placeholder implementations to "fix" compilation
- **Don't suppress warnings** without understanding and addressing the root cause
- **If you encounter a problem you can't solve properly:**
  1. Stop and return the error message
  2. Explain your analysis of what's wrong
  3. Suggest potential approaches but don't implement incomplete solutions
  4. Let's think through the problem together rather than patch around it

```rust
// ❌ Bad: Quick fix to make it compile
// pub use app::Application;  // TODO: Fix later

// ✅ Good: Acknowledge the issue and solve it properly
// Error: Cannot re-export Application because app module doesn't define it yet
// Solution: Either implement the type first, or restructure the exports
```

## Performance Targets

These targets guide our implementation decisions:

- **File Opening**: <2 seconds for 40GB files
- **Search Response**: <500ms for 40GB files  
- **Memory Usage**: <100MB regardless of file size
- **Navigation**: <50ms response time
- **Startup**: <100ms from CLI to interactive

## Future Optimizations (Not Yet Implemented)

These optimizations should be considered only after measuring actual performance bottlenecks:

### MmapFileAccessor Line Range Caching
- **Problem**: Repeated LineIndex lookups for recently accessed lines
- **Solution**: Add LRU cache for line ranges `(start_byte, end_byte)`
- **Challenge**: LruCache requires `&mut self` for both get/put operations (no internal mutability)
- **Options**: 
  - `Mutex<LruCache>` (honest about exclusive locking)
  - Concurrent cache like `moka` or `quick_cache` (true internal mutability)
- **Decision**: Defer until profiling shows LineIndex lookups are a bottleneck

### Pipe/Stdin Support (Lower Priority)
- **Problem**: Current architecture only supports file paths, not piped input like `cat file | rlless`
- **Solution**: Add `PipeFileAccessor` to handle stdin input with smart buffering
- **Strategy**:
  - Small pipes (< 50MB): Use `InMemoryFileAccessor` directly
  - Large pipes (≥ 50MB): Spill to temporary file, then use `MmapFileAccessor`
- **Implementation**:
  ```rust
  pub enum PipeFileAccessor {
      InMemory(InMemoryFileAccessor),
      Spilled { temp_file: NamedTempFile, accessor: MmapFileAccessor }
  }
  ```
- **CLI Integration**: `FileAccessorFactory::create_from_path_or_stdin(path: Option<&Path>)`
- **Benefits**: Enables standard Unix pipe workflows while maintaining performance
- **Decision**: Implement after Phase 2 core components are complete

## Development Workflow

1. **Start with traits** - Define interfaces before implementations
2. **Implement MVP functionality** - Core features first
3. **Add tests as you go** - Don't defer testing
4. **Measure performance** - Benchmark critical paths
5. **Iterate and improve** - Refine based on actual usage

## Async Trait Guidelines

### Native vs async-trait Crate Decision
- **Rust 1.75+** stabilized `async fn` in traits but with **critical limitation**: no dynamic dispatch (`dyn Trait` support)
- **Our codebase uses `Box<dyn FileAccessor>`** for dependency injection
- **Therefore: Continue using `async-trait` crate** for trait objects
- **Use `Cow<str>` return types** instead of `String` to avoid unnecessary allocations

### When to Use Each Approach
```rust
// ✅ Use async-trait crate when:
// - Need dynamic dispatch (Box<dyn Trait>)  
// - Support older Rust versions
// - Complex lifetime scenarios

#[async_trait]
trait FileAccessor {
    async fn read_line(&self, line_number: u64) -> Result<Cow<'_, str>>;
}

// ✅ Use native async fn when:
// - Static dispatch only
// - Simple lifetime requirements
// - Rust 1.75+ minimum version

trait SimpleService {
    async fn process(&self, data: &str) -> String; // No dyn support
}
```

### Memory Efficiency Rules
- **Return `Cow<str>` instead of `String`** when possible
- **Let caller decide**: `.as_ref()` for `&str`, `.into_owned()` for `String`
- **InMemoryFileAccessor**: Use `Cow::Borrowed` for cached lines (zero allocation)
- **Other accessors**: Use `Cow::Owned` when data must be constructed

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

## Phase 1: Foundation & File Access ✅ COMPLETED

**Tasks 1-4: Complete implementation with 77 passing tests**

Phase 1 established the core file handling infrastructure including:
- **Task 1**: ✅ Rust project foundation with Cargo.toml and module structure
- **Task 2**: ✅ Error handling patterns with `anyhow` integration
- **Task 3**: ✅ File access traits with byte-based navigation (`FileAccessor`, `FileAccessorFactory`)
- **Task 4**: ✅ Multiple file access implementations:
  - `InMemoryFileAccessor` for small files (<50MB)
  - `MmapFileAccessor` for large files using memory mapping
  - `AdaptiveFileAccessor` with intelligent strategy selection
  - SIMD-optimized line detection and zero-copy operations

**Key Architectural Achievements**:
- Modern Rust module system (no mod.rs files)
- Trait-based plugin architecture for extensibility  
- Comprehensive unit testing (77 tests passing)
- Performance-optimized byte-based navigation
- Memory-efficient handling of files up to 40GB+

## Phase 2: Core Components ✅ COMPLETED

**Goal**: Implementation of search, UI, and application coordination

### Task 5: Add Compression Support ❌ WILL NOT IMPLEMENT
**Decision**: Compression support is deprioritized for MVP. The core file access architecture supports it via trait extension, but implementation is deferred until after user feedback on MVP.

**Rationale**: 
- Adds complexity without clear user demand
- Can be added later without architectural changes
- Focus on core viewer functionality first

### Task 6: Create Search Engine Module ✅ IMPLEMENTED 
- ✅ `SearchEngine` trait with async search methods
- ✅ `RipgrepEngine` implementation with regex support
- ✅ `SearchOptions` for different search modes
- ✅ Bidirectional search (forward/backward navigation)
- ✅ Line-based match highlighting
- ✅ Integration with byte-based file access

### Task 7: Implement UI Module ✅ IMPLEMENTED
- ✅ `UIRenderer` trait for pluggable UI backends
- ✅ `TerminalUI` with ratatui backend
- ✅ Complete input handling state machine
- ✅ Key binding system matching `less` interface
- ✅ Viewport management with search highlighting
- ✅ Color theming system
- ✅ Real-time search prompt display

### Task 8: Create Application Core ✅ IMPLEMENTED
- ✅ `Application` struct coordinating all components
- ✅ `ViewState` with viewport and status management
- ✅ Event loop with proper EOF handling
- ✅ Navigation actions (PageUp/Down, directional Scroll, GoToStart/End)
- ✅ Search integration with match navigation
- ✅ Proper "EOD" display when reaching end of content

## Phase 3: User Interface & Navigation ✅ COMPLETED

**Tasks 9-11: Complete less-like interface with 77 passing tests**

### Task 9: Navigation Commands ✅ IMPLEMENTED
- ✅ Line navigation (j/k, arrow keys) with <50ms response
- ✅ Page navigation (Space, PageUp/PageDown)
- ✅ File position navigation (g for start, G for end)
- ✅ Proper viewport management and EOF handling
- ✅ Navigation works smoothly with files of any size

### Task 10: Bidirectional Search ✅ IMPLEMENTED
- ✅ Forward search (`/`) and backward search (`?`) input handling
- ✅ Next match (`n`) and previous match (`N`) navigation
- ✅ Search result highlighting in display
- ✅ Real-time search prompt with live updates
- ✅ Search state management across navigation
- ✅ Integration with search engine caching

### Task 11: CLI Interface ✅ IMPLEMENTED
- ✅ Basic usage: `rlless /path/to/file.log`
- ✅ File validation and error handling
- ✅ Comprehensive help text and version info
- ✅ User-friendly error messages for invalid arguments
- ✅ `clap`-based argument parsing

## Current Status: **MVP COMPLETE** 🎉

**What Works**:
- Fast file loading and navigation for files of any size
- Real-time regex search with highlighting
- Complete `less`-like interface (j/k, PageUp/Down, /, ?, n, N, g, G, q)
- Proper EOF handling and status display
- Memory-efficient operation (<100MB regardless of file size)

**Performance Results** (Recent benchmarks):
- ✅ File Opening: <2s for large files (target met)
- ⚠️ Search Performance: Regression detected (needs investigation)
- ✅ Memory Usage: <100MB (target met)
- ✅ Navigation: <50ms response time (target met)

**Known Issues**:
- Search performance regression (~2800x slower for regex, needs investigation)
- Line access 25-30% slower than previous implementation

## Phase 3 & 4: Quality Assurance & Advanced Features ⏸️ DEFERRED

**Decision**: Focus on fixing performance regressions and gathering user feedback before implementing advanced features.

**Deferred Items**:
- Compression support (can be added via trait extension)
- Advanced search features (case sensitivity options, whole word)
- Configuration system
- Extensive integration testing
- Performance optimizations beyond fixing regressions

## Next Priorities

1. **P0**: Investigate and fix search performance regression
2. **P0**: Investigate line access performance regression  
3. **P1**: User testing and feedback collection
4. **P2**: Performance optimizations based on real usage patterns
