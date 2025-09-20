//! Terminal UI module with ratatui
//!
//! This module provides a high-performance terminal interface for rlless using the ratatui
//! library. It follows a trait-based architecture with command pattern for event handling.

pub mod renderer;
pub mod state;
pub mod terminal;
pub mod theme;

// Re-export public API
pub use crate::input::{
    InputAction, InputService, InputState, InputStateMachine, ScrollDirection, SearchDirection,
};
pub use ratatui::style::{Color, Style};
pub use renderer::UIRenderer;
pub use state::{DisplayMode, StatusLine, ViewState};
pub use terminal::TerminalUI;
pub use theme::ColorTheme;

#[cfg(test)]
pub use renderer::tests::MockUIRenderer;
