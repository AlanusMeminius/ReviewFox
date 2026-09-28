//! Dual-pane Diff: pure Layout (per file) and Viewport (per frame), and the
//! DualPane Entity that owns them. See docs/diffview-architecture.md.

pub mod element;
pub mod layout;
pub mod pane;
mod tabs;
mod trace;
mod visual_wrap;
mod wrap;
pub mod viewport;

#[allow(unused_imports)]
pub use visual_wrap::{AppliedWrap, WrapPlan, WrapSide};

#[cfg(test)]
mod perf;
