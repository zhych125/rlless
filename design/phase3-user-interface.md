# Phase 3: User Interface & Navigation

**Phase Goal**: Integration of user-facing features including navigation, search, and command-line interface to complete the MVP.

**Priority**: P0 Essential  
**Tasks**: 9-11  
**Dependencies**: Phase 2 Complete  
**Estimated Duration**: 2-3 days  

## Overview

Phase 3 completes the MVP by integrating all core components into a cohesive user experience. This phase implements the familiar less-like interface, bidirectional search functionality, and command-line entry point that users interact with directly.

## Tasks

### **Task 9: Implement basic less-like navigation commands (j/k, Space, g/G)**

**Priority**: P0 Essential  
**Scope**: Core navigation commands, view state management  
**Inputs**: Navigation specifications from design  
**Outputs**: Navigation command implementation in UI and app modules  

**Acceptance Criteria**:
- Line navigation (j/k, arrow keys) with immediate response
- Page navigation (Space, b, PgUp/PgDn) maintaining context
- File position navigation (g for beginning, G for end)
- Proper view state updates and screen refresh
- Navigation works smoothly with files of any size

**Dependencies**: Task 8  
**Estimated Effort**: Medium (4-6 hours)  

**Technical Notes**: 
- Foundation for all user interaction; must be highly responsive
- Efficient viewport management for large files
- State consistency during navigation

**Implementation Details**:

**Navigation Command Processing**:
```rust
impl Application {
    async fn handle_navigation(&mut self, nav_cmd: NavigationCommand) -> Result<()> {
        match nav_cmd {
            NavigationCommand::LineDown => {
                if self.state.view_state.current_line < self.get_max_line() {
                    self.state.view_state.current_line += 1;
                    self.update_content_buffer().await?;
                }
            }
            NavigationCommand::LineUp => {
                if self.state.view_state.current_line > 0 {
                    self.state.view_state.current_line -= 1;
                    self.update_content_buffer().await?;
                }
            }
            NavigationCommand::PageDown => {
                let page_size = self.state.view_state.visible_lines as u64;
                let new_line = (self.state.view_state.current_line + page_size)
                    .min(self.get_max_line());
                self.state.view_state.current_line = new_line;
                self.update_content_buffer().await?;
            }
            NavigationCommand::PageUp => {
                let page_size = self.state.view_state.visible_lines as u64;
                let new_line = self.state.view_state.current_line
                    .saturating_sub(page_size);
                self.state.view_state.current_line = new_line;
                self.update_content_buffer().await?;
            }
            NavigationCommand::GoToBeginning => {
                self.state.view_state.current_line = 0;
                self.update_content_buffer().await?;
            }
            NavigationCommand::GoToEnd => {
                self.state.view_state.current_line = self.get_max_line();
                self.update_content_buffer().await?;
            }
        }
        Ok(())
    }
}
```

**Viewport Management**:
```rust
impl Application {
    async fn update_content_buffer(&mut self) -> Result<()> {
        let start_line = self.state.view_state.current_line;
        let num_lines = self.state.view_state.visible_lines as u64;
        
        let mut buffer = Vec::with_capacity(num_lines as usize);
        for line_num in start_line..(start_line + num_lines) {
            if let Ok(line) = self.file_handler.read_line(line_num).await {
                buffer.push(line);
            } else {
                break; // End of file
            }
        }
        
        self.state.view_state.content_buffer = buffer;
        Ok(())
    }
}
```

**Performance Optimizations**:
- Buffer ahead for smooth scrolling
- Lazy loading for very large files
- Efficient line indexing for random access

**Testing Requirements**:
- Navigation responsiveness (<50ms)
- Boundary condition handling (start/end of file)
- Large file navigation performance
- State consistency validation

---

### **Task 10: Add bidirectional search functionality (/, ?, n, N)**

**Priority**: P0 Essential  
**Scope**: Search UI integration, result navigation, match highlighting  
**Inputs**: Search interface requirements  
**Outputs**: Search integration between UI, search engine, and application core  

**Acceptance Criteria**:
- Forward search (/) and backward search (?) input handling
- Next match (n) and previous match (N) navigation
- Search result highlighting in display
- Search progress indication for large files
- Integration with search engine caching

**Dependencies**: Tasks 6, 9  
**Estimated Effort**: Medium (6-8 hours)  

**Technical Notes**: 
- UI/search engine integration critical for user experience
- Search state management across navigation
- Progressive search for large files

**Implementation Details**:

