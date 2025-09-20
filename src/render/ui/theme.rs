//! Color theme and styling definitions using ratatui colors
//!
//! This module provides color themes for terminal rendering using ratatui's
//! color system directly to avoid unnecessary abstractions.

use ratatui::style::{Color, Style};

/// Color theme for terminal UI elements
#[derive(Debug, Clone)]
pub struct ColorTheme {
    /// Normal text color (None uses terminal default)
    pub normal_text: Option<Color>,

    /// Search match highlighting
    pub search_match: Style,

    /// Current/focused search match
    pub current_match: Style,

    /// Status line background
    pub status_bg: Color,

    /// Status line text
    pub status_fg: Color,

    /// Line numbers (when enabled)
    pub line_numbers: Option<Color>,

    /// Error/warning text
    pub error_text: Color,

    /// Selection highlighting
    pub selection: Style,
}

impl Default for ColorTheme {
    /// Default color theme similar to less/more
    fn default() -> Self {
        Self {
            normal_text: None, // Use terminal default
            search_match: Style::default().fg(Color::Black).bg(Color::Yellow),
            current_match: Style::default().fg(Color::Black).bg(Color::LightYellow),
            status_bg: Color::Blue,
            status_fg: Color::White,
            line_numbers: Some(Color::DarkGray),
            error_text: Color::Red,
            selection: Style::default().fg(Color::White).bg(Color::Blue),
        }
    }
}

impl ColorTheme {
    /// Create a monochrome theme for terminals without color support
    pub fn monochrome() -> Self {
        Self {
            normal_text: None,
            search_match: Style::default().fg(Color::Black).bg(Color::White),
            current_match: Style::default().fg(Color::White).bg(Color::Black),
            status_bg: Color::Black,
            status_fg: Color::White,
            line_numbers: None,
            error_text: Color::White,
            selection: Style::default().fg(Color::Black).bg(Color::White),
        }
    }

    /// Create a high-contrast theme for accessibility
    pub fn high_contrast() -> Self {
        Self {
            normal_text: Some(Color::White),
            search_match: Style::default().fg(Color::Black).bg(Color::LightYellow),
            current_match: Style::default().fg(Color::LightYellow).bg(Color::Black),
            status_bg: Color::White,
            status_fg: Color::Black,
            line_numbers: Some(Color::LightGreen),
            error_text: Color::LightRed,
            selection: Style::default().fg(Color::White).bg(Color::LightBlue),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_theme() {
        let theme = ColorTheme::default();
        assert_eq!(theme.normal_text, None);
        assert_eq!(theme.status_fg, Color::White);
        assert_eq!(theme.status_bg, Color::Blue);

        // Test search match style
        assert_eq!(theme.search_match.fg, Some(Color::Black));
        assert_eq!(theme.search_match.bg, Some(Color::Yellow));
    }

    #[test]
    fn test_monochrome_theme() {
        let theme = ColorTheme::monochrome();
        assert_eq!(theme.line_numbers, None);
        assert_eq!(theme.status_fg, Color::White);
        assert_eq!(theme.status_bg, Color::Black);

        // Test monochrome search highlighting
        assert_eq!(theme.search_match.fg, Some(Color::Black));
        assert_eq!(theme.search_match.bg, Some(Color::White));
    }

    #[test]
    fn test_high_contrast_theme() {
        let theme = ColorTheme::high_contrast();
        assert_eq!(theme.normal_text, Some(Color::White));
        assert_eq!(theme.error_text, Color::LightRed);
        assert_eq!(theme.status_bg, Color::White);
        assert_eq!(theme.status_fg, Color::Black);
    }

    #[test]
    fn test_style_creation() {
        let style = Style::default().fg(Color::Black).bg(Color::Yellow);

        assert_eq!(style.fg, Some(Color::Black));
        assert_eq!(style.bg, Some(Color::Yellow));

        let fg_only = Style::default().fg(Color::Red);
        assert_eq!(fg_only.fg, Some(Color::Red));
        assert_eq!(fg_only.bg, None);
    }

    #[test]
    fn test_color_variants() {
        // Test standard colors
        assert_eq!(Color::Red, Color::Red);
        assert_ne!(Color::Red, Color::Blue);

        // Test indexed color
        let indexed = Color::Indexed(42);
        assert_eq!(indexed, Color::Indexed(42));
        assert_ne!(indexed, Color::Indexed(43));

        // Test RGB color
        let rgb = Color::Rgb(255, 128, 64);
        assert_eq!(rgb, Color::Rgb(255, 128, 64));
        assert_ne!(rgb, Color::Rgb(255, 128, 65));
    }
}
