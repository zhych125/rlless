//! Terminal rendering components.
//!
//! This module hosts the concrete terminal UI implementation along with the supporting view/state
//! structures and styling utilities.

pub mod renderer;
pub mod state;
pub mod terminal;
pub mod theme;

pub use renderer::UIRenderer;
pub use state::{DisplayMode, StatusLine, ViewState};
pub use terminal::TerminalUI;
pub use theme::ColorTheme;

#[cfg(test)]
pub use renderer::tests::MockUIRenderer;

pub use ratatui::style::{Color, Style};
