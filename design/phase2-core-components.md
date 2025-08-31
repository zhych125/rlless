# Phase 2: Core Components

**Phase Goal**: Implementation of the three main modules (file handler, search engine, UI) and application core that form the heart of the system.

**Priority**: P0 Essential  
**Tasks**: 5-8  
**Dependencies**: Phase 1 Complete  
**Estimated Duration**: 4-5 days  

## Overview

Phase 2 builds the core functionality of rlless by implementing compression support, SIMD-optimized search, terminal UI, and application coordination. This phase delivers the essential components needed for a working MVP.

## Tasks

### **Task 5: Add compression support (gzip, bzip2, xz) to file handler**

**Priority**: P0 Essential  
**Scope**: Decompression integration, streaming decompression  
**Inputs**: Compression format specifications  
**Outputs**: Compression support integrated into FileAccessor implementations  

**Acceptance Criteria**:
- Transparent decompression for supported formats
- Streaming decompression to avoid memory explosion
- Format auto-detection from file headers and extensions
- Error handling for corrupted compressed files

**Dependencies**: Task 4  
**Estimated Effort**: Medium (6-8 hours)  

**Technical Notes**: 
- Use streaming decompression to maintain memory constraints
- Buffer management for efficient decompression
- Consider partial decompression for seek operations

**Implementation Details**:
```rust
pub struct CompressedFileAccessor {
    inner: Box<dyn FileAccessor>,
    compression: CompressionType,
    decompressor: Box<dyn AsyncRead + Unpin>,
}

impl CompressedFileAccessor {
    pub fn new(path: &Path) -> Result<Self> {
        let compression = detect_compression(path)?;
        let reader = match compression {
            CompressionType::Gzip => {
                let file = File::open(path)?;
                Box::new(GzipDecoder::new(file)) as Box<dyn AsyncRead + Unpin>
            }
            // Similar for bzip2, xz
        };
        Ok(CompressedFileAccessor { /* ... */ })
    }
}
```

**Testing Requirements**:
- Test files for each compression format
- Corrupted file handling
- Memory usage validation during decompression

---

### **Task 6: Create search engine module with ripgrep integration**

**Priority**: P0 Essential  
**Scope**: SearchEngine trait, ripgrep-core integration, result caching  
**Inputs**: Search specifications, ripgrep integration requirements  
**Outputs**: search.rs module with full search functionality  

**Acceptance Criteria**:
- SearchEngine trait with async search methods
- Direct integration with ripgrep-core for SIMD optimization
- LRU cache for search results
- SearchOptions support (case sensitivity, regex mode, context)
- ReDoS protection with timeouts
- Search performance <500ms for 40GB files

**Dependencies**: Task 2, Task 5  
**Estimated Effort**: Complex (12-15 hours)  

**Technical Notes**: 
- Most performance-critical component; SIMD optimization essential
- Careful integration with ripgrep-core APIs
- Progressive search with early termination for large files

**Implementation Details**:
```rust
#[async_trait]
pub trait SearchEngine: Send + Sync {
    async fn search(&self, pattern: &str, options: SearchOptions) -> Result<Vec<SearchMatch>>;
    async fn search_next(&self, from_position: u64) -> Result<Option<SearchMatch>>;
    async fn search_previous(&self, from_position: u64) -> Result<Option<SearchMatch>>;
}

pub struct RipgrepEngine {
    file_accessor: Arc<dyn FileAccessor>,
    result_cache: Arc<Mutex<LruCache<String, Vec<SearchMatch>>>>,
    search_timeout: Duration,
}

pub struct SearchMatch {
    pub line_number: u64,
    pub byte_offset: u64,
    pub match_text: String,
    pub context: Option<MatchContext>,
}

pub struct SearchOptions {
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub regex_mode: bool,
    pub context_lines: u8,
}
```

**Integration Strategy**:
- Use ripgrep-core for search execution
- Wrap results in SearchMatch format
- Implement bidirectional search navigation
- Cache frequently accessed patterns

**Performance Requirements**:
- Search initiation: <100ms
- Large file search: <500ms
- Memory usage: Bounded by cache size

---

### **Task 7: Implement UI module with ratatui terminal interface**

**Priority**: P0 Essential  
**Scope**: Terminal interface, event handling, rendering system  
**Inputs**: UI design specifications, key binding requirements  
**Outputs**: ui.rs module with complete terminal interface  

**Acceptance Criteria**:
- UIRenderer trait with render, handle_event, resize methods
- Event-driven architecture with UICommand pattern
- Efficient diff-based rendering with ratatui
- Responsive layout for different terminal sizes
- Key binding system matching less interface
- Navigation response time <50ms

**Dependencies**: Task 2  
**Estimated Effort**: Complex (10-12 hours)  

**Technical Notes**: 
- Focus on responsiveness; prepare for async integration
- Efficient rendering for large content
- Extensible key binding system

