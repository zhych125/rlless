//! UI state management structures
//!
//! This module contains viewport state for rendering. Search operations
//! are handled by SearchEngine, not ViewState.

use crate::ui::SearchDirection;
use std::path::{Path, PathBuf};

/// Viewport state for rendering - focused only on what's currently visible
#[derive(Debug)]
pub struct ViewState {
    /// Byte position of the first line currently in viewport (absolute file position)
    pub viewport_top_byte: u64,

    /// Lines currently visible in the viewport
    /// String data from FileAccessor
    pub visible_lines: Vec<String>,

    /// Status line content
    pub status_line: StatusLine,

    /// File path for display
    pub file_path: PathBuf,

    /// File size in bytes (for position calculation)
    /// None if file size is not yet known
    pub file_size: Option<u64>,

    /// Viewport dimensions
    pub viewport_width: u16,
    pub viewport_height: u16,

    /// Search highlights by viewport-relative line number (Vec index = viewport line)
    /// Empty Vec at index means no highlights for that line
    pub search_highlights: Vec<Vec<(usize, usize)>>,

    /// Track if user has hit EOF during navigation (for EOD status display)
    pub at_eof: bool,
}

impl ViewState {
    /// Create a new viewport state
    pub fn new(file_path: impl AsRef<Path>, viewport_width: u16, viewport_height: u16) -> Self {
        Self {
            viewport_top_byte: 0, // Start at beginning of file
            visible_lines: Vec::new(),
            status_line: StatusLine::new(),
            file_path: file_path.as_ref().to_path_buf(),
            file_size: None, // Will be set when content is loaded
            viewport_width,
            viewport_height,
            search_highlights: Vec::new(),
            at_eof: false, // Start not at EOF
        }
    }

    /// Get the filename for display
    pub fn filename(&self) -> String {
        self.file_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("<unnamed>")
            .to_string()
    }

    /// Get lines per page (viewport height minus status line)
    pub fn lines_per_page(&self) -> u16 {
        self.viewport_height.saturating_sub(1)
    }

    /// Get the number of lines currently in the viewport
    pub fn viewport_line_count(&self) -> usize {
        self.visible_lines.len()
    }

    /// Navigate to a specific byte position in the file
    pub fn navigate_to_byte(&mut self, byte_position: u64) {
        self.viewport_top_byte = byte_position;
    }

    /// Update viewport with content and highlights in one operation
    pub fn update_viewport_content(
        &mut self,
        lines: Vec<String>,
        highlights: Vec<Vec<(usize, usize)>>,
    ) {
        self.visible_lines = lines;
        self.search_highlights = highlights;
    }

    /// Update terminal dimensions and mark that content needs to be recalculated
    /// Returns true if dimensions actually changed
    pub fn update_terminal_size(&mut self, width: u16, height: u16) -> bool {
        let changed = self.viewport_width != width || self.viewport_height != height;

        if changed {
            self.viewport_width = width;
            self.viewport_height = height;
            // Clear visible content - it will need to be recalculated with new dimensions
            self.visible_lines.clear();
            self.search_highlights.clear();
            // Reset EOF state since viewport size changed
            self.at_eof = false;
        }

        changed
    }

    /// Format the complete status line for this view state
    pub fn format_status_line(&self) -> String {
        self.status_line.format_status_line(
            &self.filename(),
            self.viewport_top_byte,
            self.file_size.unwrap_or(0),
            self.at_eof,
        )
    }
}

/// Status line information
#[derive(Debug, Clone, Default)]
pub struct StatusLine {
    pub message: Option<String>,
    pub search_prompt: Option<(SearchDirection, String)>,
}

impl StatusLine {
    /// Create a new status line
    pub fn new() -> Self {
        Self::default()
    }

    /// Set a temporary message
    pub fn set_message(&mut self, message: String) {
        self.message = Some(message);
    }

    /// Clear any temporary message
    pub fn clear_message(&mut self) {
        self.message = None;
    }

    /// Set search prompt for input mode
    pub fn set_search_prompt(&mut self, direction: SearchDirection) {
        self.search_prompt = Some((direction, String::new()));
    }

    /// Update search prompt with current buffer
    pub fn update_search_prompt(&mut self, direction: SearchDirection, buffer: String) {
        self.search_prompt = Some((direction, buffer));
    }

    /// Clear search prompt and return to normal mode
    pub fn clear_search_prompt(&mut self) {
        self.search_prompt = None;
    }

    /// Format the status line for display (with position calculated on-the-fly)
    pub fn format_status_line(
        &self,
        filename: &str,
        current_byte: u64,
        total_bytes: u64,
        at_eof: bool,
    ) -> String {
        if let Some((direction, buffer)) = &self.search_prompt {
            // Show search prompt: "/search_term"
            format!("{}{}", direction.to_char(), buffer)
        } else {
            // Calculate position on-the-fly
            let position = if total_bytes == 0 {
                "Empty".to_string()
            } else if at_eof {
                "EOD".to_string() // End of Data - user hit EOF during navigation
            } else if current_byte >= total_bytes {
                "END".to_string() // At end of file (for other cases)
            } else {
                let percentage = (current_byte as f32 / total_bytes as f32) * 100.0;
                format!("{:.0}%", percentage)
            };

            // Format status line
            if let Some(ref message) = self.message {
                format!("{} | {} | {}", filename, position, message)
            } else {
                format!("{} | {}", filename, position)
            }
        }
    }
}

