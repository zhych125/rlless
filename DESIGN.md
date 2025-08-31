# Technical Design Specifications for rlless

## 1. Executive Summary

rlless is a high-performance terminal-based log viewer built in Rust that can handle extremely large log files (40GB+) with memory-efficient streaming and SIMD-optimized search. The architecture emphasizes simplicity, modularity, and performance while maintaining a familiar less-like interface.

**Key Technical Decisions:**
- **Language**: Rust 2024 edition for memory safety and performance
- **Architecture**: Modular design with trait-based extensibility
- **File Handling**: Hybrid approach using memory mapping + streaming
- **Search Engine**: Integration with ripgrep-core for SIMD optimization
- **UI Framework**: ratatui for cross-platform terminal interface
- **Concurrency**: Tokio async runtime with selective parallelism

## 2. Architecture Overview

### High-Level System Architecture

```
┌─────────────────┐    ┌──────────────────┐    ┌─────────────────┐
│  File Handler   │    │  Search Engine   │    │   UI Manager    │
│  (P0 Essential) │◄──►│  (P0 Essential)  │◄──►│  (P0 Essential) │
│  - Memory Map   │    │  - ripgrep-core  │    │  - ratatui      │
│  - Streaming    │    │  - SIMD Search   │    │  - Event Loop   │
│  - Compression  │    │  - Result Cache  │    │  - Key Handler  │
└─────────────────┘    └──────────────────┘    └─────────────────┘
         │                        │                        │
         └────────────────────────┼────────────────────────┘
                                  ▼
              ┌─────────────────────────────────┐
              │      Application Core           │
              │      (P0 Essential)             │
              │  - Event Dispatcher             │
              │  - State Management             │
              │  - Error Handling               │
              └─────────────────────────────────┘
```

### Module Interaction Flow

```
User Input → UI Manager → Application Core → File Handler
                             ↓
Search Request → Search Engine → Results Cache → UI Update
                             ↓
File Operations → Compression Handler → Memory Manager
```

## 3. Technology Stack

### Core Dependencies with Justification

**Essential (P0) Dependencies:**
- `ripgrep` (13.0+) - SIMD-optimized search engine, proven performance with large files
- `ratatui` (0.30+) - Mature terminal UI framework with efficient rendering
- `memmap2` (0.11+) - Safe memory-mapped file access for large file handling
- `tokio` (1.35+) - Async runtime for non-blocking I/O operations
- `crossterm` (0.28+) - Cross-platform terminal manipulation
- `anyhow` (1.0+) - Ergonomic error handling for rapid development

**Compression Support (P0):**
- `flate2` (1.0+) - Gzip compression support
- `bzip2` (0.5+) - Bzip2 compression support
- `xz2` (0.1+) - XZ compression support

**Performance & Utility (P1):**
- `rayon` (1.8+) - Data parallelism for search operations
- `clap` (4.4+) - Command-line argument parsing
- `serde` (1.0+) - Configuration serialization

**Testing & Development:**
- `proptest` (1.4+) - Property-based testing for edge cases
- `criterion` (0.5+) - Benchmarking framework

### Rust Edition and Features
- **Edition**: Rust 2024 for latest language features
- **MSRV**: 1.75.0 (latest stable as of development)
- **Key Features**: async/await, const generics, pattern matching

## 4. Data Architecture

### Core Data Models

```rust
// Core file representation
pub struct LogFile {
    path: PathBuf,
    size: u64,
    compression: CompressionType,
    access_strategy: AccessStrategy,
}

// Search result representation
pub struct SearchMatch {
    line_number: u64,
    byte_offset: u64,
    match_text: String,
    context: Option<MatchContext>,
}

// UI state management
pub struct ViewState {
    current_line: u64,
    visible_lines: u32,
    search_matches: Vec<SearchMatch>,
    current_match_index: Option<usize>,
}
```

### Data Flow Patterns

1. **File Loading Flow**:
   ```
   File Path → Format Detection → Compression Check → Access Strategy Selection → Memory Mapping/Streaming Setup
   ```

2. **Search Flow**:
   ```
   Search Pattern → Pattern Validation → ripgrep Execution → Result Parsing → Match Indexing → UI Update
   ```

