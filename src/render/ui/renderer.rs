//! UI renderer trait and event handling
//!
//! This module defines the `UIRenderer` trait for rendering terminal interfaces and managing
//! lifecycle hooks such as initialization and cleanup.

use crate::error::Result;
use crate::render::ui::state::ViewState;

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
    use crate::render::ui::state::ViewState;

    /// Mock UI renderer for testing
    ///
    /// This mock allows tests to verify render invocations and terminal sizing logic.
    pub struct MockUIRenderer {
        pub render_count: usize,
        pub terminal_size: (u16, u16),
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
                is_initialized: false,
            }
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

        // Test terminal size
        let size = renderer.get_terminal_size().unwrap();
        assert_eq!(size, (80, 24));
        // Test cleanup
        renderer.cleanup().unwrap();
        assert!(!renderer.is_initialized);
    }

    #[test]
    fn test_mock_renderer_resize_handling() {
        let mut renderer = MockUIRenderer::new();
        renderer.set_terminal_size(120, 30);
        assert_eq!(renderer.get_terminal_size().unwrap(), (120, 30));
    }
}