**Search UI Integration**:
```rust
impl TerminalUI {
    fn handle_search_input(&mut self, forward: bool) -> Result<UICommand> {
        // Enter search mode
        self.enter_search_mode(forward)?;
        
        // Capture search pattern from user input
        let pattern = self.read_search_pattern()?;
        
        if forward {
            Ok(UICommand::Search(SearchCommand::SearchForward(pattern)))
        } else {
            Ok(UICommand::Search(SearchCommand::SearchBackward(pattern)))
        }
    }
    
    fn enter_search_mode(&mut self, forward: bool) -> Result<()> {
        // Show search prompt at bottom of screen
        let prompt = if forward { "/" } else { "?" };
        self.show_search_prompt(prompt)
    }
    
    fn read_search_pattern(&mut self) -> Result<String> {
        let mut pattern = String::new();
        loop {
            let event = crossterm::event::read()?;
            match event {
                Event::Key(KeyEvent { code: KeyCode::Enter, .. }) => break,
                Event::Key(KeyEvent { code: KeyCode::Esc, .. }) => {
                    return Err(RllessError::SearchCancelled);
                }
                Event::Key(KeyEvent { code: KeyCode::Char(c), .. }) => {
                    pattern.push(c);
                    self.update_search_prompt(&pattern)?;
                }
                Event::Key(KeyEvent { code: KeyCode::Backspace, .. }) => {
                    pattern.pop();
                    self.update_search_prompt(&pattern)?;
                }
                _ => {}
            }
        }
        Ok(pattern)
    }
}
```

**Search Command Handling**:
```rust
impl Application {
    async fn handle_search_command(&mut self, search_cmd: SearchCommand) -> Result<()> {
        match search_cmd {
            SearchCommand::SearchForward(pattern) => {
                let current_pos = self.state.view_state.current_line;
                self.perform_search(pattern, current_pos, true).await?;
            }
            SearchCommand::SearchBackward(pattern) => {
                let current_pos = self.state.view_state.current_line;
                self.perform_search(pattern, current_pos, false).await?;
            }
            SearchCommand::NextMatch => {
                self.navigate_to_next_match().await?;
            }
            SearchCommand::PreviousMatch => {
                self.navigate_to_previous_match().await?;
            }
        }
        Ok(())
    }
    
    async fn perform_search(&mut self, pattern: String, from_pos: u64, forward: bool) -> Result<()> {
        // Show search progress for large files
        self.show_search_progress(&pattern)?;
        
        let options = SearchOptions {
            case_sensitive: false, // TODO: Make configurable
            regex_mode: true,
            context_lines: 2,
            whole_word: false,
        };
        
        let matches = self.search_engine.search(&pattern, options).await?;
        
        if matches.is_empty() {
            self.show_search_status("Pattern not found")?;
            return Ok(());
        }
        
        // Update search state
        self.state.search_state.current_pattern = Some(pattern);
        self.state.search_state.matches = matches;
        
        // Navigate to first match
        if forward {
            self.navigate_to_next_match().await?;
        } else {
            self.navigate_to_previous_match().await?;
        }
        
        Ok(())
    }
}
```

**Match Navigation**:
```rust
impl Application {
    async fn navigate_to_next_match(&mut self) -> Result<()> {
        let current_line = self.state.view_state.current_line;
        let matches = &self.state.search_state.matches;
        
        if let Some(next_match) = matches.iter()
            .find(|m| m.line_number > current_line) {
            
            self.state.view_state.current_line = next_match.line_number;
            self.state.view_state.current_match_index = 
                matches.iter().position(|m| m.line_number == next_match.line_number);
            
            self.update_content_buffer().await?;
            self.highlight_current_match()?;
        } else {
            self.show_search_status("Search hit BOTTOM, continuing at TOP")?;
            if let Some(first_match) = matches.first() {
                self.state.view_state.current_line = first_match.line_number;
                self.update_content_buffer().await?;
            }
        }
        
        Ok(())
    }
    
    async fn navigate_to_previous_match(&mut self) -> Result<()> {
        let current_line = self.state.view_state.current_line;
        let matches = &self.state.search_state.matches;
        
        if let Some(prev_match) = matches.iter().rev()
            .find(|m| m.line_number < current_line) {
            
            self.state.view_state.current_line = prev_match.line_number;
            self.state.view_state.current_match_index = 
                matches.iter().position(|m| m.line_number == prev_match.line_number);
            
            self.update_content_buffer().await?;
            self.highlight_current_match()?;
        } else {
            self.show_search_status("Search hit TOP, continuing at BOTTOM")?;
            if let Some(last_match) = matches.last() {
                self.state.view_state.current_line = last_match.line_number;
                self.update_content_buffer().await?;
            }
        }
        
        Ok(())
    }
}
```

