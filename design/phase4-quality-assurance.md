# Phase 4: Configuration & Quality Assurance

**Phase Goal**: Configuration system, comprehensive testing, and performance validation to ensure production readiness.

**Priority**: P1 Important  
**Tasks**: 12-15  
**Dependencies**: Phase 3 Complete (MVP Working)  
**Estimated Duration**: 3-4 days  

## Overview

Phase 4 transforms the working MVP into a production-ready application with comprehensive testing, performance validation, and user customization capabilities. This phase ensures reliability, maintainability, and optimal performance for the target use cases.

## Tasks

### **Task 12: Implement configuration system with TOML support**

**Priority**: P1 Important  
**Scope**: Configuration loading, default values, user customization  
**Inputs**: Configuration format from design specifications  
**Outputs**: config.rs module with configuration management  

**Acceptance Criteria**:
- TOML configuration file support (~/.rlless.toml)
- All configuration options from design (performance, UI, search, compression)
- Default value fallbacks when config missing
- Runtime configuration updates where applicable
- Configuration validation and error reporting

**Dependencies**: Task 2  
**Estimated Effort**: Simple (4-5 hours)  

**Technical Notes**: 
- Enables user customization; design for future extensibility
- Validate configuration values for safety
- Hot-reload support where possible

**Implementation Details**:

**Configuration Structure**:
```rust
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    pub performance: PerformanceConfig,
    pub ui: UIConfig,
    pub search: SearchConfig,
    pub compression: CompressionConfig,
    pub key_bindings: Option<KeyBindingsConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PerformanceConfig {
    pub max_memory_mb: usize,
    pub buffer_size_kb: usize,
    pub enable_simd: bool,
    pub search_timeout_ms: u64,
    pub cache_size: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct UIConfig {
    pub show_line_numbers: bool,
    pub highlight_matches: bool,
    pub context_lines: u8,
    pub wrap_lines: bool,
    pub status_line: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SearchConfig {
    pub case_sensitive: bool,
    pub regex_mode: bool,
    pub max_results: usize,
    pub incremental_search: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CompressionConfig {
    pub auto_detect: bool,
    pub temp_dir: Option<PathBuf>,
    pub decompress_buffer_size: usize,
}
```

**Configuration Loading**:
```rust
impl Config {
    pub fn load() -> Result<Self> {
        let config_path = Self::get_config_path()?;
        
        if config_path.exists() {
            let content = std::fs::read_to_string(&config_path)?;
            let config: Config = toml::from_str(&content)
                .map_err(|e| RllessError::ConfigError(format!("Invalid TOML: {}", e)))?;
            
            config.validate()?;
            Ok(config)
        } else {
            Ok(Self::default())
        }
    }
    
    pub fn get_config_path() -> Result<PathBuf> {
        let home_dir = dirs::home_dir()
            .ok_or_else(|| RllessError::ConfigError("Cannot determine home directory".to_string()))?;
        Ok(home_dir.join(".rlless.toml"))
    }
    
    pub fn validate(&self) -> Result<()> {
        if self.performance.max_memory_mb == 0 {
            return Err(RllessError::ConfigError("max_memory_mb must be greater than 0".to_string()));
        }
        
        if self.search.max_results == 0 {
            return Err(RllessError::ConfigError("max_results must be greater than 0".to_string()));
        }
        
        if self.ui.context_lines > 50 {
            return Err(RllessError::ConfigError("context_lines cannot exceed 50".to_string()));
        }
        
        Ok(())
    }
    
    pub fn save(&self) -> Result<()> {
        let config_path = Self::get_config_path()?;
        let content = toml::to_string_pretty(self)
            .map_err(|e| RllessError::ConfigError(format!("Failed to serialize config: {}", e)))?;
        
        std::fs::write(&config_path, content)
            .map_err(|e| RllessError::ConfigError(format!("Failed to write config: {}", e)))?;
        
        Ok(())
    }
}

impl Default for Config {
    fn default() -> Self {
        Config {
            performance: PerformanceConfig {
                max_memory_mb: 100,
                buffer_size_kb: 64,
                enable_simd: true,
                search_timeout_ms: 5000,
                cache_size: 1000,
            },
            ui: UIConfig {
                show_line_numbers: true,
                highlight_matches: true,
                context_lines: 2,
                wrap_lines: false,
                status_line: true,
            },
            search: SearchConfig {
                case_sensitive: false,
                regex_mode: true,
                max_results: 10000,
                incremental_search: false,
            },
            compression: CompressionConfig {
                auto_detect: true,
                temp_dir: None, // Use system temp dir
                decompress_buffer_size: 8192,
            },
            key_bindings: None, // Use defaults
        }
    }
}
```

