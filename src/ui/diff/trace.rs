//! `REVIEWFOX_FRAME_TRACE=1`: one stderr line per drawn dual-pane frame
//! (Viewport, frame build, newly shaped rows, paint, visible rows) and one per
//! Layout build. Off by default. The env var is read once; when off, every
//! hook is a load of a cached bool and nothing is timed or allocated.

use std::sync::OnceLock;
use std::time::{Duration, Instant};

pub fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("REVIEWFOX_FRAME_TRACE").is_some_and(|v| v == "1"))
}

/// `Some(now)` only when tracing.
pub fn start() -> Option<Instant> {
    enabled().then(Instant::now)
}

/// Time since `t`, zero when not tracing.
pub fn since(t: Option<Instant>) -> Duration {
    t.map(|t| t.elapsed()).unwrap_or_default()
}

/// Prepaint numbers carried to paint in the Frame. Plain `Copy` data.
#[derive(Clone, Copy, Default)]
pub struct FrameStats {
    pub viewport: Duration,
    /// Whole paint-list build, shaping included.
    pub build: Duration,
    pub shape: Duration,
    /// Rows shaped this frame (cache misses).
    pub shaped: u32,
    /// Visible rows `[old, new]`.
    pub rows: [usize; 2],
    pub prepaint: Duration,
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1e3
}

pub fn frame(stats: &FrameStats, paint: Duration) {
    eprintln!(
        "[frame-trace] frame: viewport {:.3}ms build {:.3}ms shape {} rows {:.3}ms prepaint {:.3}ms paint {:.3}ms visible {}+{} rows",
        ms(stats.viewport),
        ms(stats.build),
        stats.shaped,
        ms(stats.shape),
        ms(stats.prepaint),
        ms(paint),
        stats.rows[0],
        stats.rows[1],
    );
}

pub fn layout(took: Duration, rows: [usize; 2], bridges: usize, soft_wrap: bool) {
    eprintln!(
        "[frame-trace] layout build {:.3}ms rows {}+{} bridges {}{}",
        ms(took),
        rows[0],
        rows[1],
        bridges,
        if soft_wrap { " soft-wrap" } else { "" },
    );
}

pub fn highlight(took: Duration) {
    eprintln!(
        "[frame-trace] highlight {:.3}ms (count={})",
        ms(took),
        crate::syntax::highlight_count(),
    );
}