3. **Navigation Flow**:
   ```
   User Input → Command Translation → State Update → Buffer Management → Screen Rendering
   ```

## 5. Component Design

### 5.1 File Handler Module (`file_handler.rs`)

**Priority**: P0 Essential
**Responsibilities**: File access, compression handling, memory management

```rust
pub trait FileAccessor {
    async fn read_range(&self, start: u64, length: usize) -> Result<Vec<u8>>;
    async fn read_line(&self, line_number: u64) -> Result<String>;
    fn file_size(&self) -> u64;
    fn total_lines(&self) -> Option<u64>;
}

pub enum AccessStrategy {
    MemoryMapped(Mmap),
    Streaming(Box<dyn AsyncRead + Unpin>),
    Hybrid { mmap: Mmap, stream: Box<dyn AsyncRead + Unpin> },
}

pub enum CompressionType {
    None,
    Gzip,
    Bzip2,
    Xz,
}
```

**Key Implementation Details:**
- Automatic strategy selection based on file size and system memory
- Lazy decompression for compressed files
- Buffer pooling for memory efficiency
- Error recovery for corrupted files

**Testing Strategy:**
- Unit tests for each compression format
- Property tests for large file handling
- Memory usage benchmarks

### 5.2 Search Engine Module (`search.rs`)

**Priority**: P0 Essential
**Responsibilities**: Pattern matching, result management, search optimization

```rust
pub trait SearchEngine {
    async fn search(&self, pattern: &str, options: SearchOptions) -> Result<Vec<SearchMatch>>;
    async fn search_next(&self, from_position: u64) -> Result<Option<SearchMatch>>;
    async fn search_previous(&self, from_position: u64) -> Result<Option<SearchMatch>>;
}

pub struct RipgrepEngine {
    file_accessor: Arc<dyn FileAccessor>,
    result_cache: LruCache<String, Vec<SearchMatch>>,
}

pub struct SearchOptions {
    case_sensitive: bool,
    whole_word: bool,
    regex_mode: bool,
    context_lines: u8,
}
```

**Key Implementation Details:**
- Direct integration with ripgrep-core library
- LRU cache for frequent searches
- Progressive search with early termination
- ReDoS protection with timeout mechanisms

**Testing Strategy:**
- Regex pattern validation tests
- Performance benchmarks on large files
- Edge case testing (Unicode, binary data)

### 5.3 UI Manager Module (`ui.rs`)

**Priority**: P0 Essential
**Responsibilities**: Terminal interface, event handling, rendering

```rust
pub trait UIRenderer {
    fn render(&mut self, state: &ViewState) -> Result<()>;
    fn handle_event(&mut self, event: CrosstermEvent) -> Result<UICommand>;
    fn resize(&mut self, width: u16, height: u16) -> Result<()>;
}

pub enum UICommand {
    Navigation(NavigationCommand),
    Search(SearchCommand),
    Quit,
    NoOp,
}

pub struct TerminalUI {
    terminal: Terminal<CrosstermBackend<io::Stdout>>,
    current_state: ViewState,
    key_bindings: HashMap<KeyEvent, UICommand>,
}
```

**Key Implementation Details:**
- Event-driven architecture with command pattern
- Efficient diff-based rendering
- Configurable key bindings
- Responsive layout for different terminal sizes

**Testing Strategy:**
- Mock terminal testing for rendering logic
- Integration tests for key binding functionality
- Visual regression tests for layout

### 5.4 Application Core Module (`app.rs`)

**Priority**: P0 Essential
**Responsibilities**: State coordination, error handling, business logic

```rust
pub struct Application {
    file_handler: Box<dyn FileAccessor>,
    search_engine: Box<dyn SearchEngine>,
    ui_manager: Box<dyn UIRenderer>,
    state: ApplicationState,
}

pub struct ApplicationState {
    current_file: Option<LogFile>,
    view_state: ViewState,
    search_state: SearchState,
    error_state: Option<ErrorState>,
}

impl Application {
    pub async fn run(&mut self) -> Result<()>;
    pub async fn handle_command(&mut self, command: UICommand) -> Result<()>;
    pub async fn load_file(&mut self, path: PathBuf) -> Result<()>;
}
```

