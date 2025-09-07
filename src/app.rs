//! Application orchestration layer
//!
//! This module provides minimal coordination between file handling, search, and UI components.
//! It avoids duplicating state management that already exists in individual components.

use crate::error::Result;
use crate::file_handler::{FileAccessor, FileAccessorFactory};
use crate::search::{RipgrepEngine, SearchEngine, SearchOptions};
use crate::ui::{InputAction, SearchDirection, UIRenderer, ViewState};
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
}

impl Application {
    /// Create application by initializing and wiring components together
    pub async fn new(file_path: &Path, ui_renderer: Box<dyn UIRenderer>) -> Result<Self> {
        let file_accessor: Arc<dyn FileAccessor> =
            Arc::new(FileAccessorFactory::create(file_path).await?);
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
            // Handle input in separate scope to avoid borrowing conflicts
            let action = {
                match self
                    .ui_renderer
                    .handle_input(Some(Duration::from_millis(50)))
                {
                    Ok(action_opt) => action_opt,
                    Err(e) => {
                        eprintln!("Input error: {}", e);
                        break;
                    }
                }
            };

            // Execute action if we have one
            if let Some(action) = action {
                running = self.execute_action(action, &mut view_state).await?;
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
        view_state: &mut ViewState,
    ) -> Result<bool> {
        use crate::search::SearchOptions;

        match action {
            InputAction::Quit => Ok(false),

            // Navigation actions - immediate viewport scrolling (less-like behavior)
            InputAction::ScrollUp(n) => {
                let new_byte = self
                    .file_accessor
                    .prev_page_start(view_state.viewport_top_byte, n as usize)
                    .await?;
                view_state.at_eof = false; // Clear EOF flag (moving backward)
                view_state.navigate_to_byte(new_byte);
                self.update_view_content(view_state, self.search_state.is_some())
                    .await?;
                Ok(true)
            }
            InputAction::ScrollDown(n) => {
                // User is trying to scroll - check if last line of file is already visible
                let file_size = self.file_accessor.file_size();

                if !view_state.visible_lines.is_empty() {
                    // Calculate byte position after our current viewport (end of what we can see)
                    let end_of_viewport = self
                        .file_accessor
                        .next_page_start(
                            view_state.viewport_top_byte,
                            view_state.visible_lines.len(),
                        )
                        .await?;

                    // If current viewport already shows the end of file, show EOD and stop scrolling
                    if end_of_viewport >= file_size {
                        view_state.at_eof = true;
                        return Ok(true);
                    }
                }

                // Normal scroll - try to advance
                let new_byte = self
                    .file_accessor
                    .next_page_start(view_state.viewport_top_byte, n as usize)
                    .await?;

                // Move viewport and clear EOF flag for normal scrolling
                view_state.at_eof = false;
                view_state.navigate_to_byte(new_byte);

                self.update_view_content(view_state, self.search_state.is_some())
                    .await?;
                Ok(true)
            }
            InputAction::PageUp => {
                let page_size = view_state.lines_per_page() as usize;
                let new_byte = self
                    .file_accessor
                    .prev_page_start(view_state.viewport_top_byte, page_size)
                    .await?;
                view_state.at_eof = false; // Clear EOF flag (moving backward)
                view_state.navigate_to_byte(new_byte);
                self.update_view_content(view_state, self.search_state.is_some())
                    .await?;
                Ok(true)
            }
            InputAction::PageDown => {
                let page_size = view_state.lines_per_page() as usize;
                let new_byte = self
                    .file_accessor
                    .next_page_start(view_state.viewport_top_byte, page_size)
                    .await?;

                // Check if we hit EOF (next_page_start returns file_size when can't advance)
                let file_size = self.file_accessor.file_size();
                if new_byte == file_size {
                    view_state.at_eof = true;
                } else {
                    view_state.at_eof = false;
                    view_state.navigate_to_byte(new_byte);
                }

                self.update_view_content(view_state, self.search_state.is_some())
                    .await?;
                Ok(true)
            }
            InputAction::GoToStart => {
                view_state.at_eof = false; // Clear EOF flag
                view_state.navigate_to_byte(0);
                self.update_view_content(view_state, self.search_state.is_some())
                    .await?;
                Ok(true)
            }
            InputAction::GoToEnd => {
                let page_size = view_state.lines_per_page() as usize;
                let end_byte = self.file_accessor.last_page_start(page_size).await?;
                view_state.at_eof = false; // Clear EOF flag (GoToEnd navigates to a valid position)
                view_state.navigate_to_byte(end_byte);
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
                let current_byte = view_state.viewport_top_byte;

                // Search from current viewport position (less-like behavior)
                let search_result = match direction {
                    SearchDirection::Forward => {
                        self.search_engine
                            .search_from(&pattern, current_byte, &options)
                            .await
                    }
                    SearchDirection::Backward => {
                        self.search_engine
                            .search_prev(&pattern, current_byte, &options)
                            .await
                    }
                };

                match search_result {
                    Ok(Some(match_byte)) => {
                        // Store search state
                        self.search_state = Some(SearchState {
                            pattern: pattern.clone(),
                            direction,
                            options,
                        });

                        // Put the match at the top of viewport (less-like behavior)
                        view_state.navigate_to_byte(match_byte);

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
                if let Some(ref search) = self.search_state {
                    // NextMatch continues in the same direction as original search
                    let next_start_byte = match search.direction {
                        SearchDirection::Forward => {
                            // For forward search, start from next line after current viewport
                            self.find_next_line_start(view_state.viewport_top_byte)
                                .await?
                        }
                        SearchDirection::Backward => {
                            // For backward search, start from previous line before current viewport
                            self.find_prev_line_start(view_state.viewport_top_byte)
                                .await?
                        }
                    };

                    let search_result = match search.direction {
                        SearchDirection::Forward => {
                            self.search_engine
                                .search_from(&search.pattern, next_start_byte, &search.options)
                                .await
                        }
                        SearchDirection::Backward => {
                            self.search_engine
                                .search_prev(&search.pattern, next_start_byte, &search.options)
                                .await
                        }
                    };

                    match search_result {
                        Ok(Some(match_byte)) => {
                            // Put the match at the top of viewport
                            view_state.at_eof = false; // Clear EOF flag when search succeeds
                            view_state.navigate_to_byte(match_byte);
                        }
                        Ok(None) => {
                            view_state
                                .status_line
                                .set_message("Pattern not found".to_string());
                        }
                        Err(e) => {
                            view_state
                                .status_line
                                .set_message(format!("Search error: {}", e));
                        }
                    }
                } else {
                    view_state
                        .status_line
                        .set_message("No active search".to_string());
                }
                self.update_view_content(view_state, self.search_state.is_some())
                    .await?;
                Ok(true)
            }
            InputAction::PreviousMatch => {
                if let Some(ref search) = self.search_state {
                    // PreviousMatch goes in opposite direction of original search
                    let prev_start_byte = match search.direction {
                        SearchDirection::Forward => {
                            // For forward search, previous means go backward from current viewport
                            self.find_prev_line_start(view_state.viewport_top_byte)
                                .await?
                        }
                        SearchDirection::Backward => {
                            // For backward search, previous means go forward from current viewport
                            self.find_next_line_start(view_state.viewport_top_byte)
                                .await?
                        }
                    };

                    let search_result = match search.direction {
                        SearchDirection::Forward => {
                            // For forward search, previous means search backward
                            self.search_engine
                                .search_prev(&search.pattern, prev_start_byte, &search.options)
                                .await
                        }
                        SearchDirection::Backward => {
                            // For backward search, previous means search forward
                            self.search_engine
                                .search_from(&search.pattern, prev_start_byte, &search.options)
                                .await
                        }
                    };

                    match search_result {
                        Ok(Some(match_byte)) => {
                            // Put the match at the top of viewport
                            view_state.at_eof = false; // Clear EOF flag when search succeeds
                            view_state.navigate_to_byte(match_byte);
                        }
                        Ok(None) => {
                            view_state
                                .status_line
                                .set_message("Pattern not found".to_string());
                        }
                        Err(e) => {
                            view_state
                                .status_line
                                .set_message(format!("Search error: {}", e));
                        }
                    }
                } else {
                    view_state
                        .status_line
                        .set_message("No active search".to_string());
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

    /// Find the byte position of the start of the next line after given byte position
    async fn find_next_line_start(&self, current_byte: u64) -> Result<u64> {
        // Use FileAccessor's next_page_start with 1 line to find next line
        let new_byte = self.file_accessor.next_page_start(current_byte, 1).await?;

        // If at EOF (returned file_size), return current position to indicate no advancement
        if new_byte == self.file_accessor.file_size() {
            Ok(current_byte)
        } else {
            Ok(new_byte)
        }
    }

    /// Find the byte position of the start of the previous line before given byte position
    async fn find_prev_line_start(&self, current_byte: u64) -> Result<u64> {
        if current_byte == 0 {
            return Ok(0); // Beginning of file
        }

        // Use FileAccessor's prev_page_start with 1 line to find previous line
        self.file_accessor.prev_page_start(current_byte, 1).await
    }

    /// Update viewport content with optional search highlights
    async fn update_view_content(
        &self,
        view_state: &mut ViewState,
        with_highlights: bool,
    ) -> Result<()> {
        let page_size = view_state.lines_per_page() as usize;

        // ✅ Use FileAccessor byte-based method
        let lines = self
            .file_accessor
            .read_from_byte(view_state.viewport_top_byte, page_size)
            .await?;

        // Compute viewport-relative highlights
        let highlights = if with_highlights {
            self.compute_viewport_highlights(&lines).await?
        } else {
            vec![Vec::new(); lines.len()]
        };

        // Update view state with both lines and highlights
        view_state.update_viewport_content(lines, highlights);

        // Update file size for position calculation
        view_state.file_size = Some(self.file_accessor.file_size());

        Ok(())
    }

    /// Compute search highlights for viewport-relative line indices
    async fn compute_viewport_highlights(
        &self,
        lines: &[String],
    ) -> Result<Vec<Vec<(usize, usize)>>> {
        let mut highlights = vec![Vec::new(); lines.len()];

        if let Some(ref search_state) = self.search_state {
            for (viewport_line_idx, line_content) in lines.iter().enumerate() {
                let match_ranges = self.search_engine.get_line_matches(
                    &search_state.pattern,
                    line_content,
                    &search_state.options,
                )?;

                highlights[viewport_line_idx] = match_ranges;
            }
        }

        Ok(highlights)
    }
}
