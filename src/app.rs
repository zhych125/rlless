//! Application orchestration layer
//!
//! This module provides minimal coordination between file handling, search, and UI components.
//! It avoids duplicating state management that already exists in individual components.

use crate::error::Result;
use crate::file_handler::{FileAccessor, FileAccessorFactory};
use crate::search::{RipgrepEngine, SearchEngine, SearchOptions};
use crate::ui::{UICommand, UIRenderer, ViewState};
use std::borrow::Cow;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

/// Application orchestrator - coordinates components without duplicating their state
pub struct Application {
    file_accessor: Arc<dyn FileAccessor>,
    search_engine: RipgrepEngine,
    ui_renderer: Box<dyn UIRenderer>,
    /// Current search results and navigation state
    search_state: Option<SearchState>,
}

/// Minimal search state for less-like navigation
struct SearchState {
    pattern: String,
    options: SearchOptions,
    last_found_line: Option<u64>,
}

impl Application {
    /// Create application by initializing and wiring components together
    pub async fn new(file_path: &Path, ui_renderer: Box<dyn UIRenderer>) -> Result<Self> {
        let file_accessor = Arc::from(FileAccessorFactory::create(file_path).await?);
        let search_engine = RipgrepEngine::new(Arc::clone(&file_accessor));

        Ok(Self {
            file_accessor,
            search_engine,
            ui_renderer,
            search_state: None,
        })
    }

