//! UI state management structures
//!
//! This module contains viewport state for rendering. Search operations
//! are handled by SearchEngine, not ViewState.

use crate::ui::{ColorTheme, SearchDirection};
use std::borrow::Cow;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Viewport state for rendering - focused only on what's currently visible
#[derive(Debug)]
pub struct ViewState<'a> {
    /// Current top line of the viewport
    pub viewport_top: u64,

    /// Current cursor line (may be different from viewport_top)
    pub cursor_line: u64,

    /// Lines currently visible in the viewport
    /// Borrowed from FileAccessor for zero-copy efficiency
    pub visible_lines: Vec<Cow<'a, str>>,

    /// Status line content
    pub status_line: StatusLine,

    /// File metadata for display
    pub file_info: FileDisplayInfo,

    /// Display configuration
    pub display_config: DisplayConfig,

    /// Viewport dimensions
    pub viewport_info: ViewportInfo,

    /// Search highlights mapped by absolute line number to match ranges
    /// Uses absolute line numbers (not viewport-relative positions)
    pub search_highlights: std::collections::HashMap<u64, Vec<(usize, usize)>>,
}

impl<'a> ViewState<'a> {
    /// Create a new viewport state
    pub fn new(file_path: impl AsRef<Path>, viewport_width: u16, viewport_height: u16) -> Self {
        Self {
            viewport_top: 0,
            cursor_line: 0,
            visible_lines: Vec::new(),
            status_line: StatusLine::default(),
            file_info: FileDisplayInfo::new(file_path),
            display_config: DisplayConfig::default(),
            viewport_info: ViewportInfo::new(viewport_width, viewport_height),
            search_highlights: HashMap::new(),
        }
    }

    /// Get the number of lines currently in the viewport
    pub fn viewport_line_count(&self) -> usize {
        self.visible_lines.len()
    }

    /// Update viewport content with new lines
    pub fn update_visible_lines(&mut self, lines: Vec<Cow<'a, str>>) {
        self.visible_lines = lines;
    }

    /// Move cursor to a specific line and update viewport if needed
    pub fn move_cursor_to(&mut self, line_number: u64) {
        self.cursor_line = line_number;

        // Ensure cursor is within viewport
        let lines_per_page = self.viewport_info.lines_per_page();
        if line_number < self.viewport_top {
            self.viewport_top = line_number;
        } else if line_number >= self.viewport_top + lines_per_page {
            self.viewport_top = line_number.saturating_sub(lines_per_page - 1);
        }
    }

    /// Scroll viewport by the given number of lines
    pub fn scroll_by(&mut self, lines: i64) {
        if lines > 0 {
            self.viewport_top += lines as u64;
        } else {
            self.viewport_top = self.viewport_top.saturating_sub((-lines) as u64);
        }
    }

    /// Set search highlights from a list of (line_number, match_ranges) pairs
    pub fn set_search_highlights(&mut self, highlights: Vec<(u64, Vec<(usize, usize)>)>) {
        self.search_highlights.clear();
        for (line_num, ranges) in highlights {
            self.search_highlights.insert(line_num, ranges);
        }
    }

    /// Clear all search highlights
    pub fn clear_search_highlights(&mut self) {
        self.search_highlights.clear();
    }
}

/// Helper functions for working with line content and matches
pub struct LineDisplayUtils;

impl LineDisplayUtils {
    /// Get the display content length (accounting for tab expansion)
    pub fn display_length(content: &str, tab_width: u8) -> usize {
        let mut length = 0;
        for ch in content.chars() {
            if ch == '\t' {
                length += tab_width as usize - (length % tab_width as usize);
            } else {
                length += 1;
            }
        }
        length
    }

    /// Check if two match ranges overlap
    pub fn ranges_overlap(range1: (usize, usize), range2: (usize, usize)) -> bool {
        range1.0 < range2.1 && range2.0 < range1.1
    }

    /// Get the length of a match range
    pub fn range_length(range: (usize, usize)) -> usize {
        range.1.saturating_sub(range.0)
    }
}

/// Highlight styles for different types of matches
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HighlightStyle {
    SearchMatch,  // Current search matches
    CurrentMatch, // Currently focused match
    Selection,    // Selected text
    Error,        // Error highlighting
    Warning,      // Warning highlighting
    Info,         // Info highlighting
}

/// Status line information
#[derive(Debug, Clone)]
pub struct StatusLine {
    pub mode: DisplayMode,
    pub message: Option<String>,
    pub position: PositionInfo,
    pub search_prompt: Option<(SearchDirection, String)>,
}

impl Default for StatusLine {
    fn default() -> Self {
        Self {
            mode: DisplayMode::Normal,
            message: None,
            position: PositionInfo::default(),
            search_prompt: None,
        }
    }
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

    /// Update position information
    pub fn update_position(&mut self, current_line: u64, total_lines: Option<u64>) {
        self.position = PositionInfo {
            current_line,
            total_lines,
            percentage: match total_lines {
                Some(total) if total > 0 => (current_line + 1) as f32 / total as f32,
                _ => 0.0,
            },
        };
    }

