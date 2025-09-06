//! Terminal UI implementation using ratatui
//!
//! This module provides the concrete implementation of UIRenderer using ratatui
//! for cross-platform terminal interface. It integrates with existing FileAccessor
//! and SearchEngine components rather than managing data itself.

use crate::error::Result;
use crate::ui::{
    ColorTheme, NavigationCommand, SearchCommand, SearchDirection, UICommand, UIRenderer, ViewState,
};
use ratatui::crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::Paragraph,
    Frame, Terminal,
};
use std::io::{self, Stdout};
use std::time::Duration;

type CrosstermTerminal = Terminal<CrosstermBackend<Stdout>>;

/// Terminal UI implementation with ratatui backend
///
/// This implementation focuses purely on rendering and input handling.
/// Data management is handled by Application coordinating FileAccessor and SearchEngine.
pub struct TerminalUI {
    terminal: Option<CrosstermTerminal>,
    theme: ColorTheme,
}

impl TerminalUI {
    /// Create a new terminal UI instance with specified theme
    pub fn new() -> Result<Self> {
        Ok(Self {
            terminal: None,
            theme: ColorTheme::default(),
        })
    }

    /// Create terminal UI with custom theme
    pub fn with_theme(theme: ColorTheme) -> Result<Self> {
        Ok(Self {
            terminal: None,
            theme,
        })
    }

    /// Convert UI key events to UICommands
    fn key_to_command(&self, key: KeyCode, modifiers: KeyModifiers) -> Option<UICommand> {
        match (key, modifiers) {
            // Navigation commands
            (KeyCode::Char('j'), KeyModifiers::NONE) | (KeyCode::Down, _) => {
                Some(UICommand::Navigation(NavigationCommand::LineDown(1)))
            }
            (KeyCode::Char('k'), KeyModifiers::NONE) | (KeyCode::Up, _) => {
                Some(UICommand::Navigation(NavigationCommand::LineUp(1)))
            }
            (KeyCode::Char('f'), KeyModifiers::NONE)
            | (KeyCode::PageDown, _)
            | (KeyCode::Char(' '), KeyModifiers::NONE) => {
                Some(UICommand::Navigation(NavigationCommand::PageDown))
            }
            (KeyCode::Char('b'), KeyModifiers::NONE) | (KeyCode::PageUp, _) => {
                Some(UICommand::Navigation(NavigationCommand::PageUp))
            }
            (KeyCode::Char('d'), KeyModifiers::NONE) => {
                Some(UICommand::Navigation(NavigationCommand::HalfPageDown))
            }
            (KeyCode::Char('u'), KeyModifiers::NONE) => {
                Some(UICommand::Navigation(NavigationCommand::HalfPageUp))
            }
            (KeyCode::Char('g'), KeyModifiers::NONE) | (KeyCode::Home, _) => {
                Some(UICommand::Navigation(NavigationCommand::GoToStart))
            }
            (KeyCode::Char('G'), KeyModifiers::SHIFT) | (KeyCode::End, _) => {
                Some(UICommand::Navigation(NavigationCommand::GoToEnd))
            }

            // Search commands
            (KeyCode::Char('/'), KeyModifiers::NONE) => Some(UICommand::Search(
                SearchCommand::StartSearch(SearchDirection::Forward),
            )),
            (KeyCode::Char('?'), KeyModifiers::NONE) => Some(UICommand::Search(
                SearchCommand::StartSearch(SearchDirection::Backward),
            )),
            (KeyCode::Char('n'), KeyModifiers::NONE) => {
                Some(UICommand::Search(SearchCommand::NextMatch))
            }
            (KeyCode::Char('N'), KeyModifiers::SHIFT) => {
                Some(UICommand::Search(SearchCommand::PreviousMatch))
            }

            // Quit commands
            (KeyCode::Char('q'), KeyModifiers::NONE)
            | (KeyCode::Char('c'), KeyModifiers::CONTROL) => Some(UICommand::Quit),

            _ => None,
        }
    }

    /// Render content area with search highlights (helper for closure)
    fn render_content_with_data(
        frame: &mut Frame,
        area: Rect,
        view_state: &ViewState,
        theme: &ColorTheme,
    ) {
        let content_lines: Vec<Line> = view_state
            .visible_lines
            .iter()
            .enumerate()
            .map(|(idx, line)| {
                let line_number = view_state.viewport_top + idx as u64;

                // Check for search highlights on this line
                if let Some(highlights) = view_state.search_highlights.get(&line_number) {
                    Self::create_highlighted_line_with_theme(line.as_ref(), highlights, theme)
                } else {
                    Line::from(line.as_ref())
                }
            })
            .collect();

        let paragraph = Paragraph::new(content_lines);
        frame.render_widget(paragraph, area);
    }

