//! Renderer implementation scaffolding.
//!
//! Concrete terminal rendering (currently `TerminalUI`) will move here in later phases.

use crate::error::Result;
use crate::ui::ViewState;

/// Placeholder trait for renderers. Once migrated, this will likely mirror the existing
/// `UIRenderer` trait.
pub trait Renderer {
    /// Render the current view.
    fn render(&mut self, _view_state: &ViewState) -> Result<()> {
        Ok(())
    }
}