**Key Implementation Details:**
- Central event loop with async coordination
- State management with atomic operations
- Graceful error recovery and user feedback
- Plugin architecture for future extensibility

**Testing Strategy:**
- Integration tests for complete workflows
- State transition testing
- Error handling validation

## 6. API Specifications

### 6.1 Command-Line Interface

```bash
# Basic usage
rlless /path/to/large/file.log

# With compression
rlless /path/to/compressed/file.log.gz

# With initial search
rlless --search "ERROR" /path/to/file.log

# Memory limit override
rlless --max-memory 200M /path/to/file.log

# Configuration file
rlless --config ~/.rlless.toml /path/to/file.log
```

### 6.2 Key Bindings (Less-Compatible)

```
Navigation:
  j, ↓          - Move down one line
  k, ↑          - Move up one line
  Space, PgDn   - Move down one page
  b, PgUp       - Move up one page
  g             - Go to beginning of file
  G             - Go to end of file

Search:
  /             - Search forward
  ?             - Search backward
  n             - Next match
  N             - Previous match
  *             - Search for word under cursor

Control:
  q             - Quit application
  h             - Show help
  :             - Command mode (future)
```

### 6.3 Configuration Format

```toml
# ~/.rlless.toml
[performance]
max_memory_mb = 100
buffer_size_kb = 64
enable_simd = true

[ui]
show_line_numbers = true
highlight_matches = true
context_lines = 2

[search]
case_sensitive = false
regex_mode = true
max_results = 10000

[compression]
auto_detect = true
temp_dir = "/tmp/rlless"
```

## 7. Project Structure

```
rlless/
├── Cargo.toml                    # Project configuration
├── src/
│   ├── main.rs                   # Application entry point
│   ├── app.rs                    # Application core logic
│   ├── cli.rs                    # Command-line interface
│   ├── config.rs                 # Configuration management
│   ├── error.rs                  # Error types and handling
│   ├── file_handler.rs           # File access and management
│   ├── search.rs                 # Search engine integration
│   ├── ui.rs                     # Terminal user interface
│   ├── compression.rs            # Compression format support
│   ├── buffer.rs                 # Buffer management utilities
│   └── utils.rs                  # Utility functions
├── tests/
│   ├── integration/
│   │   ├── large_file_tests.rs   # Large file handling tests
│   │   ├── search_tests.rs       # Search functionality tests
│   │   └── ui_tests.rs           # UI interaction tests
│   └── fixtures/                 # Test data files
├── benches/
│   ├── file_access.rs            # File access benchmarks
│   ├── search_performance.rs     # Search performance benchmarks
│   └── memory_usage.rs           # Memory usage benchmarks
├── examples/
│   ├── basic_usage.rs            # Basic usage examples
│   └── advanced_features.rs     # Advanced feature examples
└── docs/                         # Additional documentation
    ├── ARCHITECTURE.md           # Detailed architecture docs
    ├── PERFORMANCE.md            # Performance tuning guide
    └── TROUBLESHOOTING.md        # Common issues and solutions
```

### Module Organization Principles

1. **No mod.rs Files**: Each module is self-contained in its own file
2. **Clear Boundaries**: Each module has a single responsibility
3. **Minimal Dependencies**: Modules depend on traits, not concrete types
4. **Easy Testing**: Each module can be tested in isolation
5. **Future-Proof**: Clean interfaces allow for implementation swapping

## 8. Testing Strategy

### 8.1 Testing Pyramid

**Unit Tests (70% of test effort)**
- Each module tested in isolation
- Focus on public interfaces and error conditions
- Property-based testing for complex algorithms
- Memory safety and performance regression tests

**Integration Tests (25% of test effort)**
- Complete workflows with real file data
- Cross-module interaction validation
- Performance benchmarks with large files
- Error recovery scenarios

**End-to-End Tests (5% of test effort)**
- Full application testing with terminal simulation
- User workflow validation
- Platform compatibility testing

### 8.2 Test Data Strategy