**Integration with Application**:
```rust
impl Application {
    pub async fn new_with_config(file_path: PathBuf, config: Config) -> Result<Self> {
        // Apply configuration to components
        let file_accessor = MmapFileAccessor::new_with_config(&file_path, &config.performance).await?;
        let search_engine = RipgrepEngine::new_with_config(Arc::new(file_accessor), &config.search)?;
        let ui_manager = TerminalUI::new_with_config(&config.ui)?;
        
        Ok(Application {
            file_handler: Box::new(file_accessor),
            search_engine: Box::new(search_engine),
            ui_manager: Box::new(ui_manager),
            state: ApplicationState::new(),
            config,
        })
    }
}
```

---

### **Task 13: Add comprehensive unit tests for all core modules**

**Priority**: P1 Important  
**Scope**: Unit testing for all modules, test infrastructure  
**Inputs**: Testing strategy from design  
**Outputs**: Complete unit test suite with >80% coverage  

**Acceptance Criteria**:
- Unit tests for each module's public interface
- Property-based tests for complex algorithms using proptest
- Mock implementations for trait testing
- Error condition testing for all modules
- Test coverage reporting setup
- All tests pass with `cargo test`

**Dependencies**: Tasks 1-11  
**Estimated Effort**: Medium (8-10 hours)  

**Technical Notes**: 
- Test-driven approach; focus on public interfaces and error paths
- Use dependency injection for testability
- Comprehensive edge case coverage

**Implementation Details**:

**File Handler Tests**:
```rust
// src/file_handler.rs - Unit tests
#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use tempfile::NamedTempFile;
    
    #[tokio::test]
    async fn test_mmap_file_accessor_basic() {
        let temp_file = NamedTempFile::new().unwrap();
        let content = "line1\nline2\nline3\n";
        std::fs::write(temp_file.path(), content).unwrap();
        
        let accessor = MmapFileAccessor::new(temp_file.path()).await.unwrap();
        
        assert_eq!(accessor.file_size(), content.len() as u64);
        assert_eq!(accessor.total_lines(), Some(3));
        
        let line = accessor.read_line(0).await.unwrap();
        assert_eq!(line, "line1");
        
        let range = accessor.read_range(0, 5).await.unwrap();
        assert_eq!(range, b"line1");
    }
    
    #[tokio::test]
    async fn test_compression_detection() {
        // Test gzip detection
        let temp_file = NamedTempFile::with_suffix(".gz").unwrap();
        let compression = detect_compression(temp_file.path()).unwrap();
        assert_eq!(compression, CompressionType::Gzip);
        
        // Test plain file
        let temp_file = NamedTempFile::with_suffix(".log").unwrap();
        let compression = detect_compression(temp_file.path()).unwrap();
        assert_eq!(compression, CompressionType::None);
    }
    
    proptest! {
        #[test]
        fn test_large_file_access(
            file_size in 1024usize..10_000_000,
            read_offset in 0u64..1000,
            read_length in 1usize..1024
        ) {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let temp_file = NamedTempFile::new().unwrap();
                let content = "x".repeat(file_size);
                std::fs::write(temp_file.path(), &content).unwrap();
                
                let accessor = MmapFileAccessor::new(temp_file.path()).await.unwrap();
                
                let actual_offset = read_offset.min(file_size as u64 - 1);
                let actual_length = read_length.min(file_size - actual_offset as usize);
                
                let result = accessor.read_range(actual_offset, actual_length).await;
                prop_assert!(result.is_ok());
                prop_assert_eq!(result.unwrap().len(), actual_length);
            });
        }
    }
}
```