**Search Highlighting**:
```rust
impl TerminalUI {
    fn highlight_current_match(&mut self) -> Result<()> {
        // Use ratatui styling to highlight matches
        let matches = &self.current_state.search_matches;
        let current_line = self.current_state.current_line;
        
        // Find matches in current viewport
        let visible_matches: Vec<_> = matches.iter()
            .filter(|m| {
                let line = m.line_number;
                line >= current_line && 
                line < current_line + self.current_state.visible_lines as u64
            })
            .collect();
            
        // Apply highlighting styles
        for match_info in visible_matches {
            self.apply_match_highlight(match_info)?;
        }
        
        Ok(())
    }
}
```

---

### **Task 11: Create CLI interface with command-line argument parsing**

**Priority**: P0 Essential  
**Scope**: Command-line parsing, application initialization  
**Inputs**: CLI specifications from design  
**Outputs**: cli.rs module and updated main.rs  

**Acceptance Criteria**:
- Support for basic usage: `rlless /path/to/file.log`
- Compression file handling: `rlless file.log.gz`
- Initial search: `rlless --search "ERROR" file.log`
- Memory limit override: `rlless --max-memory 200M file.log`
- Proper error messages for invalid arguments
- Help text matching design specifications

**Dependencies**: Task 8  
**Estimated Effort**: Simple (3-4 hours)  

**Technical Notes**: 
- Entry point for user experience; error messages critical
- Validation and user-friendly feedback
- Future extensibility for configuration options

**Implementation Details**:

**CLI Structure**:
```rust
use clap::{Arg, ArgAction, Command};

pub struct CliArgs {
    pub file_path: PathBuf,
    pub initial_search: Option<String>,
    pub max_memory_mb: Option<usize>,
    pub config_path: Option<PathBuf>,
    pub case_sensitive: bool,
    pub follow: bool,
}

pub fn parse_args() -> Result<CliArgs> {
    let matches = Command::new("rlless")
        .version(env!("CARGO_PKG_VERSION"))
        .about("A fast terminal log viewer for large files")
        .long_about("rlless is a high-performance terminal-based log viewer that can handle extremely large files (40GB+) with SIMD-optimized search and memory-efficient streaming.")
        .arg(Arg::new("file")
            .help("Path to the log file to view")
            .required(true)
            .index(1))
        .arg(Arg::new("search")
            .long("search")
            .short('s')
            .help("Initial search pattern")
            .value_name("PATTERN"))
        .arg(Arg::new("max-memory")
            .long("max-memory")
            .help("Maximum memory usage (e.g., 100M, 2G)")
            .value_name("SIZE"))
        .arg(Arg::new("config")
            .long("config")
            .short('c')
            .help("Path to configuration file")
            .value_name("PATH"))
        .arg(Arg::new("case-sensitive")
            .long("case-sensitive")
            .short('C')
            .help("Enable case-sensitive search")
            .action(ArgAction::SetTrue))
        .arg(Arg::new("follow")
            .long("follow")
            .short('f')
            .help("Follow file for new content (tail -f mode)")
            .action(ArgAction::SetTrue))
        .get_matches();

    let file_path = PathBuf::from(matches.get_one::<String>("file").unwrap());
    
    // Validate file exists and is readable
    if !file_path.exists() {
        return Err(RllessError::FileNotFound(file_path));
    }
    
    if !file_path.is_file() {
        return Err(RllessError::NotAFile(file_path));
    }

    let max_memory_mb = if let Some(mem_str) = matches.get_one::<String>("max-memory") {
        Some(parse_memory_size(mem_str)?)
    } else {
        None
    };

    Ok(CliArgs {
        file_path,
        initial_search: matches.get_one::<String>("search").cloned(),
        max_memory_mb,
        config_path: matches.get_one::<String>("config").map(PathBuf::from),
        case_sensitive: matches.get_flag("case-sensitive"),
        follow: matches.get_flag("follow"),
    })
}

fn parse_memory_size(size_str: &str) -> Result<usize> {
    let size_str = size_str.to_uppercase();
    
    if let Some(num_str) = size_str.strip_suffix("GB") {
        Ok(num_str.parse::<usize>()? * 1024)
    } else if let Some(num_str) = size_str.strip_suffix("G") {
        Ok(num_str.parse::<usize>()? * 1024)
    } else if let Some(num_str) = size_str.strip_suffix("MB") {
        Ok(num_str.parse::<usize>()?)
    } else if let Some(num_str) = size_str.strip_suffix("M") {
        Ok(num_str.parse::<usize>()?)
    } else {
        // Assume MB if no suffix
        Ok(size_str.parse::<usize>()?)
    }
}
```

