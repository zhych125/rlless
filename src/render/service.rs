//! Render coordination service scaffolding.
//!
//! Will own the render loop, manage input actions, and coordinate search requests.

use crate::ui::ViewState;

/// Placeholder render service struct.
#[derive(Debug, Default)]
pub struct RenderService;

impl RenderService {
    /// Create a new placeholder render service.
    pub fn new() -> Self {
        Self
    }

    /// Run the render loop.
    pub async fn run(&mut self, _view_state: &mut ViewState) {
        // No-op until the implementation is migrated.
    }
}