**Search Engine Tests**:
```rust
// src/search.rs - Unit tests
#[cfg(test)]
mod tests {
    use super::*;
    use mockall::mock;
    
    mock! {
        TestFileAccessor {}
        
        #[async_trait]
        impl FileAccessor for TestFileAccessor {
            async fn read_range(&self, start: u64, length: usize) -> Result<Vec<u8>>;
            async fn read_line(&self, line_number: u64) -> Result<String>;
            fn file_size(&self) -> u64;
            fn total_lines(&self) -> Option<u64>;
        }
    }
    
    #[tokio::test]
    async fn test_ripgrep_search_basic() {
        let mut mock_accessor = MockTestFileAccessor::new();
        mock_accessor
            .expect_read_range()
            .returning(|_, _| Ok(b"line1 ERROR test\nline2 normal\nline3 ERROR again\n".to_vec()));
        mock_accessor
            .expect_file_size()
            .returning(|| 100);
        
        let engine = RipgrepEngine::new(Arc::new(mock_accessor)).unwrap();
        let options = SearchOptions::default();
        
        let matches = engine.search("ERROR", options).await.unwrap();
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].line_number, 0);
        assert_eq!(matches[1].line_number, 2);
    }
    
    #[tokio::test]
    async fn test_search_caching() {
        let mut mock_accessor = MockTestFileAccessor::new();
        mock_accessor
            .expect_read_range()
            .times(1) // Should only be called once due to caching
            .returning(|_, _| Ok(b"test ERROR content\n".to_vec()));
        
        let engine = RipgrepEngine::new(Arc::new(mock_accessor)).unwrap();
        let options = SearchOptions::default();
        
        // First search
        let matches1 = engine.search("ERROR", options.clone()).await.unwrap();
        // Second search (should use cache)
        let matches2 = engine.search("ERROR", options).await.unwrap();
        
        assert_eq!(matches1.len(), matches2.len());
    }
    
    #[tokio::test]
    async fn test_search_timeout() {
        let mut mock_accessor = MockTestFileAccessor::new();
        mock_accessor
            .expect_read_range()
            .returning(|_, _| {
                // Simulate slow operation
                std::thread::sleep(Duration::from_millis(100));
                Ok(vec![])
            });
        
        let mut engine = RipgrepEngine::new(Arc::new(mock_accessor)).unwrap();
        engine.set_timeout(Duration::from_millis(50));
        
        let result = engine.search("test", SearchOptions::default()).await;
        assert!(result.is_err());
        // Should be timeout error
    }
}
```

**UI Tests**:
```rust
// src/ui.rs - Unit tests
#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    
    #[test]
    fn test_key_binding_translation() {
        let ui = TerminalUI::new().unwrap();
        
        let j_key = KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE);
        let command = ui.translate_key_event(j_key).unwrap();
        
        match command {
            UICommand::Navigation(NavigationCommand::LineDown) => {},
            _ => panic!("Expected LineDown command"),
        }
    }
    
    #[test]
    fn test_search_prompt_handling() {
        let mut ui = TerminalUI::new().unwrap();
        
        // Test search mode entry
        ui.enter_search_mode(true).unwrap();
        assert!(ui.is_in_search_mode());
        
        // Test pattern building
        ui.add_search_char('t').unwrap();
        ui.add_search_char('e').unwrap();
        ui.add_search_char('s').unwrap();
        ui.add_search_char('t').unwrap();
        
        assert_eq!(ui.get_current_search_pattern(), "test");
        
        // Test backspace
        ui.backspace_search().unwrap();
        assert_eq!(ui.get_current_search_pattern(), "tes");
    }
    
    #[test]
    fn test_viewport_calculation() {
        let ui = TerminalUI::new().unwrap();
        let state = ViewState {
            current_line: 100,
            visible_lines: 20,
            search_matches: vec![],
            current_match_index: None,
            content_buffer: vec![],
        };
        
        let viewport = ui.calculate_viewport(&state);
        assert_eq!(viewport.start_line, 100);
        assert_eq!(viewport.end_line, 120);
    }
}
```