```
tests/fixtures/
├── small/
│   ├── plain.log              # Small plain text file
│   ├── binary.log             # File with binary data
│   └── unicode.log            # File with Unicode content
├── medium/
│   ├── apache.log.gz          # Compressed Apache log
│   ├── json.log               # JSON formatted logs
│   └── multiline.log          # Logs with stack traces
├── large/
│   ├── generate_large.sh      # Script to generate large files
│   └── synthetic_40gb.log     # Generated 40GB test file
└── corrupt/
    ├── truncated.log.gz       # Corrupted compressed file
    └── invalid_utf8.log       # Invalid UTF-8 sequences
```

### 8.3 Performance Testing

**Benchmark Targets:**
- File opening: <2 seconds for 40GB files
- Search response: <500ms for 40GB files
- Memory usage: <100MB regardless of file size
- Navigation response: <50ms for any operation

**Benchmark Implementation:**
```rust
// benches/file_access.rs
use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn bench_large_file_open(c: &mut Criterion) {
    c.bench_function("open_40gb_file", |b| {
        b.iter(|| {
            // Benchmark file opening
        });
    });
}

criterion_group!(benches, bench_large_file_open);
criterion_main!(benches);
```

## 9. Security Design

### 9.1 Security Principles

**Read-Only Operation:**
- No file modification capabilities
- No temporary file creation without explicit user consent
- Sandboxed execution environment

**Input Validation:**
- Regex pattern sanitization to prevent ReDoS attacks
- File path validation to prevent directory traversal
- Memory allocation limits to prevent DoS

**Memory Safety:**
- Rust's ownership system prevents buffer overflows
- Bounds checking for all array/vector access
- Safe handling of memory-mapped files

### 9.2 Security Implementation

```rust
// Regex timeout protection
pub fn safe_regex_search(pattern: &str, timeout: Duration) -> Result<Regex> {
    let regex = Regex::new(pattern)?;
    // Implement timeout mechanism
    Ok(regex)
}

// Path validation
pub fn validate_file_path(path: &Path) -> Result<PathBuf> {
    let canonical = path.canonicalize()?;
    if !canonical.is_file() {
        return Err(Error::InvalidPath);
    }
    Ok(canonical)
}
```

## 10. Deployment Architecture

### 10.1 Build Configuration

```toml
# Cargo.toml
[profile.release]
lto = true                # Link-time optimization
codegen-units = 1        # Single codegen unit for optimal performance
panic = "abort"          # Reduce binary size
strip = true             # Strip debug symbols

[profile.dev]
opt-level = 1           # Some optimization for development
debug = true            # Full debug info for development

[features]
default = ["compression", "simd"]
compression = ["flate2", "bzip2", "xz2"]
simd = ["ripgrep/simd-accel"]
mimalloc = ["dep:mimalloc"]  # Alternative allocator for performance
```

### 10.2 Platform Support

**Tier 1 Platforms (Full support and testing):**
- Linux x86_64 (Ubuntu 20.04+, RHEL 8+)
- macOS x86_64 and ARM64 (macOS 11+)
- Windows x86_64 (Windows 10+)

**Tier 2 Platforms (Best effort):**
- Linux ARM64
- FreeBSD x86_64

### 10.3 Distribution Strategy

**Package Managers:**
- Cargo: `cargo install rlless`
- Homebrew: `brew install rlless`
- APT: `sudo apt install rlless`
- Chocolatey: `choco install rlless`

**Binary Releases:**
- GitHub Releases with automated builds
- Platform-specific optimized binaries
- Checksums and signatures for verification

## 11. Performance Specifications

### 11.1 Performance Targets

| Metric | Target | Measurement Method |
|--------|--------|--------------------|
| File Open Time | <2 seconds (40GB) | Time from CLI execution to first content display |
| Search Response | <500ms (40GB) | Time from search input to first result highlight |
| Memory Usage | <100MB | Peak RSS during any operation |
| Navigation Response | <50ms | Time from key press to screen update |
| Startup Time | <100ms | Time from CLI execution to interactive state |

### 11.2 Optimization Strategies

**Memory Optimization:**
- Memory mapping for large files with lazy loading
- Buffer pooling to reduce allocations
- Streaming decompression to avoid full file expansion
- LRU cache eviction for search results

