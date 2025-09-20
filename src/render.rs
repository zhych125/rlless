//! Rendering subsystem modules.
//!
//! Provides the render coordinator, protocol definitions, and terminal UI components used by the
//! high-level application.

pub mod protocol;
pub mod service;
pub mod ui;

pub use service::{RenderCoordinator, RenderLoopState};