**Test Infrastructure Setup**:
```rust
// tests/common/mod.rs - Shared test utilities
pub mod fixtures {
    use std::path::PathBuf;
    use tempfile::{NamedTempFile, TempDir};
    
    pub struct TestFile {
        pub file: NamedTempFile,
        pub path: PathBuf,
        pub content: String,
    }
    
    impl TestFile {
        pub fn new(content: &str) -> Self {
            let file = NamedTempFile::new().unwrap();
            std::fs::write(file.path(), content).unwrap();
            
            TestFile {
                path: file.path().to_path_buf(),
                file,
                content: content.to_string(),
            }
        }
        
        pub fn large_file(size_mb: usize) -> Self {
            let content = generate_log_content(size_mb);
            Self::new(&content)
        }
    }
    
    pub fn generate_log_content(size_mb: usize) -> String {
        let line = "2024-01-01 12:00:00 INFO Application started\n";
        let lines_needed = (size_mb * 1024 * 1024) / line.len();
        line.repeat(lines_needed)
    }
}
```

---

### **Task 14: Create integration tests for large file handling**

**Priority**: P1 Important  
**Scope**: End-to-end testing with real large files, workflow validation  
**Inputs**: Integration testing requirements  
**Outputs**: Integration test suite in tests/ directory  

**Acceptance Criteria**:
- Tests with generated large files (multi-GB)
- Complete workflow testing (file loading, search, navigation)
- Performance regression tests
- Cross-module interaction validation
- Compressed file format testing
- Memory usage validation during tests

**Dependencies**: Task 13  
**Estimated Effort**: Medium (6-8 hours)  

**Technical Notes**: 
- Performance-critical testing; may require CI optimization
- Use generated test files to avoid repository bloat
- Focus on realistic user scenarios

**Implementation Details**:

