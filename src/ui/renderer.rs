//! UI renderer trait and event handling
//!
//! This module defines the UIRenderer trait for rendering terminal interfaces
//! and handling user input events in an event-driven architecture.

use crate::error::Result;
use crate::ui::{InputAction, ViewState};
use std::time::Duration;

/// Core trait for UI rendering and event handling
pub trait UIRenderer {
    /// Render the current view state to the terminal
    ///
    /// This method should:
    /// - Clear and redraw the content area
    /// - Apply search highlights if provided by SearchEngine
    /// - Update the status line
    /// - Handle terminal resizing
    fn render(&mut self, view_state: &ViewState) -> Result<()>;

    /// Handle user input and return the next input action
    ///
    /// This method should:
    /// - Block until user input or timeout
    /// - Parse key combinations into InputActions
    /// - Handle mode-specific input (search input vs navigation)
    /// - Return None on timeout for periodic updates
    fn handle_input(&mut self, timeout: Option<Duration>) -> Result<Option<InputAction>>;

    /// Initialize the terminal UI
    ///
    /// This method should:
    /// - Set up raw mode
    /// - Hide cursor
    /// - Clear screen
    /// - Set up signal handlers
    fn initialize(&mut self) -> Result<()>;

    /// Clean up and restore terminal state
    ///
    /// This method should:
    /// - Restore cursor
    /// - Exit raw mode
    /// - Clear screen if needed
    fn cleanup(&mut self) -> Result<()>;

    /// Get current terminal dimensions
    fn get_terminal_size(&self) -> Result<(u16, u16)>; // (width, height)
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::ui::InputAction;
    use std::collections::VecDeque;

    /// Mock UI renderer for testing
    ///
    /// This mock allows tests to:
    /// - Verify render calls were made
    /// - Simulate user input sequences
    /// - Test UI command generation
    pub struct MockUIRenderer {
        pub render_count: usize,
        pub terminal_size: (u16, u16),
        pub input_sequence: VecDeque<InputAction>,
        pub is_initialized: bool,
    }

    impl Default for MockUIRenderer {
        fn default() -> Self {
            Self::new()
        }
    }

    impl MockUIRenderer {
        /// Create a new mock renderer with default settings
        pub fn new() -> Self {
            Self {
                render_count: 0,
                terminal_size: (80, 24),
                input_sequence: VecDeque::new(),
                is_initialized: false,
            }
        }

        /// Add a command to the input sequence for testing
        pub fn add_input(&mut self, action: InputAction) {
            self.input_sequence.push_back(action);
        }

        /// Set terminal size for testing
        pub fn set_terminal_size(&mut self, width: u16, height: u16) {
            self.terminal_size = (width, height);
        }
    }

    impl UIRenderer for MockUIRenderer {
        fn render(&mut self, _view_state: &ViewState) -> Result<()> {
            self.render_count += 1;
            Ok(())
        }

        fn handle_input(&mut self, _timeout: Option<Duration>) -> Result<Option<InputAction>> {
            Ok(self.input_sequence.pop_front())
        }

        fn initialize(&mut self) -> Result<()> {
            self.is_initialized = true;
            Ok(())
        }

        fn cleanup(&mut self) -> Result<()> {
            self.is_initialized = false;
            Ok(())
        }

        fn get_terminal_size(&self) -> Result<(u16, u16)> {
            Ok(self.terminal_size)
        }
    }

    #[test]
    fn test_mock_renderer_basic() {
        use std::path::PathBuf;

        let mut renderer = MockUIRenderer::new();
        let view_state = ViewState::new(PathBuf::from("/test"), 80, 24);

        // Test initialization
        assert!(!renderer.is_initialized);
        renderer.initialize().unwrap();
        assert!(renderer.is_initialized);

        // Test rendering
        assert_eq!(renderer.render_count, 0);
        renderer.render(&view_state).unwrap();
        assert_eq!(renderer.render_count, 1);

        // Test input simulation
        renderer.add_input(InputAction::ScrollDown(1));
        let cmd = renderer.handle_input(None).unwrap();
        assert_eq!(cmd, Some(InputAction::ScrollDown(1)));

        // Test terminal size
        let size = renderer.get_terminal_size().unwrap();
        assert_eq!(size, (80, 24));
        // Test cleanup
        renderer.cleanup().unwrap();
        assert!(!renderer.is_initialized);
    }

    #[test]
    fn test_mock_renderer_input_sequence() {
        let mut renderer = MockUIRenderer::new();

        // Add multiple commands
        renderer.add_input(InputAction::PageDown);
        renderer.add_input(InputAction::GoToEnd);
        renderer.add_input(InputAction::Quit);

        // Verify they come out in order
        assert_eq!(
            renderer.handle_input(None).unwrap(),
            Some(InputAction::PageDown)
        );
        assert_eq!(
            renderer.handle_input(None).unwrap(),
            Some(InputAction::GoToEnd)
        );
        assert_eq!(
            renderer.handle_input(None).unwrap(),
            Some(InputAction::Quit)
        );
        assert_eq!(renderer.handle_input(None).unwrap(), None);
    }
}
