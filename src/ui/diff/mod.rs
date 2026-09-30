//! Dual-pane Diff: pure Layout (per file) and Viewport (per frame), and the
//! DualPane Entity that owns them. See docs/diffview-architecture.md.

pub mod element;
pub mod layout;
pub mod pane;
pub mod review;
mod tabs;
mod trace;
pub mod viewport;
mod visual_wrap;
mod wrap;

#[cfg(test)]
mod perf;