**Large File Integration Tests**:
```rust
// tests/integration/large_file_tests.rs
use rlless::{Application, Config};
use std::time::{Duration, Instant};
use sysinfo::{System, SystemExt, ProcessExt};

#[tokio::test]
#[ignore] // Run with --ignored for performance tests
async fn test_large_file_opening_performance() {
    // Generate 1GB test file
    let test_file = create_test_file_gb(1);
    
    let start = Instant::now();
    let app = Application::new(test_file.path.clone(), Config::default()).await;
    let duration = start.elapsed();
    
    assert!(app.is_ok());
    assert!(duration < Duration::from_secs(2), 
           "File opening took {:?}, expected <2s", duration);
}

#[tokio::test]
#[ignore]
async fn test_search_performance_large_file() {
    let test_file = create_test_file_gb(2);
    let mut app = Application::new(test_file.path.clone(), Config::default()).await.unwrap();
    
    let start = Instant::now();
    let matches = app.search("ERROR".to_string()).await.unwrap();
    let duration = start.elapsed();
    
    assert!(!matches.is_empty());
    assert!(duration < Duration::from_millis(500),
           "Search took {:?}, expected <500ms", duration);
}

#[tokio::test]
async fn test_memory_usage_large_file() {
    let test_file = create_test_file_gb(5); // 5GB file
    let mut system = System::new_all();
    
    let pid = std::process::id();
    
    // Baseline memory
    system.refresh_process(pid.into());
    let initial_memory = system.process(pid.into()).unwrap().memory();
    
    // Load large file
    let _app = Application::new(test_file.path.clone(), Config::default()).await.unwrap();
    
    // Check memory after loading
    system.refresh_process(pid.into());
    let final_memory = system.process(pid.into()).unwrap().memory();
    
    let memory_increase = final_memory - initial_memory;
    assert!(memory_increase < 100 * 1024 * 1024, // 100MB
           "Memory usage increased by {}MB, expected <100MB", 
           memory_increase / (1024 * 1024));
}

#[tokio::test]
async fn test_compressed_file_workflow() {
    let compressed_file = create_compressed_test_file("test_data.log.gz");
    
    let mut app = Application::new(compressed_file.path.clone(), Config::default()).await.unwrap();
    
    // Test navigation
    app.handle_navigation_command(NavigationCommand::PageDown).await.unwrap();
    app.handle_navigation_command(NavigationCommand::GoToEnd).await.unwrap();
    
    // Test search
    let matches = app.search("test".to_string()).await.unwrap();
    assert!(!matches.is_empty());
    
    // Test search navigation
    app.navigate_to_next_match().await.unwrap();
    app.navigate_to_previous_match().await.unwrap();
}

fn create_test_file_gb(size_gb: usize) -> TestFile {
    let log_line = "2024-01-01 12:00:00 INFO [main] Application started successfully with configuration loaded\n";
    let error_line = "2024-01-01 12:00:01 ERROR [worker] Failed to process request: connection timeout\n";
    
    let mut content = String::new();
    let target_size = size_gb * 1024 * 1024 * 1024;
    let line_size = log_line.len();
    let lines_needed = target_size / line_size;
    
    for i in 0..lines_needed {
        if i % 100 == 0 {
            content.push_str(error_line);
        } else {
            content.push_str(log_line);
        }
    }
    
    TestFile::new(&content)
}

fn create_compressed_test_file(filename: &str) -> TestFile {
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use std::io::Write;
    
    let content = "line1\ntest line with pattern\nline3\n".repeat(1000);
    
    let temp_file = NamedTempFile::with_suffix(".gz").unwrap();
    let file = std::fs::File::create(temp_file.path()).unwrap();
    let mut encoder = GzEncoder::new(file, Compression::default());
    encoder.write_all(content.as_bytes()).unwrap();
    encoder.finish().unwrap();
    
    TestFile {
        file: temp_file,
        path: temp_file.path().to_path_buf(),
        content,
    }
}
```

**Complete Workflow Tests**:
```rust
// tests/integration/workflow_tests.rs
use rlless::{Application, Config, UICommand, NavigationCommand, SearchCommand};

#[tokio::test]
async fn test_complete_viewing_workflow() {
    let test_file = create_medium_test_file();
    let mut app = Application::new(test_file.path.clone(), Config::default()).await.unwrap();
    
    // Basic navigation workflow
    let commands = vec![
        UICommand::Navigation(NavigationCommand::LineDown),
        UICommand::Navigation(NavigationCommand::LineDown),
        UICommand::Navigation(NavigationCommand::PageDown),
        UICommand::Navigation(NavigationCommand::GoToBeginning),
        UICommand::Navigation(NavigationCommand::GoToEnd),
    ];
    
    for command in commands {
        app.handle_command(command).await.unwrap();
    }
    
    assert!(app.get_current_line() > 0);
}

#[tokio::test]
async fn test_search_workflow() {
    let test_file = create_test_file_with_patterns();
    let mut app = Application::new(test_file.path.clone(), Config::default()).await.unwrap();
    
    // Search workflow
    app.handle_command(UICommand::Search(SearchCommand::SearchForward("ERROR".to_string()))).await.unwrap();
    
    let matches = app.get_search_matches();
    assert!(!matches.is_empty());
    
    // Navigate through matches
    app.handle_command(UICommand::Search(SearchCommand::NextMatch)).await.unwrap();
    app.handle_command(UICommand::Search(SearchCommand::PreviousMatch)).await.unwrap();
    
    // New search
    app.handle_command(UICommand::Search(SearchCommand::SearchBackward("WARN".to_string()))).await.unwrap();
    
    let warn_matches = app.get_search_matches();
    assert!(!warn_matches.is_empty());
}

fn create_test_file_with_patterns() -> TestFile {
    let content = r#"
2024-01-01 10:00:00 INFO Application starting
2024-01-01 10:00:01 ERROR Database connection failed
2024-01-01 10:00:02 INFO Retrying connection
2024-01-01 10:00:03 WARN Connection unstable
2024-01-01 10:00:04 INFO Connection restored
2024-01-01 10:00:05 ERROR Authentication failed
"#;
    TestFile::new(content)
}
```