**Implementation Details**:
```rust
pub trait UIRenderer: Send + Sync {
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

pub enum NavigationCommand {
    LineUp,
    LineDown,
    PageUp,
    PageDown,
    GoToBeginning,
    GoToEnd,
}

pub enum SearchCommand {
    SearchForward(String),
    SearchBackward(String),
    NextMatch,
    PreviousMatch,
}

pub struct TerminalUI {
    terminal: Terminal<CrosstermBackend<io::Stdout>>,
    current_state: ViewState,
    key_bindings: HashMap<KeyEvent, UICommand>,
}
```

**Key Bindings Implementation**:
```rust
fn default_key_bindings() -> HashMap<KeyEvent, UICommand> {
    let mut bindings = HashMap::new();
    bindings.insert(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE), 
                   UICommand::Navigation(NavigationCommand::LineDown));
    bindings.insert(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE), 
                   UICommand::Navigation(NavigationCommand::LineUp));
    // ... more bindings
    bindings
}
```

**Rendering Strategy**:
- Use ratatui widgets for efficient terminal output
- Implement viewport management for large content
- Highlight search matches in display
- Status line with file information and position

---

### **Task 8: Create application core with state management and event coordination**

**Priority**: P0 Essential  
**Scope**: Central application logic, state management, component coordination  
**Inputs**: Application architecture, state management design  
**Outputs**: app.rs module with Application struct and main event loop  

**Acceptance Criteria**:
- Application struct coordinating all components via traits
- ApplicationState with atomic state management
- Central async event loop handling UI commands
- Graceful error recovery and user feedback
- File loading and management
- Startup time <100ms

**Dependencies**: Tasks 3, 6, 7  
**Estimated Effort**: Complex (8-10 hours)  

**Technical Notes**: 
- Integration complexity; careful async coordination needed
- State consistency across components
- Error propagation and user feedback

**Implementation Details**:
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

pub struct ViewState {
    current_line: u64,
    visible_lines: u32,
    search_matches: Vec<SearchMatch>,
    current_match_index: Option<usize>,
    content_buffer: Vec<String>,
}

impl Application {
    pub async fn new(file_path: PathBuf) -> Result<Self>;
    pub async fn run(&mut self) -> Result<()>;
    pub async fn handle_command(&mut self, command: UICommand) -> Result<()>;
    pub async fn load_file(&mut self, path: PathBuf) -> Result<()>;
}
```

**Main Event Loop**:
```rust
pub async fn run(&mut self) -> Result<()> {
    loop {
        // Render current state
        self.ui_manager.render(&self.state.view_state)?;
        
        // Handle events
        if crossterm::event::poll(Duration::from_millis(100))? {
            let event = crossterm::event::read()?;
            let command = self.ui_manager.handle_event(event)?;
            
            match command {
                UICommand::Quit => break,
                cmd => self.handle_command(cmd).await?,
            }
        }
    }
    Ok(())
}
```

**State Management**:
- Atomic updates for view state
- Consistent state across all components  
- Error state handling and recovery
- Performance metric tracking

## Phase 2 Integration Points

### File Handler → Search Engine
```rust
// Search engine uses file accessor for content access
let search_engine = RipgrepEngine::new(Arc::clone(&file_accessor))?;
```

### Search Engine → UI
```rust
// UI displays search results and handles navigation
pub struct ViewState {
    search_matches: Vec<SearchMatch>,
    current_match_index: Option<usize>,
}
```

### Application Core Coordination
```rust
// Central coordination of all components
async fn handle_search_command(&mut self, pattern: String) -> Result<()> {
    let matches = self.search_engine.search(&pattern, SearchOptions::default()).await?;
    self.state.view_state.search_matches = matches;
    self.state.view_state.current_match_index = Some(0);
    Ok(())
}
```

## Phase 2 Success Criteria

By the end of Phase 2, the following should be complete:
- ✅ All compression formats work transparently
- ✅ SIMD-optimized search performs within targets (<500ms)
- ✅ Terminal UI renders efficiently and responds quickly (<50ms)
- ✅ Application coordinates all components seamlessly
- ✅ Basic functionality works end-to-end
- ✅ Memory usage stays under 100MB for large files

## Testing Strategy for Phase 2

### Unit Tests Required
- **File Handler**: Compression format handling, memory mapping
- **Search Engine**: Pattern matching, result caching, error handling
- **UI Module**: Event handling, command translation, rendering
- **Application Core**: State management, component coordination

### Integration Tests Required
- **File + Search**: Search across compressed files
- **Search + UI**: Search result display and navigation
- **Full Stack**: Complete user workflows

### Performance Tests Required
- **Search Performance**: Validate <500ms target with large files
- **UI Responsiveness**: Validate <50ms navigation response
- **Memory Usage**: Validate <100MB usage constraint

## Risk Mitigation

**High-Risk Areas**:
1. **ripgrep Integration (Task 6)**: Complex API, performance critical
2. **UI Responsiveness (Task 7)**: Terminal rendering performance
3. **State Coordination (Task 8)**: Component integration complexity

**Mitigation Strategies**:
- **ripgrep**: Start with simple patterns, gradually add complexity
- **UI**: Profile rendering performance early, optimize hot paths
- **State**: Design simple state transitions, extensive testing

## Next Steps

Upon completion of Phase 2, proceed to Phase 3 (User Interface & Navigation) which will implement:
- Complete less-like navigation commands
- Search UI integration  
- Command-line interface