/// Current display mode
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DisplayMode {
    Normal,
    Search(SearchDirection),
    Command,
    Help,
}

impl DisplayMode {
    /// Get the mode indicator string for display
    pub fn indicator(&self) -> &'static str {
        match self {
            DisplayMode::Normal => "",
            DisplayMode::Search(SearchDirection::Forward) => "/",
            DisplayMode::Search(SearchDirection::Backward) => "?",
            DisplayMode::Command => ":",
            DisplayMode::Help => "HELP",
        }
    }

    /// Check if this mode accepts text input
    pub fn accepts_input(&self) -> bool {
        matches!(self, DisplayMode::Search(_) | DisplayMode::Command)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_view_state_creation() {
        let path = PathBuf::from("/test/file.log");
        let state = ViewState::new(path.clone(), 80, 24);

        assert_eq!(state.viewport_top_byte, 0);
        assert_eq!(state.visible_lines.len(), 0);
        assert_eq!(state.file_path, path);
        assert_eq!(state.viewport_width, 80);
        assert_eq!(state.viewport_height, 24);
        assert!(state.file_size.is_none());
    }

    #[test]
    fn test_viewport_navigation() {
        let path = PathBuf::from("/test/file.log");
        let mut state = ViewState::new(path, 80, 24);

        // Test byte-based navigation
        state.navigate_to_byte(1000);
        assert_eq!(state.viewport_top_byte, 1000);

        // Test navigate to different position
        state.navigate_to_byte(2048);
        assert_eq!(state.viewport_top_byte, 2048);
    }

    #[test]
    fn test_display_mode() {
        assert_eq!(DisplayMode::Normal.indicator(), "");
        assert_eq!(
            DisplayMode::Search(SearchDirection::Forward).indicator(),
            "/"
        );
        assert_eq!(
            DisplayMode::Search(SearchDirection::Backward).indicator(),
            "?"
        );
        assert_eq!(DisplayMode::Command.indicator(), ":");
        assert_eq!(DisplayMode::Help.indicator(), "HELP");

        assert!(DisplayMode::Search(SearchDirection::Forward).accepts_input());
        assert!(DisplayMode::Command.accepts_input());
        assert!(!DisplayMode::Normal.accepts_input());
    }

    #[test]
    fn test_status_line_format() {
        let mut status = StatusLine::new();

        // Test normal status line with position
        let formatted = status.format_status_line("test.log", 512, 1024, false);
        assert_eq!(formatted, "test.log | 50%");

        // Test with message
        status.set_message("Pattern not found".to_string());
        let formatted = status.format_status_line("test.log", 512, 1024, false);
        assert_eq!(formatted, "test.log | 50% | Pattern not found");

        // Test empty file
        let formatted = status.format_status_line("empty.log", 0, 0, false);
        assert_eq!(formatted, "empty.log | Empty | Pattern not found");

        // Test at end
        status.clear_message();
        let formatted = status.format_status_line("test.log", 1024, 1024, false);
        assert_eq!(formatted, "test.log | END");

        // Test search prompt
        status.set_search_prompt(SearchDirection::Forward);
        let formatted = status.format_status_line("test.log", 512, 1024, false);
        assert_eq!(formatted, "/");

        status.update_search_prompt(SearchDirection::Forward, "search term".to_string());
        let formatted = status.format_status_line("test.log", 512, 1024, false);
        assert_eq!(formatted, "/search term");

        // Test EOD (End of Data) display when at_eof is true
        status.clear_search_prompt();
        let formatted = status.format_status_line("test.log", 512, 1024, true);
        assert_eq!(formatted, "test.log | EOD");
    }

    #[test]
    fn test_terminal_resize() {
        let path = PathBuf::from("/test/file.log");
        let mut state = ViewState::new(path, 80, 24);

        // Add some mock visible content
        state.visible_lines = vec!["line1".to_string(), "line2".to_string()];
        state.search_highlights = vec![vec![(0, 4)], vec![]]; // highlight "line" in first line

        // Test resize to same dimensions - should return false
        assert!(!state.update_terminal_size(80, 24));
        assert_eq!(state.viewport_width, 80);
        assert_eq!(state.viewport_height, 24);
        // Content should remain unchanged
        assert_eq!(state.visible_lines.len(), 2);
        assert_eq!(state.search_highlights.len(), 2);

        // Test resize to different dimensions - should return true
        assert!(state.update_terminal_size(120, 30));
        assert_eq!(state.viewport_width, 120);
        assert_eq!(state.viewport_height, 30);
        // Content should be cleared for recalculation
        assert_eq!(state.visible_lines.len(), 0);
        assert_eq!(state.search_highlights.len(), 0);
        assert!(!state.at_eof); // EOF state should be reset

        // Test width-only change
        state.visible_lines = vec!["test".to_string()];
        assert!(state.update_terminal_size(100, 30));
        assert_eq!(state.visible_lines.len(), 0);

        // Test height-only change
        state.visible_lines = vec!["test".to_string()];
        assert!(state.update_terminal_size(100, 25));
        assert_eq!(state.visible_lines.len(), 0);
    }
}