---

### **Task 15: Set up performance benchmarks and memory usage validation**

**Priority**: P1 Important  
**Scope**: Performance benchmarking, memory profiling, target validation  
**Inputs**: Performance specifications from design  
**Outputs**: Benchmark suite in benches/ directory  

**Acceptance Criteria**:
- File opening benchmarks (<2 seconds for 40GB files)
- Search performance benchmarks (<500ms response)
- Memory usage validation (<100MB peak usage)
- Navigation responsiveness benchmarks (<50ms)
- Continuous benchmark tracking setup
- Benchmark results reporting

**Dependencies**: Task 14  
**Estimated Effort**: Medium (6-8 hours)  

**Technical Notes**: 
- Essential for performance targets; automate in CI
- Use realistic data patterns for benchmarks
- Track performance regression over time

**Implementation Details**:

**File Access Benchmarks**:
```rust
// benches/file_access.rs
use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId};
use rlless::file_handler::MmapFileAccessor;
use std::time::Duration;
use tokio::runtime::Runtime;

fn bench_file_opening(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let mut group = c.benchmark_group("file_opening");
    
    // Test different file sizes
    for size_mb in [1, 10, 100, 1000, 5000].iter() {
        let test_file = create_test_file_mb(*size_mb);
        
        group.bench_with_input(
            BenchmarkId::new("mmap_open", format!("{}MB", size_mb)),
            size_mb,
            |b, _| {
                b.to_async(&rt).iter(|| async {
                    let accessor = MmapFileAccessor::new(black_box(&test_file.path)).await.unwrap();
                    black_box(accessor);
                });
            },
        );
    }
    
    group.finish();
}

fn bench_line_reading(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let test_file = create_test_file_mb(100);
    let accessor = rt.block_on(async {
        MmapFileAccessor::new(&test_file.path).await.unwrap()
    });
    
    c.bench_function("read_line_sequential", |b| {
        b.to_async(&rt).iter(|| async {
            for line_num in 0..100 {
                let line = accessor.read_line(black_box(line_num)).await.unwrap();
                black_box(line);
            }
        });
    });
    
    c.bench_function("read_line_random", |b| {
        b.to_async(&rt).iter(|| async {
            use rand::Rng;
            let mut rng = rand::thread_rng();
            for _ in 0..100 {
                let line_num = rng.gen_range(0..1000);
                let line = accessor.read_line(black_box(line_num)).await.unwrap();
                black_box(line);
            }
        });
    });
}

criterion_group!(
    name = file_benches;
    config = Criterion::default().measurement_time(Duration::from_secs(10));
    targets = bench_file_opening, bench_line_reading
);
criterion_main!(file_benches);
```