    /// Run the application - simple event loop that delegates to components
    pub async fn run(&mut self) -> Result<()> {
        // Initialize UI
        self.ui_renderer.initialize()?;

        // Create view state owned by this loop
        let (width, height) = self.ui_renderer.get_terminal_size()?;
        let file_path = self.file_accessor.file_path().to_path_buf();
        let mut view_state = ViewState::new(file_path, width, height);

        // Load initial content
        self.update_view_content(&mut view_state, false).await?;

        // Simple event loop - each iteration is independent
        let mut running = true;
        while running {
            // Get next command
            match self
                .ui_renderer
                .handle_input(Some(Duration::from_millis(50)))
            {
                Ok(Some(command)) => {
                    running = self.execute_command(command, &mut view_state).await?;
                }
                Ok(None) => {
                    // No input - continue
                }
                Err(e) => {
                    eprintln!("Input error: {}", e);
                    break;
                }
            }

            // Render after handling input
            self.ui_renderer.render(&view_state)?;

            // Brief pause
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        self.ui_renderer.cleanup()?;
        Ok(())
    }

    /// Execute a command - returns false if should quit
    async fn execute_command(
        &mut self,
        command: UICommand,
        view_state: &mut ViewState<'_>,
    ) -> Result<bool> {
        use crate::search::SearchOptions;
        use crate::ui::{DisplayCommand, FileCommand, NavigationCommand, SearchCommand};

        match command {
            UICommand::Quit => Ok(false),

            UICommand::Navigation(nav) => {
                // Handle navigation inline to avoid lifetime issues
                match nav {
                    NavigationCommand::LineUp(n) => {
                        let new_line = view_state.cursor_line.saturating_sub(n);
                        view_state.move_cursor_to(new_line);
                    }
                    NavigationCommand::LineDown(n) => {
                        view_state.move_cursor_to(view_state.cursor_line + n);
                    }
                    NavigationCommand::PageUp => {
                        let page_size = view_state.viewport_info.lines_per_page();
                        let new_line = view_state.cursor_line.saturating_sub(page_size);
                        view_state.move_cursor_to(new_line);
                    }
                    NavigationCommand::PageDown => {
                        let page_size = view_state.viewport_info.lines_per_page();
                        view_state.move_cursor_to(view_state.cursor_line + page_size);
                    }
                    NavigationCommand::GoToStart => {
                        view_state.move_cursor_to(0);
                    }
                    NavigationCommand::GoToEnd => {
                        if let Some(total) = self.file_accessor.total_lines() {
                            view_state.move_cursor_to(total.saturating_sub(1));
                        }
                    }
                    NavigationCommand::GoToLine(line) => {
                        view_state.move_cursor_to(line);
                    }
                    _ => {} // Other nav commands
                }
                // Update content once for all navigation commands
                self.update_view_content(view_state, self.search_state.is_some())
                    .await?;
                Ok(true)
            }

            UICommand::Search(search_cmd) => {
                match search_cmd {
                    SearchCommand::SearchPattern(pattern) => {
                        let options = SearchOptions::default();
                        let current_line = view_state.cursor_line;

                        // Search from current position (less-like behavior)
                        match self
                            .search_engine
                            .search_from(&pattern, current_line, &options)
                            .await
                        {
                            Ok(Some(line_number)) => {
                                // Store search state
                                self.search_state = Some(SearchState {
                                    pattern: pattern.clone(),
                                    options,
                                    last_found_line: Some(line_number),
                                });

                                // Jump to match
                                view_state.move_cursor_to(line_number);
                                view_state.status_line.search_info = None; // No status message like less
                            }
                            Ok(None) => {
                                self.search_state = None;
                                view_state.status_line.message =
                                    Some("Pattern not found".to_string());
                                view_state.status_line.search_info = None;
                            }
                            Err(e) => {
                                self.search_state = None;
                                view_state.status_line.message =
                                    Some(format!("Search failed: {}", e));
                                view_state.status_line.search_info = None;
                            }
                        }
                    }
                    SearchCommand::NextMatch => {
                        if let Some(ref mut search) = self.search_state {
                            let start_line = search
                                .last_found_line
                                .map_or(view_state.cursor_line, |line| line + 1);

                            match self
                                .search_engine
                                .search_from(&search.pattern, start_line, &search.options)
                                .await
                            {
                                Ok(Some(line_number)) => {
                                    search.last_found_line = Some(line_number);
                                    view_state.move_cursor_to(line_number);
                                    view_state.status_line.search_info = None; // No status message like less
                                }
                                Ok(None) => {
                                    view_state.status_line.message =
                                        Some("Pattern not found".to_string());
                                }
                                Err(e) => {
                                    view_state.status_line.message =
                                        Some(format!("Search error: {}", e));
                                }
                            }
                        } else {
                            view_state.status_line.message = Some("No active search".to_string());
                        }
                    }
                    SearchCommand::PreviousMatch => {
                        if let Some(ref mut search) = self.search_state {
                            let start_line =
                                search.last_found_line.unwrap_or(view_state.cursor_line);

                            match self
                                .search_engine
                                .search_prev(&search.pattern, start_line, &search.options)
                                .await
                            {
                                Ok(Some(line_number)) => {
                                    search.last_found_line = Some(line_number);
                                    view_state.move_cursor_to(line_number);
                                    view_state.status_line.search_info = None; // No status message like less
                                }
                                Ok(None) => {
                                    view_state.status_line.message =
                                        Some("Pattern not found".to_string());
                                }
                                Err(e) => {
                                    view_state.status_line.message =
                                        Some(format!("Search error: {}", e));
                                }
                            }
                        } else {
                            view_state.status_line.message = Some("No active search".to_string());
                        }
                    }
                    SearchCommand::ClearSearch => {
                        self.search_state = None;
                        view_state.status_line.search_info = None;
                    }
                    _ => {} // Other search commands
                }
                // Update content once for all search commands
                self.update_view_content(view_state, self.search_state.is_some())
                    .await?;
                Ok(true)
            }

            UICommand::Display(display) => {
                match display {
                    DisplayCommand::ToggleLineNumbers => {
                        view_state.display_config.show_line_numbers =
                            !view_state.display_config.show_line_numbers;
                    }
                    DisplayCommand::ToggleWordWrap => {
                        view_state.display_config.wrap_lines =
                            !view_state.display_config.wrap_lines;
                    }
                    _ => {} // Other display commands
                }
                // Update content once for all display commands
                self.update_view_content(view_state, self.search_state.is_some())
                    .await?;
                Ok(true)
            }

            UICommand::File(file_cmd) => {
                if file_cmd == FileCommand::ReloadFile {
                    let file_path = self.file_accessor.file_path();
                    self.file_accessor = Arc::from(FileAccessorFactory::create(file_path).await?);
                    self.search_engine = RipgrepEngine::new(Arc::clone(&self.file_accessor));
                    self.search_state = None; // Clear search state on reload
                    view_state.status_line.search_info = None;
                    view_state.status_line.message = Some("File reloaded".to_string());
                }
                // Update content once for all file commands
                self.update_view_content(view_state, self.search_state.is_some())
                    .await?;
                Ok(true)
            }
        }
    }

    /// Update viewport content with optional search highlights
    async fn update_view_content(
        &mut self,
        view_state: &mut ViewState<'_>,
        with_highlights: bool,
    ) -> Result<()> {
        let page_size = view_state.viewport_info.lines_per_page();
        let start = view_state.viewport_top;

        // Read lines for current viewport - we need to own them to avoid lifetime issues
        let mut lines = Vec::with_capacity(page_size as usize);
        for line_num in start..start + page_size {
            match self.file_accessor.read_line(line_num).await {
                Ok(line) => lines.push(Cow::Owned(line.into_owned())),
                Err(_) => break, // EOF
            }
        }

        // Compute highlights before moving lines, if needed
        let highlights = if with_highlights {
            if let Some(ref search_state) = self.search_state {
                let mut highlights: Vec<(u64, Vec<(usize, usize)>)> = Vec::new();

                // Compute highlights for visible lines on-demand
                for (idx, line_content) in lines.iter().enumerate() {
                    let line_number = start + idx as u64;

                    // Get match ranges for this line
                    if let Ok(match_ranges) = self.search_engine.get_line_matches(
                        &search_state.pattern,
                        line_content,
                        &search_state.options,
                    ) {
                        if !match_ranges.is_empty() {
                            highlights.push((line_number, match_ranges));
                        }
                    }
                }
                Some(highlights)
            } else {
                None
            }
        } else {
            None
        };

        view_state.update_visible_lines(lines);

        // Apply highlights to ViewState
        if let Some(highlights) = highlights {
            view_state.set_search_highlights(highlights);
        } else {
            view_state.clear_search_highlights();
        }

        // Update position info using the proper method to recalculate percentage
        view_state.status_line.update_position(
            view_state.cursor_line,
            self.file_accessor.total_lines(),
            0, // TODO: Calculate actual byte offset if needed
        );

        Ok(())
    }
}
