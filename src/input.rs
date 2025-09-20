//! Input subsystem scaffolding.
//!
//! Phase 1 introduces empty shells so we can migrate existing logic in later steps
//! without a massive diff.

pub mod raw;
pub mod service;
pub mod state;

// Public re-exports for convenience. Modules outside this crate should prefer importing
// from `crate::input` rather than reaching into submodules.
pub use service::InputService;
pub use state::{InputAction, InputState, InputStateMachine, SearchDirection};