    /// Create a line with search highlights applied using theme colors (helper for closure)
    fn create_highlighted_line_with_theme<'a>(
        content: &'a str,
        highlights: &[(usize, usize)],
        theme: &ColorTheme,
    ) -> Line<'a> {
        if highlights.is_empty() {
            return Line::from(content);
        }

        let mut spans = Vec::new();
        let mut last_end = 0;

        for &(start, end) in highlights {
            // Add normal text before highlight
            if start > last_end {
                spans.push(Span::raw(&content[last_end..start]));
            }

            // Add highlighted text using theme style directly
            if end > start && end <= content.len() {
                spans.push(Span::styled(&content[start..end], theme.search_match));
            }

            last_end = end;
        }

        // Add remaining normal text
        if last_end < content.len() {
            spans.push(Span::raw(&content[last_end..]));
        }

        Line::from(spans)
    }

    /// Render status line using theme colors (helper for closure)
    fn render_status_with_data(
        frame: &mut Frame,
        area: Rect,
        view_state: &ViewState,
        theme: &ColorTheme,
    ) {
        let status_text = format!(
            "{} | {} | {}",
            view_state.file_info.filename(),
            view_state.status_line.position.format_position(),
            view_state.status_line.search_info.as_deref().unwrap_or("")
        );

        // Use theme colors for status line directly
        let status_style = Style::default().bg(theme.status_bg).fg(theme.status_fg);

        let status = Paragraph::new(status_text).style(status_style);
        frame.render_widget(status, area);
    }
}

impl UIRenderer for TerminalUI {
    fn render(&mut self, view_state: &ViewState) -> Result<()> {
        if let Some(ref mut terminal) = self.terminal {
            // Extract theme before closure to avoid borrowing issues
            let theme = &self.theme;

            terminal.draw(move |frame| {
                let size = frame.size();

                // Split screen: content area and status line
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Min(0), Constraint::Length(1)].as_ref())
                    .split(size);

                // Render content area - highlights are now in view_state
                Self::render_content_with_data(frame, chunks[0], view_state, theme);

                // Render status line
                Self::render_status_with_data(frame, chunks[1], view_state, theme);
            })?;
        }
        Ok(())
    }

    fn handle_input(&mut self, timeout: Option<Duration>) -> Result<Option<UICommand>> {
        let timeout_duration = timeout.unwrap_or(Duration::from_millis(100));

        if event::poll(timeout_duration)? {
            if let Event::Key(key_event) = event::read()? {
                return Ok(self.key_to_command(key_event.code, key_event.modifiers));
            }
        }

        Ok(None)
    }

    fn initialize(&mut self) -> Result<()> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;

        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend)?;
        self.terminal = Some(terminal);

        Ok(())
    }

    fn cleanup(&mut self) -> Result<()> {
        if self.terminal.is_some() {
            disable_raw_mode()?;
            execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture)?;
            self.terminal = None;
        }
        Ok(())
    }

    fn get_terminal_size(&self) -> Result<(u16, u16)> {
        let (cols, rows) = ratatui::crossterm::terminal::size()?;
        Ok((cols, rows))
    }
}

impl Drop for TerminalUI {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Color;

    #[test]
    fn test_terminal_ui_creation() {
        let ui = TerminalUI::new();
        assert!(ui.is_ok());
        let ui = ui.unwrap();
        assert!(ui.terminal.is_none());

        // Test with custom theme
        let custom_theme = ColorTheme::monochrome();
        let ui_with_theme = TerminalUI::with_theme(custom_theme);
        assert!(ui_with_theme.is_ok());
    }

    #[test]
    fn test_theme_integration() {
        let ui = TerminalUI::new().unwrap();

        // Test that theme is properly integrated
        assert_eq!(ui.theme.status_fg, Color::White);
        assert_eq!(ui.theme.status_bg, Color::Blue);

        // Test custom theme
        let custom_theme = ColorTheme::monochrome();
        let ui_with_theme = TerminalUI::with_theme(custom_theme).unwrap();
        assert_eq!(ui_with_theme.theme.status_fg, Color::White);
        assert_eq!(ui_with_theme.theme.status_bg, Color::Black);
    }

    #[test]
    fn test_key_to_command_navigation() {
        let ui = TerminalUI::new().unwrap();

        // Test basic navigation
        assert_eq!(
            ui.key_to_command(KeyCode::Char('j'), KeyModifiers::NONE),
            Some(UICommand::Navigation(NavigationCommand::LineDown(1)))
        );

        assert_eq!(
            ui.key_to_command(KeyCode::Char('k'), KeyModifiers::NONE),
            Some(UICommand::Navigation(NavigationCommand::LineUp(1)))
        );

        assert_eq!(
            ui.key_to_command(KeyCode::Char(' '), KeyModifiers::NONE),
            Some(UICommand::Navigation(NavigationCommand::PageDown))
        );

        assert_eq!(
            ui.key_to_command(KeyCode::Char('G'), KeyModifiers::SHIFT),
            Some(UICommand::Navigation(NavigationCommand::GoToEnd))
        );
    }

    #[test]
    fn test_key_to_command_search() {
        let ui = TerminalUI::new().unwrap();

        assert_eq!(
            ui.key_to_command(KeyCode::Char('/'), KeyModifiers::NONE),
            Some(UICommand::Search(SearchCommand::StartSearch(
                SearchDirection::Forward
            )))
        );

        assert_eq!(
            ui.key_to_command(KeyCode::Char('?'), KeyModifiers::NONE),
            Some(UICommand::Search(SearchCommand::StartSearch(
                SearchDirection::Backward
            )))
        );

        assert_eq!(
            ui.key_to_command(KeyCode::Char('n'), KeyModifiers::NONE),
            Some(UICommand::Search(SearchCommand::NextMatch))
        );
    }

    #[test]
    fn test_key_to_command_quit() {
        let ui = TerminalUI::new().unwrap();

        assert_eq!(
            ui.key_to_command(KeyCode::Char('q'), KeyModifiers::NONE),
            Some(UICommand::Quit)
        );

        assert_eq!(
            ui.key_to_command(KeyCode::Char('c'), KeyModifiers::CONTROL),
            Some(UICommand::Quit)
        );
    }
}
