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


#[cfg(test)]
mod perf;
