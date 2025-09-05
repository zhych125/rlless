//! Terminal UI module with ratatui
//!
//! This module provides a high-performance terminal interface for rlless using the ratatui
//! library. It follows a trait-based architecture with command pattern for event handling.

pub mod commands;
pub mod renderer;
pub mod state;
pub mod terminal;
pub mod theme;

// Re-export public API
pub use commands::{
    DisplayCommand, FileCommand, NavigationCommand, SearchCommand, SearchDirection, UICommand,
};
pub use ratatui::style::{Color, Style};
pub use renderer::UIRenderer;
pub use state::{
    DisplayConfig, DisplayMode, FileDisplayInfo, HighlightStyle, LineDisplayUtils, PositionInfo,
    StatusLine, ViewState, ViewportInfo,
};
pub use terminal::TerminalUI;
pub use theme::ColorTheme;

#[cfg(test)]
pub use renderer::tests::MockUIRenderer;