**Search Performance Benchmarks**:
```rust
// benches/search_performance.rs
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use rlless::{search::{RipgrepEngine, SearchOptions}, file_handler::MmapFileAccessor};
use std::sync::Arc;
use tokio::runtime::Runtime;

fn bench_search_patterns(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let test_file = create_large_log_file_gb(1);
    let file_accessor = rt.block_on(async {
        Arc::new(MmapFileAccessor::new(&test_file.path).await.unwrap())
    });
    let search_engine = RipgrepEngine::new(file_accessor).unwrap();
    
    let patterns = vec![
        ("simple", "ERROR"),
        ("regex", r"\d{4}-\d{2}-\d{2}"),
        ("complex", r"ERROR.*connection.*timeout"),
    ];
    
    for (name, pattern) in patterns {
        c.bench_function(&format!("search_{}", name), |b| {
            b.to_async(&rt).iter(|| async {
                let options = SearchOptions::default();
                let matches = search_engine.search(black_box(pattern), options).await.unwrap();
                black_box(matches);
            });
        });
    }
}

fn bench_search_file_sizes(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let mut group = c.benchmark_group("search_by_file_size");
    
    for size_gb in [1, 2, 5, 10].iter() {
        let test_file = create_large_log_file_gb(*size_gb);
        let file_accessor = rt.block_on(async {
            Arc::new(MmapFileAccessor::new(&test_file.path).await.unwrap())
        });
        let search_engine = RipgrepEngine::new(file_accessor).unwrap();
        
        group.bench_with_input(
            BenchmarkId::new("search_error", format!("{}GB", size_gb)),
            size_gb,
            |b, _| {
                b.to_async(&rt).iter(|| async {
                    let options = SearchOptions::default();
                    let matches = search_engine.search(black_box("ERROR"), options).await.unwrap();
                    black_box(matches);
                });
            },
        );
    }
    
    group.finish();
}

criterion_group!(
    name = search_benches;
    config = Criterion::default()
        .measurement_time(Duration::from_secs(30))
        .sample_size(10);
    targets = bench_search_patterns, bench_search_file_sizes
);
criterion_main!(search_benches);
```

**Memory Usage Benchmarks**:
```rust
// benches/memory_usage.rs
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use rlless::Application;
use sysinfo::{System, SystemExt, ProcessExt};
use std::sync::{Arc, Mutex};

struct MemoryTracker {
    system: Arc<Mutex<System>>,
    pid: u32,
}

impl MemoryTracker {
    fn new() -> Self {
        let system = Arc::new(Mutex::new(System::new_all()));
        let pid = std::process::id();
        
        MemoryTracker { system, pid }
    }
    
    fn current_memory_mb(&self) -> f64 {
        let mut system = self.system.lock().unwrap();
        system.refresh_process(self.pid.into());
        
        if let Some(process) = system.process(self.pid.into()) {
            process.memory() as f64 / (1024.0 * 1024.0)
        } else {
            0.0
        }
    }
}

fn bench_memory_usage_large_files(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let tracker = MemoryTracker::new();
    
    let mut group = c.benchmark_group("memory_usage");
    group.sample_size(10);
    
    for size_gb in [1, 5, 10, 20, 40].iter() {
        let test_file = create_large_log_file_gb(*size_gb);
        
        group.bench_function(&format!("load_{}GB", size_gb), |b| {
            b.iter(|| {
                let initial_memory = tracker.current_memory_mb();
                
                let app = rt.block_on(async {
                    Application::new(black_box(test_file.path.clone()), Config::default()).await.unwrap()
                });
                
                let final_memory = tracker.current_memory_mb();
                let memory_increase = final_memory - initial_memory;
                
                // Validate memory constraint
                assert!(memory_increase < 100.0, 
                       "Memory usage {}MB exceeded 100MB limit for {}GB file", 
                       memory_increase, size_gb);
                
                black_box(app);
                
                memory_increase
            });
        });
    }
    
    group.finish();
}

criterion_group!(memory_benches, bench_memory_usage_large_files);
criterion_main!(memory_benches);
```