    /// Format the status line content (position and message only)
    pub fn format_status_content(&self) -> String {
        if let Some(ref message) = self.message {
            // Show message (like "Pattern not found") when available
            format!("{} | {}", self.position.format_position(), message)
        } else {
            // Just position info
            self.position.format_position()
        }
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

    /// Format the complete status line for display
    pub fn format_status_line(&self, filename: &str) -> String {
        if let Some((direction, buffer)) = &self.search_prompt {
            // Show search prompt: "/search_term"
            format!("{}{}", direction.to_char(), buffer)
        } else {
            // Normal status line: "filename | position | message"
            format!("{} | {}", filename, self.format_status_content())
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

/// Position information for status line
#[derive(Debug, Clone)]
pub struct PositionInfo {
    pub current_line: u64,
    pub total_lines: Option<u64>,
    pub percentage: f32,
}

impl Default for PositionInfo {
    fn default() -> Self {
        Self {
            current_line: 0,
            total_lines: None,
            percentage: 0.0,
        }
    }
}

impl PositionInfo {
    /// Format position as a display string
    pub fn format_position(&self) -> String {
        match self.total_lines {
            Some(0) => "Empty".to_string(),
            Some(total) => {
                if self.current_line >= total {
                    "END".to_string()
                } else {
                    format!(
                        "Line {}/{} ({:.0}%)",
                        self.current_line + 1, // Display as 1-based
                        total,
                        self.percentage * 100.0
                    )
                }
            }
            None => {
                // Total lines not known yet (large file with lazy indexing)
                format!("Line {} (?)", self.current_line + 1)
            }
        }
    }
}

/// File information for display
#[derive(Debug, Clone)]
pub struct FileDisplayInfo {
    pub path: PathBuf,
    pub size: u64,
    pub line_count: u64,
    pub modified: Option<SystemTime>,
    pub encoding: String,
}

impl FileDisplayInfo {
    /// Create new file display info
    pub fn new(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
            size: 0,
            line_count: 0,
            modified: None,
            encoding: "UTF-8".to_string(),
        }
    }

    /// Get the filename for display
    pub fn filename(&self) -> String {
        self.path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("<unnamed>")
            .to_string()
    }

    /// Format file size for display
    pub fn format_size(&self) -> String {
        const UNITS: &[(&str, u64)] = &[
            ("GB", 1024 * 1024 * 1024),
            ("MB", 1024 * 1024),
            ("KB", 1024),
            ("B", 1),
        ];

        for &(unit, size) in UNITS {
            if self.size >= size {
                return format!("{:.1} {}", self.size as f64 / size as f64, unit);
            }
        }

        "0 B".to_string()
    }
}

/// Display configuration
#[derive(Debug, Clone)]
pub struct DisplayConfig {
    pub show_line_numbers: bool,
    pub wrap_lines: bool,
    pub tab_width: u8,
    pub highlight_search: bool,
    pub theme: ColorTheme,
}

impl Default for DisplayConfig {
    fn default() -> Self {
        Self {
            show_line_numbers: false,
            wrap_lines: false,
            tab_width: 8,
            highlight_search: true,
            theme: ColorTheme::default(),
        }
    }
}

impl DisplayConfig {
    /// Toggle line number display
    pub fn toggle_line_numbers(&mut self) {
        self.show_line_numbers = !self.show_line_numbers;
    }

    /// Toggle line wrapping
    pub fn toggle_word_wrap(&mut self) {
        self.wrap_lines = !self.wrap_lines;
    }

    /// Set tab width (clamped to reasonable values)
    pub fn set_tab_width(&mut self, width: u8) {
        self.tab_width = width.clamp(1, 16);
    }

    /// Toggle search highlighting
    pub fn toggle_search_highlight(&mut self) {
        self.highlight_search = !self.highlight_search;
    }
}

/// Viewport information
#[derive(Debug, Clone, Copy)]
pub struct ViewportInfo {
    pub width: u16,
    pub height: u16,
    pub content_height: u16, // Height minus status line
}

impl ViewportInfo {
    /// Create new viewport info
    pub fn new(width: u16, height: u16) -> Self {
        Self {
            width,
            height,
            content_height: height.saturating_sub(2), // Reserve for status line
        }
    }

    /// Get the number of lines that can be displayed
    pub fn lines_per_page(&self) -> u64 {
        self.content_height as u64
    }

    /// Get half page size for half-page navigation
    pub fn half_page_size(&self) -> u64 {
        (self.content_height / 2).max(1) as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_view_state_creation() {
        let path = PathBuf::from("/test/file.log");
        let state = ViewState::new(path.clone(), 80, 24);

        assert_eq!(state.viewport_top, 0);
        assert_eq!(state.cursor_line, 0);
        assert_eq!(state.visible_lines.len(), 0);
        assert_eq!(state.file_info.path, path);
        assert_eq!(state.viewport_info.width, 80);
        assert_eq!(state.viewport_info.height, 24);
    }

    #[test]
    fn test_viewport_navigation() {
        let path = PathBuf::from("/test/file.log");
        let mut state = ViewState::new(path, 80, 24);

        // Test cursor movement
        state.move_cursor_to(10);
        assert_eq!(state.cursor_line, 10);
        assert_eq!(state.viewport_top, 0); // Should still be visible

        // Test cursor movement beyond viewport
        state.move_cursor_to(30);
        assert_eq!(state.cursor_line, 30);
        assert!(state.viewport_top > 0); // Viewport should have scrolled

        // Test scrolling
        let original_top = state.viewport_top;
        state.scroll_by(5);
        assert_eq!(state.viewport_top, original_top + 5);

        state.scroll_by(-3);
        assert_eq!(state.viewport_top, original_top + 2);
    }

    #[test]
    fn test_line_display_utils() {
        // Test tab expansion: "a\tb\t\tc" with tab_width=8
        // a(1) + tab_to_8(7) + b(1) + tab_to_16(7) + tab_to_24(8) + c(1) = 25
        assert_eq!(LineDisplayUtils::display_length("a\tb\t\tc", 8), 25);

        // Test with tab_width=4: "a\tb\t\tc"
        // a(1) + tab_to_4(3) + b(1) + tab_to_8(3) + tab_to_12(4) + c(1) = 13
        assert_eq!(LineDisplayUtils::display_length("a\tb\t\tc", 4), 13);

        // Test range operations
        assert!(LineDisplayUtils::ranges_overlap((0, 5), (3, 8)));
        assert!(!LineDisplayUtils::ranges_overlap((0, 5), (10, 15)));
        assert_eq!(LineDisplayUtils::range_length((5, 10)), 5);
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
    fn test_position_info() {
        let pos = PositionInfo {
            current_line: 49,
            total_lines: Some(100),
            percentage: 0.5,
        };

        assert_eq!(pos.format_position(), "Line 50/100 (50%)");

        let empty_pos = PositionInfo {
            current_line: 0,
            total_lines: Some(0),
            percentage: 0.0,
        };
        assert_eq!(empty_pos.format_position(), "Empty");

        let unknown_total_pos = PositionInfo {
            current_line: 49,
            total_lines: None,
            percentage: 0.0,
        };
        assert_eq!(unknown_total_pos.format_position(), "Line 50 (?)");

        let end_pos = PositionInfo {
            current_line: 100,
            total_lines: Some(100),
            percentage: 1.0,
        };
        assert_eq!(end_pos.format_position(), "END");
    }

    #[test]
    fn test_percentage_calculation() {
        let mut status_line = StatusLine::new();

        // Test percentage calculation for different positions
        // Line 1 of 10 should be 10%
        status_line.update_position(0, Some(10));
        assert_eq!(status_line.position.percentage, 0.1);
        assert_eq!(status_line.position.format_position(), "Line 1/10 (10%)");

        // Line 5 of 10 should be 50%
        status_line.update_position(4, Some(10));
        assert_eq!(status_line.position.percentage, 0.5);
        assert_eq!(status_line.position.format_position(), "Line 5/10 (50%)");

        // Line 10 of 10 should be 100%
        status_line.update_position(9, Some(10));
        assert_eq!(status_line.position.percentage, 1.0);
        assert_eq!(status_line.position.format_position(), "Line 10/10 (100%)");

        // Edge case: single line file
        status_line.update_position(0, Some(1));
        assert_eq!(status_line.position.percentage, 1.0);
        assert_eq!(status_line.position.format_position(), "Line 1/1 (100%)");

        // Edge case: zero lines (should not crash)
        status_line.update_position(0, Some(0));
        assert_eq!(status_line.position.percentage, 0.0);
        assert_eq!(status_line.position.format_position(), "Empty");
    }

    #[test]
    fn test_file_display_info() {
        let path = PathBuf::from("/path/to/test.log");
        let mut info = FileDisplayInfo::new(&path);
        info.size = 1536; // 1.5 KB

        assert_eq!(info.filename(), "test.log");
        assert_eq!(info.format_size(), "1.5 KB");

        info.size = 2048 * 1024 * 1024; // 2 GB
        assert_eq!(info.format_size(), "2.0 GB");
    }

    #[test]
    fn test_display_config() {
        let mut config = DisplayConfig::default();
        assert!(!config.show_line_numbers);
        assert!(!config.wrap_lines);
        assert_eq!(config.tab_width, 8);

        config.toggle_line_numbers();
        assert!(config.show_line_numbers);

        config.set_tab_width(20); // Should be clamped to 16
        assert_eq!(config.tab_width, 16);

        config.set_tab_width(0); // Should be clamped to 1
        assert_eq!(config.tab_width, 1);
    }

    #[test]
    fn test_viewport_info() {
        let viewport = ViewportInfo::new(80, 24);
        assert_eq!(viewport.width, 80);
        assert_eq!(viewport.height, 24);
        assert_eq!(viewport.content_height, 22); // 24 - 2 for status line
        assert_eq!(viewport.lines_per_page(), 22);
        assert_eq!(viewport.half_page_size(), 11);

        // Test minimum half page size
        let small_viewport = ViewportInfo::new(80, 3);
        assert_eq!(small_viewport.half_page_size(), 1);
    }
}