**Main Application Entry**:
```rust
// main.rs
use rlless::{cli, Application, TerminalUI, MmapFileAccessor, RipgrepEngine};

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    env_logger::init();
    
    // Parse command line arguments
    let args = cli::parse_args()?;
    
    // Create application components
    let file_accessor = MmapFileAccessor::new(&args.file_path).await?;
    let search_engine = RipgrepEngine::new(Arc::new(file_accessor))?;
    let ui_manager = TerminalUI::new()?;
    
    // Create and configure application
    let mut app = Application::new(
        Box::new(file_accessor),
        Box::new(search_engine),
        Box::new(ui_manager),
    ).await?;
    
    // Handle initial search if provided
    if let Some(pattern) = args.initial_search {
        app.perform_initial_search(pattern).await?;
    }
    
    // Run main application loop
    app.run().await?;
    
    Ok(())
}
```

**Error Handling and User Feedback**:
```rust
impl From<RllessError> for i32 {
    fn from(error: RllessError) -> i32 {
        match error {
            RllessError::FileNotFound(_) => 1,
            RllessError::PermissionDenied(_) => 2,
            RllessError::InvalidArgument(_) => 3,
            RllessError::SearchError(_) => 4,
            RllessError::UIError(_) => 5,
            _ => 255,
        }
    }
}

pub fn handle_error(error: RllessError) -> ! {
    match error {
        RllessError::FileNotFound(path) => {
            eprintln!("Error: File not found: {}", path.display());
            eprintln!("Make sure the file exists and you have permission to read it.");
        }
        RllessError::NotAFile(path) => {
            eprintln!("Error: Not a regular file: {}", path.display());
            eprintln!("rlless can only view regular files, not directories or special files.");
        }
        RllessError::PermissionDenied(path) => {
            eprintln!("Error: Permission denied: {}", path.display());
            eprintln!("You don't have permission to read this file.");
        }
        _ => {
            eprintln!("Error: {}", error);
        }
    }
    
    std::process::exit(error.into());
}
```

## Phase 3 Integration Testing

### Complete Workflow Tests
1. **Basic File Viewing**: Open file, navigate, quit
2. **Search Workflow**: Open file, search pattern, navigate matches
3. **Large File Handling**: Test with multi-GB files
4. **Compressed Files**: Test with various compression formats
5. **Error Scenarios**: Invalid files, permission errors, corrupted files

### Performance Validation
- **Navigation Response**: <50ms for all navigation commands
- **Search Response**: <500ms for search initiation on 40GB files
- **Memory Usage**: <100MB throughout all operations
- **Startup Time**: <100ms from CLI to interactive state

## Phase 3 Success Criteria

By the end of Phase 3, the following should be complete:
- ✅ Complete less-like navigation working smoothly
- ✅ Bidirectional search with highlighting functional
- ✅ Command-line interface handles all specified arguments
- ✅ Full user workflows work end-to-end
- ✅ MVP is complete and ready for user testing
- ✅ Performance targets met for all operations

## User Acceptance Criteria

The MVP should support these user workflows:

### Basic Viewing Workflow
```bash
$ rlless /var/log/large.log
# User can navigate with j/k, Space, g/G
# User can quit with q
```

### Search Workflow  
```bash
$ rlless /var/log/large.log
# User types "/" to search
# User enters "ERROR" pattern
# User navigates matches with n/N
# User can perform new search with "/" or "?"
```

### Compressed File Workflow
```bash
$ rlless /var/log/archive.log.gz
# File opens transparently
# All navigation and search works normally
```

### CLI Options Workflow
```bash
$ rlless --search "WARN" --max-memory 50M /var/log/app.log
# File opens with initial search results highlighted
# Memory usage respects the 50MB limit
```

## Next Steps

Upon completion of Phase 3, proceed to Phase 4 (Configuration & Quality Assurance) which will implement:
- Configuration system with TOML support
- Comprehensive testing infrastructure  
- Performance benchmarks and validation
- Documentation and examples