**Navigation Responsiveness Benchmarks**:
```rust
// benches/ui_responsiveness.rs
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use rlless::{Application, UICommand, NavigationCommand};
use std::time::Duration;

fn bench_navigation_commands(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let test_file = create_large_log_file_mb(100);
    let mut app = rt.block_on(async {
        Application::new(test_file.path.clone(), Config::default()).await.unwrap()
    });
    
    let commands = vec![
        ("line_down", UICommand::Navigation(NavigationCommand::LineDown)),
        ("line_up", UICommand::Navigation(NavigationCommand::LineUp)),
        ("page_down", UICommand::Navigation(NavigationCommand::PageDown)),
        ("page_up", UICommand::Navigation(NavigationCommand::PageUp)),
        ("goto_beginning", UICommand::Navigation(NavigationCommand::GoToBeginning)),
        ("goto_end", UICommand::Navigation(NavigationCommand::GoToEnd)),
    ];
    
    for (name, command) in commands {
        c.bench_function(&format!("navigation_{}", name), |b| {
            b.to_async(&rt).iter(|| async {
                let start = std::time::Instant::now();
                app.handle_command(black_box(command.clone())).await.unwrap();
                let duration = start.elapsed();
                
                // Validate responsiveness requirement
                assert!(duration < Duration::from_millis(50),
                       "Navigation command took {:?}, expected <50ms", duration);
                
                duration
            });
        });
    }
}

criterion_group!(ui_benches, bench_navigation_commands);
criterion_main!(ui_benches);
```

**Continuous Benchmarking Setup**:
```yaml
# .github/workflows/benchmarks.yml
name: Performance Benchmarks

on:
  push:
    branches: [main]
  pull_request:
    branches: [main]

jobs:
  benchmark:
    runs-on: ubuntu-latest
    
    steps:
    - uses: actions/checkout@v2
    
    - name: Install Rust
      uses: actions-rs/toolchain@v1
      with:
        toolchain: stable
        override: true
    
    - name: Run benchmarks
      run: |
        cargo bench --bench file_access -- --output-format json | tee file_bench.json
        cargo bench --bench search_performance -- --output-format json | tee search_bench.json
        cargo bench --bench memory_usage -- --output-format json | tee memory_bench.json
    
    - name: Store benchmark results
      uses: benchmark-action/github-action-benchmark@v1
      with:
        name: Rust Benchmark
        tool: 'cargo'
        output-file-path: file_bench.json
        github-token: ${{ secrets.GITHUB_TOKEN }}
        auto-push: true
```

## Phase 4 Success Criteria

By the end of Phase 4, the following should be complete:
- ✅ Configuration system allows user customization
- ✅ Comprehensive unit test suite with >80% coverage
- ✅ Integration tests validate complete workflows
- ✅ Performance benchmarks confirm all targets met:
  - File opening: <2 seconds (40GB files)
  - Search response: <500ms (40GB files)  
  - Memory usage: <100MB (any file size)
  - Navigation: <50ms response time
- ✅ Continuous benchmarking tracks performance regressions
- ✅ Production-ready quality and reliability

## Quality Gates

### Performance Validation
- All benchmarks must pass defined thresholds
- Memory usage validation across all file sizes
- Performance regression detection in CI

### Test Coverage
- Unit tests: >80% line coverage
- Integration tests cover all major workflows
- Error path testing for all modules

### Documentation
- API documentation for all public interfaces
- Usage examples and troubleshooting guides
- Performance tuning recommendations

## Final MVP Validation

Upon completion of Phase 4, the rlless MVP should:

1. **Handle Large Files**: Open and navigate 40GB+ files smoothly
2. **SIMD-Optimized Search**: Fast search with ripgrep integration
3. **Memory Efficient**: Stay under 100MB regardless of file size
4. **Responsive UI**: All operations under specified time limits
5. **Compressed File Support**: Transparent handling of gzip/bzip2/xz
6. **Configurable**: User customization via TOML configuration
7. **Well Tested**: Comprehensive test coverage and benchmarks
8. **Production Ready**: Error handling, logging, performance monitoring

## Future Enhancements (Post-MVP)

With the solid foundation from Phase 4, future development can focus on:
- **P1 Features**: File monitoring (tail -f), multi-file support
- **P2 Features**: Syntax highlighting, plugin architecture
- **Performance Optimization**: Network filesystem support
- **User Experience**: Advanced search features, bookmarks