**CPU Optimization:**
- SIMD instructions via ripgrep integration
- Multi-threaded search for very large files
- Lazy computation for non-visible content
- Efficient data structures (B-trees for line indexing)

**I/O Optimization:**
- Async I/O for non-blocking operations
- Read-ahead buffering for sequential access
- Memory-mapped files for random access patterns
- Compression format-specific optimizations

### 11.3 Performance Monitoring

```rust
// Built-in performance metrics
pub struct PerformanceMetrics {
    pub file_open_time: Duration,
    pub search_time: Duration,
    pub memory_usage: usize,
    pub cache_hit_rate: f64,
}

impl Application {
    pub fn get_metrics(&self) -> PerformanceMetrics {
        // Implementation
    }
}
```

## 12. Risk Analysis

### 12.1 Technical Risks

**High Impact Risks:**

**R-001: Memory Mapping Failures on Large Files**
- **Probability**: Medium
- **Impact**: High (Core functionality broken)
- **Mitigation**: Automatic fallback to streaming mode, memory availability detection
- **Contingency**: Pure streaming implementation as backup

**R-002: ripgrep Integration Complexity**
- **Probability**: Low
- **Impact**: High (Search performance degraded)
- **Mitigation**: Use stable ripgrep-core APIs, maintain fallback regex engine
- **Contingency**: Custom SIMD implementation or alternative search library

**Medium Impact Risks:**

**R-003: Performance Degradation with Pathological Inputs**
- **Probability**: Medium
- **Impact**: Medium (Poor user experience)
- **Mitigation**: Input validation, timeout mechanisms, progress indicators
- **Contingency**: Graceful degradation and user warnings

**R-004: Cross-Platform Compatibility Issues**
- **Probability**: Medium
- **Impact**: Medium (Limited platform support)
- **Mitigation**: Comprehensive testing matrix, platform-specific optimizations
- **Contingency**: Tier-based platform support strategy

### 12.2 Development Risks

**R-005: Complex Architecture Over-Engineering**
- **Probability**: Medium
- **Impact**: Medium (Development velocity reduced)
- **Mitigation**: Start with MVP, iterative complexity introduction
- **Contingency**: Simplification refactoring if needed

**R-006: Testing Infrastructure Complexity**
- **Probability**: Low
- **Impact**: Medium (Quality assurance challenges)
- **Mitigation**: Simple test data generation, automated benchmarking
- **Contingency**: Manual testing procedures as backup

### 12.3 Risk Monitoring

```rust
// Built-in risk detection
pub struct RiskMonitor {
    memory_usage_threshold: usize,
    search_timeout_threshold: Duration,
    error_rate_threshold: f64,
}

impl RiskMonitor {
    pub fn check_memory_pressure(&self) -> RiskLevel;
    pub fn check_performance_degradation(&self) -> RiskLevel;
    pub fn generate_risk_report(&self) -> RiskReport;
}
```

## Feature Priority Matrix

### P0 - Essential (MVP Delivery)
- [x] Large file handling (40GB+)
- [x] Basic less-like navigation (j/k, Space, g/G)
- [x] SIMD-optimized search integration
- [x] Bidirectional search navigation (n/N)
- [x] Compression support (gzip, bzip2, xz)
- [x] Memory efficiency (<100MB usage)
- [x] Cross-platform terminal interface

### P1 - Important (Post-MVP)
- [ ] Advanced search features (ranges, bookmarks)
- [ ] File monitoring (tail -f behavior)
- [ ] Multi-file support
- [ ] Configuration system
- [ ] Enhanced error handling and recovery

### P2 - Nice-to-have (Future)
- [ ] Syntax highlighting for log formats
- [ ] Custom filtering rules
- [ ] Plugin architecture
- [ ] Network file system optimization
- [ ] Search result export

---

This technical design specification provides a complete roadmap for implementing rlless while adhering to the simplicity and modularity principles requested. The architecture balances high-performance requirements with maintainable, testable code that can evolve over time.

The design prioritizes essential functionality (P0) while providing clear interfaces for future enhancement. Each module can be developed and tested independently, supporting iterative development and reducing integration complexity.