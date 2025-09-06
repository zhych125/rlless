//! Application orchestration layer
//!
//! This module provides minimal coordination between file handling, search, and UI components.
//! It avoids duplicating state management that already exists in individual components.

use crate::error::Result;
use crate::file_handler::{FileAccessor, FileAccessorFactory};
use crate::search::{RipgrepEngine, SearchEngine, SearchOptions};
use crate::ui::{InputAction, SearchDirection, UIRenderer, ViewState, ViewportInfo};
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
    direction: SearchDirection,
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
                Ok(Some(action)) => {
                    running = self.execute_action(action, &mut view_state).await?;
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

    /// Execute an action - returns false if should quit
    async fn execute_action(
        &mut self,
        action: InputAction,
        view_state: &mut ViewState<'_>,
    ) -> Result<bool> {
        use crate::search::SearchOptions;

        match action {
            InputAction::Quit => Ok(false),

            // Navigation actions - immediate viewport scrolling (less-like behavior)
            InputAction::ScrollUp(n) => {
                view_state.viewport_top = view_state.viewport_top.saturating_sub(n);
                self.update_view_content(view_state, self.search_state.is_some())
                    .await?;
                Ok(true)
            }
            InputAction::ScrollDown(n) => {
                let max_top = self.calculate_max_viewport_top(&view_state.viewport_info);
                view_state.viewport_top = (view_state.viewport_top + n).min(max_top);
                self.update_view_content(view_state, self.search_state.is_some())
                    .await?;
                Ok(true)
            }
            InputAction::PageUp => {
                let page_size = view_state.viewport_info.lines_per_page();
                view_state.viewport_top = view_state.viewport_top.saturating_sub(page_size);
                self.update_view_content(view_state, self.search_state.is_some())
                    .await?;
                Ok(true)
            }
            InputAction::PageDown => {
                let page_size = view_state.viewport_info.lines_per_page();
                let max_top = self.calculate_max_viewport_top(&view_state.viewport_info);
                view_state.viewport_top = (view_state.viewport_top + page_size).min(max_top);
                self.update_view_content(view_state, self.search_state.is_some())
                    .await?;
                Ok(true)
            }
            InputAction::GoToStart => {
                view_state.viewport_top = 0;
                self.update_view_content(view_state, self.search_state.is_some())
                    .await?;
                Ok(true)
            }
            InputAction::GoToEnd => {
                view_state.viewport_top =
                    self.calculate_max_viewport_top(&view_state.viewport_info);
                self.update_view_content(view_state, self.search_state.is_some())
                    .await?;
                Ok(true)
            }

            // Search actions
            InputAction::StartSearch(direction) => {
                // Set search prompt in status line to show user is in search mode
                view_state.status_line.set_search_prompt(direction);
                Ok(true)
            }
            InputAction::UpdateSearchBuffer { direction, buffer } => {
                // Update search prompt with current buffer as user types
                view_state
                    .status_line
                    .update_search_prompt(direction, buffer);
                Ok(true)
            }
            InputAction::CancelSearch => {
                // Clear search prompt and return to normal display
                view_state.status_line.clear_search_prompt();
                view_state.status_line.message = None; // Clear any search messages
                Ok(true)
            }
            InputAction::ExecuteSearch { pattern, direction } => {
                // Less-like search: enable regex mode (default has regex_mode: false)
                let options = SearchOptions {
                    regex_mode: true, // Less treats patterns as basic regex
                    ..Default::default()
                };
                let current_line = view_state.viewport_top;

                // Search from current viewport position (less-like behavior)
                let search_result = match direction {
                    SearchDirection::Forward => {
                        self.search_engine
                            .search_from(&pattern, current_line, &options)
                            .await
                    }
                    SearchDirection::Backward => {
                        self.search_engine
                            .search_prev(&pattern, current_line, &options)
                            .await
                    }
                };

                match search_result {
                    Ok(Some(line_number)) => {
                        // Store search state
                        self.search_state = Some(SearchState {
                            pattern: pattern.clone(),
                            direction,
                            options,
                            last_found_line: Some(line_number),
                        });

                        // Center the match in viewport (less-like behavior)
                        let page_size = view_state.viewport_info.lines_per_page();
                        view_state.viewport_top = line_number.saturating_sub(page_size / 2);

                        // Clear search prompt and messages - search completed successfully
                        view_state.status_line.clear_search_prompt();
                        view_state.status_line.message = None;
                    }
                    Ok(None) => {
                        self.search_state = None;

                        // Clear search prompt and show error message
                        view_state.status_line.clear_search_prompt();
                        view_state.status_line.message = Some("Pattern not found".to_string());
                    }
                    Err(e) => {
                        self.search_state = None;

                        // Clear search prompt and show error message
                        view_state.status_line.clear_search_prompt();
                        view_state.status_line.message = Some(format!("Search failed: {}", e));
                    }
                }
                self.update_view_content(view_state, self.search_state.is_some())
                    .await?;
                Ok(true)
            }
            InputAction::NextMatch => {
                if let Some(ref mut search) = self.search_state {
                    let start_line = search
                        .last_found_line
                        .map_or(view_state.viewport_top, |line| line + 1);

                    let search_result = match search.direction {
                        SearchDirection::Forward => {
                            self.search_engine
                                .search_from(&search.pattern, start_line, &search.options)
                                .await
                        }
                        SearchDirection::Backward => {
                            self.search_engine
                                .search_prev(&search.pattern, start_line, &search.options)
                                .await
                        }
                    };

                    match search_result {
                        Ok(Some(line_number)) => {
                            search.last_found_line = Some(line_number);
                            // Center match in viewport
                            let page_size = view_state.viewport_info.lines_per_page();
                            view_state.viewport_top = line_number.saturating_sub(page_size / 2);
                        }
                        Ok(None) => {
                            view_state.status_line.message = Some("Pattern not found".to_string());
                        }
                        Err(e) => {
                            view_state.status_line.message = Some(format!("Search error: {}", e));
                        }
                    }
                } else {
                    view_state.status_line.message = Some("No active search".to_string());
                }
                self.update_view_content(view_state, self.search_state.is_some())
                    .await?;
                Ok(true)
            }
            InputAction::PreviousMatch => {
                if let Some(ref mut search) = self.search_state {
                    let start_line = search.last_found_line.unwrap_or(view_state.viewport_top);

                    // Reverse the search direction for "previous"
                    let search_result = match search.direction {
                        SearchDirection::Forward => {
                            self.search_engine
                                .search_prev(&search.pattern, start_line, &search.options)
                                .await
                        }
                        SearchDirection::Backward => {
                            self.search_engine
                                .search_from(&search.pattern, start_line, &search.options)
                                .await
                        }
                    };

                    match search_result {
                        Ok(Some(line_number)) => {
                            search.last_found_line = Some(line_number);
                            // Center match in viewport
                            let page_size = view_state.viewport_info.lines_per_page();
                            view_state.viewport_top = line_number.saturating_sub(page_size / 2);
                        }
                        Ok(None) => {
                            view_state.status_line.message = Some("Pattern not found".to_string());
                        }
                        Err(e) => {
                            view_state.status_line.message = Some(format!("Search error: {}", e));
                        }
                    }
                } else {
                    view_state.status_line.message = Some("No active search".to_string());
                }
                self.update_view_content(view_state, self.search_state.is_some())
                    .await?;
                Ok(true)
            }

            // Other actions - simplified for essential functionality only
            InputAction::NoAction => Ok(true),
            InputAction::InvalidInput => {
                // Just ignore invalid input - no error message needed
                Ok(true)
            }
        }
    }

    /// Calculate the maximum viewport top position
    fn calculate_max_viewport_top(&self, viewport_info: &ViewportInfo) -> u64 {
        if let Some(total_lines) = self.file_accessor.total_lines() {
            let page_size = viewport_info.lines_per_page();
            total_lines.saturating_sub(page_size)
        } else {
            // For large files without known total, allow scrolling to very large position
            // The file accessor will handle EOF gracefully
            u64::MAX - viewport_info.lines_per_page()
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

        // Update position info - use viewport_top instead of cursor_line (we removed cursor)
        view_state
            .status_line
            .update_position(view_state.viewport_top, self.file_accessor.total_lines());

        Ok(())
    }
}
