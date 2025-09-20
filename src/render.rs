//! Rendering subsystem scaffolding.
//!
//! Phase 1 introduces shells for the render loop, protocol, and renderer so they can be
//! populated incrementally.

pub mod protocol;
pub mod service;
pub mod ui;

// Re-exports will be added once implementations migrate here.
// pub use service::RenderService;
