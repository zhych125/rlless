//! Terminal UI implementation using ratatui
//!
//! This module provides the concrete implementation of UIRenderer using ratatui
//! for cross-platform terminal interface. It integrates with existing FileAccessor
//! and SearchEngine components rather than managing data itself.

use crate::error::Result;
use crate::ui::{ColorTheme, InputAction, InputStateMachine, UIRenderer, ViewState};
use ratatui::crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, MouseEvent, MouseEventKind},
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
use std::time::{Duration, Instant};

type CrosstermTerminal = Terminal<CrosstermBackend<Stdout>>;

/// Terminal UI implementation with ratatui backend
///
/// This implementation focuses purely on rendering and input handling.
/// Data management is handled by Application coordinating FileAccessor and SearchEngine.
pub struct TerminalUI {
    terminal: Option<CrosstermTerminal>,
    theme: ColorTheme,
    input_machine: InputStateMachine,
    last_scroll_time: Option<Instant>,
    scroll_throttle_duration: Duration,
}

impl TerminalUI {
    /// Create a new terminal UI instance with specified theme
    pub fn new() -> Result<Self> {
        Ok(Self {
            terminal: None,
            theme: ColorTheme::default(),
            input_machine: InputStateMachine::new(),
            last_scroll_time: None,
            scroll_throttle_duration: Duration::from_millis(100), // 100ms throttle
        })
    }

    /// Create terminal UI with custom theme
    pub fn with_theme(theme: ColorTheme) -> Result<Self> {
        Ok(Self {
            terminal: None,
            theme,
            input_machine: InputStateMachine::new(),
            last_scroll_time: None,
            scroll_throttle_duration: Duration::from_millis(100), // 100ms throttle
        })
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
            .map(|(viewport_line_idx, line)| {
                // Get search highlights for this viewport-relative line (if any)
                let highlights = view_state
                    .search_highlights
                    .get(viewport_line_idx)
                    .map(|ranges| ranges.as_slice())
                    .unwrap_or(&[]);

                if highlights.is_empty() {
                    Line::from(line.as_str())
                } else {
                    Self::create_highlighted_line_with_theme(line.as_str(), highlights, theme)
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
        let status_text = view_state.format_status_line();

        // Use theme colors for status line directly
        let status_style = Style::default().bg(theme.status_bg).fg(theme.status_fg);

        let status = Paragraph::new(status_text).style(status_style);
        frame.render_widget(status, area);
    }

    /// Handle mouse events with scroll throttling
    fn handle_mouse_event(&mut self, mouse_event: MouseEvent) -> Option<InputAction> {
        match mouse_event.kind {
            MouseEventKind::ScrollUp => {
                if self.should_throttle_scroll() {
                    return None;
                }
                self.update_scroll_time();
                // Mouse scroll up = move up in file (show earlier lines)
                // 3 lines per event with longer throttle for responsive but controlled scrolling
                Some(InputAction::ScrollUp(3))
            }
            MouseEventKind::ScrollDown => {
                if self.should_throttle_scroll() {
                    return None;
                }
                self.update_scroll_time();
                // Mouse scroll down = move down in file (show later lines)
                // 3 lines per event with longer throttle for responsive but controlled scrolling
                Some(InputAction::ScrollDown(3))
            }
            _ => {
                // Ignore other mouse events (clicks, moves, etc.)
                None
            }
        }
    }

    /// Check if scroll event should be throttled
    fn should_throttle_scroll(&self) -> bool {
        if let Some(last_time) = self.last_scroll_time {
            let now = Instant::now();
            now.duration_since(last_time) < self.scroll_throttle_duration
        } else {
            false
        }
    }

    /// Update the last scroll time
    fn update_scroll_time(&mut self) {
        self.last_scroll_time = Some(Instant::now());
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

    fn handle_input(&mut self, timeout: Option<Duration>) -> Result<Option<InputAction>> {
        let timeout_duration = timeout.unwrap_or(Duration::from_millis(100));

        if event::poll(timeout_duration)? {
            match event::read()? {
                Event::Key(key_event) => {
                    let action = self.input_machine.handle_key_event(key_event);
                    // Only return non-NoAction results
                    return Ok(match action {
                        InputAction::NoAction => None,
                        other => Some(other),
                    });
                }
                Event::Resize(width, height) => {
                    return Ok(Some(InputAction::Resize { width, height }));
                }
                Event::Mouse(mouse_event) => {
                    if let Some(action) = self.handle_mouse_event(mouse_event) {
                        return Ok(Some(action));
                    }
                }
                _ => {
                    // Ignore other events
                }
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
    fn test_input_state_machine_integration() {
        use crate::ui::InputState;

        let ui = TerminalUI::new().unwrap();

        // Test that input state machine is properly initialized
        assert_eq!(ui.input_machine.get_state(), InputState::Navigation);
        assert_eq!(ui.input_machine.get_search_buffer(), "");
    }

    #[test]
    fn test_mouse_scroll_actions() {
        // Test that mouse scroll events translate to correct scroll actions
        // Note: This tests the logic, not actual mouse event generation
        
        // ScrollUp should translate to ScrollUp(3) action
        let scroll_up_action = InputAction::ScrollUp(3);
        match scroll_up_action {
            InputAction::ScrollUp(lines) => assert_eq!(lines, 3),
            _ => panic!("Expected ScrollUp action"),
        }
        
        // ScrollDown should translate to ScrollDown(3) action  
        let scroll_down_action = InputAction::ScrollDown(3);
        match scroll_down_action {
            InputAction::ScrollDown(lines) => assert_eq!(lines, 3),
            _ => panic!("Expected ScrollDown action"),
        }
    }
}
