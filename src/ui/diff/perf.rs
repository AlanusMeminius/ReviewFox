//! Headless timing of the pure per-frame path (Viewport + visible-row paint
//! list, as `element::build_frame` builds it minus glyph shaping). Ignored by
//! default; run in release:
//!
//! - `cargo test --release -- --ignored frame_cost_is_flat_in_file_length --nocapture`
//! - `cargo test --release -- --ignored rewrap_cost_report --nocapture`

use std::hint::black_box;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::layout::{Layout, LineKind, Row};
use super::pane::nearest_hunk_index;
use super::visual_wrap::{WrapPlan, WrapSide};
use super::viewport::{self, Viewport, snap};
use crate::domain::{Alignment, AlignmentOp, FoldState, LineSpan, Side};

const ROW_H: f32 = 20.;
const VIEW_H: f32 = 900.;
const SCALE: f32 = 1.5;
const FRAMES: usize = 2000;

/// A file of about `lines` lines per side with `hunks` Hunks cycling through
/// insert / delete / replace (3→2, 2→4) between Equal runs. Every fifth Equal
/// run is expanded, the rest fold.
fn synthetic(lines: u32, hunks: u32) -> (String, String, Alignment, FoldState) {
    let (mut old, mut new) = (String::new(), String::new());
    let (mut o, mut n) = (1u32, 1u32);
    let mut ops = Vec::new();
    let mut fold = FoldState::collapsed();
    let equal = (lines / hunks).saturating_sub(3).max(1);
    let span = |start, count| LineSpan { start, count };
    let push = |text: &mut String, ln: u32, tag: &str| {
        text.push_str(&format!(
            "    let value_{ln} = compute({tag}, {ln}) + offset * {ln};\n"
        ));
    };
    for h in 0..hunks {
        if h % 5 == 0 {
            fold.expand(ops.len());
        }
        ops.push(AlignmentOp::Equal {
            old: span(o, equal),
            new: span(n, equal),
        });
        for i in 0..equal {
            push(&mut old, o + i, "same");
            push(&mut new, o + i, "same");
        }
        o += equal;
        n += equal;
        let (olds, news) = match h % 4 {
            0 => (0, 3),
            1 => (3, 0),
            2 => (3, 2),
            _ => (2, 4),
        };
        for i in 0..olds {
            push(&mut old, o + i, "before");
        }
        for i in 0..news {
            push(&mut new, n + i, "after");
        }
        ops.push(match (olds, news) {
            (0, _) => AlignmentOp::Insert {
                after_old: o - 1,
                news: span(n, news),
            },
            (_, 0) => AlignmentOp::Delete {
                olds: span(o, olds),
                at_new: n - 1,
            },
            _ => AlignmentOp::Replace {
                olds: span(o, olds),
                news: span(n, news),
            },
        });
        o += olds;
        n += news;
    }
    ops.push(AlignmentOp::Equal {
        old: span(o, equal),
        new: span(n, equal),
    });
    for i in 0..equal {
        push(&mut old, o + i, "tail");
        push(&mut new, n + i, "tail");
    }
    (old, new, Alignment { ops }, fold)
}

/// One display line for wrap perf: short, long, punct-heavy, or megabyte-ish.
fn synthetic_wrap_line(ln: u32, tag: &str) -> String {
    match ln % 23 {
        0 => format!("    let value_{ln} = compute({tag}, {ln});\n"),
        1 => format!("}}\n"),
        2 => format!("    if cond_{ln} {{ do_work({tag}); }}\n"),
        3..=5 => {
            let mut s = format!("    // context {tag} line {ln}: ");
            while s.len() < 280 {
                s.push_str("data ");
            }
            s.push('\n');
            s
        }
        6..=8 => format!(
            "    ptr->next->child[{ln}]->flags |= MASK_{ln} && ok_{ln} // tail\n"
        ),
        9..=11 => format!(
            "    stream << item_{ln} << delim << a->b->c<<=d&&e// note {ln}\n"
        ),
        12..=14 => {
            let mut s = String::from("    ");
            for i in 0..120 {
                s.push_str(&format!("x{i}->"));
            }
            s.push_str(&format!("end_{ln};\n"));
            s
        }
        15..=17 => format!("    处理行_{ln} 与 {tag} 混合 ascii;\n"),
        18..=20 => {
            let mut s = format!("    call({tag}, {ln}, ");
            while s.len() < 300 {
                s.push_str("arg, ");
            }
            s.push_str("done);\n");
            s
        }
        _ if ln % 500 == 0 => {
            let mut s = format!("    // meg line {ln} ");
            while s.len() < 2000 {
                s.push_str("a->b->c<<=d&&e//");
            }
            s.push('\n');
            s
        }
        _ => format!("    ok_{ln}();\n"),
    }
}

/// Like [`synthetic`] but with [`synthetic_wrap_line`] bodies for rewrap timing.
fn synthetic_wrap(lines: u32, hunks: u32) -> (String, String, Alignment, FoldState) {
    let (mut old, mut new) = (String::new(), String::new());
    let (mut o, mut n) = (1u32, 1u32);
    let mut ops = Vec::new();
    let mut fold = FoldState::collapsed();
    let equal = (lines / hunks).saturating_sub(3).max(1);
    let span = |start, count| LineSpan { start, count };
    let push = |text: &mut String, ln: u32, tag: &str| {
        text.push_str(&synthetic_wrap_line(ln, tag));
    };
    for h in 0..hunks {
        if h % 5 == 0 {
            fold.expand(ops.len());
        }
        ops.push(AlignmentOp::Equal {
            old: span(o, equal),
            new: span(n, equal),
        });
        for i in 0..equal {
            push(&mut old, o + i, "same");
            push(&mut new, o + i, "same");
        }
        o += equal;
        n += equal;
        let (olds, news) = match h % 4 {
            0 => (0, 3),
            1 => (3, 0),
            2 => (3, 2),
            _ => (2, 4),
        };
        for i in 0..olds {
            push(&mut old, o + i, "before");
        }
        for i in 0..news {
            push(&mut new, n + i, "after");
        }
        ops.push(match (olds, news) {
            (0, _) => AlignmentOp::Insert {
                after_old: o - 1,
                news: span(n, news),
            },
            (_, 0) => AlignmentOp::Delete {
                olds: span(o, olds),
                at_new: n - 1,
            },
            _ => AlignmentOp::Replace {
                olds: span(o, olds),
                news: span(n, news),
            },
        });
        o += olds;
        n += news;
    }
    ops.push(AlignmentOp::Equal {
        old: span(o, equal),
        new: span(n, equal),
    });
    for i in 0..equal {
        push(&mut old, o + i, "tail");
        push(&mut new, n + i, "tail");
    }
    (old, new, Alignment { ops }, fold)
}

const REWRAP_WARM: usize = 15;

fn fake_char_width(c: char) -> f32 {
    if c.is_ascii() { 8. } else { 16. }
}

struct RewrapRun {
    wrap_off: Duration,
    wrap_median: Duration,
    wrap_max: Duration,
    rows: [usize; 2],
}

fn median(mut samples: Vec<Duration>) -> Duration {
    samples.sort();
    samples[samples.len() / 2]
}

fn time_layout_rewrap(
    old: &str,
    new: &str,
    alignment: &Alignment,
    fold: Option<&FoldState>,
    width_px: f32,
) -> RewrapRun {
    let old_arc: Arc<str> = old.into();
    let new_arc: Arc<str> = new.into();
    let t0 = Instant::now();
    let off = Layout::build(
        Arc::clone(&old_arc),
        Arc::clone(&new_arc),
        alignment,
        fold,
        None,
    );
    let wrap_off = t0.elapsed();
    black_box((off.old.rows(), off.new.rows()));

    let plan = WrapPlan {
        old: WrapSide { width_px },
        new: WrapSide { width_px },
    };
    for _ in 0..2 {
        let mut cw = fake_char_width;
        let _ = Layout::build(
            Arc::clone(&old_arc),
            Arc::clone(&new_arc),
            alignment,
            fold,
            Some((&plan, &mut cw)),
        );
    }
    let mut samples = Vec::with_capacity(REWRAP_WARM);
    let mut max = Duration::ZERO;
    let mut rows = [0usize; 2];
    for _ in 0..REWRAP_WARM {
        let t = Instant::now();
        let mut cw = fake_char_width;
        let layout = Layout::build(
            Arc::clone(&old_arc),
            Arc::clone(&new_arc),
            alignment,
            fold,
            Some((&plan, &mut cw)),
        );
        let took = t.elapsed();
        samples.push(took);
        max = max.max(took);
        rows = [layout.old.rows(), layout.new.rows()];
        black_box(layout.wrap.is_some());
    }
    let wrap_median = median(samples);
    RewrapRun {
        wrap_off,
        wrap_median,
        wrap_max: max,
        rows,
    }
}

fn wrap_off_build(
    old: &str,
    new: &str,
    alignment: &Alignment,
    fold: Option<&FoldState>,
) -> Duration {
    let t = Instant::now();
    let layout = Layout::build(old.into(), new.into(), alignment, fold, None);
    black_box((layout.old.rows(), layout.new.rows()));
    t.elapsed()
}

/// What paint needs for one row, without the shaped glyphs.
#[allow(dead_code)] // Fields exist so the list costs what the real one does.
struct PlanRow {
    y0: f32,
    y1: f32,
    kind: Option<LineKind>,
    commented: bool,
    text_len: usize,
    marks: Vec<(usize, usize)>,
}

/// One frame of the pure path: clamp, Viewport, visible rows with marks and
/// comment lookups, gaps, seams, bridges, omit links, hunk index. Returns
/// the visible row count.
fn frame(layout: &Layout, scroll_s: &mut f32, dy: f32) -> usize {
    *scroll_s = viewport::clamp_s(layout, *scroll_s + dy, VIEW_H, ROW_H);
    let vp = Viewport::new(layout, *scroll_s, VIEW_H, ROW_H).snapped(SCALE);
    let mut visible = 0;
    for side in [Side::Old, Side::New] {
        let rows = layout.side(side);
        let range = vp.visible_rows(side);
        visible += range.len();
        let top = vp.top(side);
        let y_of = |r: usize| snap(r as f32 * ROW_H - top, SCALE);
        let mut out = Vec::with_capacity(range.len());
        for i in range {
            let Some(row) = rows.row(i) else { continue };
            let (kind, commented, text_len, marks) = match row {
                Row::Line(l) => {
                    let text = rows.text(l);
                    let mut at = 0;
                    let mut marks = Vec::new();
                    for part in layout.marks(side, l).unwrap_or(&[]) {
                        let end = at + part.text.len();
                        if part.changed {
                            marks.push((at, end));
                        }
                        at = end;
                    }
                    (Some(l.kind), rows.has_comment(l.ln), text.len(), marks)
                }
                Row::Omit(_) => (None, false, 0, Vec::new()),
            };
            out.push(PlanRow {
                y0: y_of(i),
                y1: y_of(i + 1),
                kind,
                commented,
                text_len,
                marks,
            });
        }
        black_box(&out);
        black_box(vp.gaps(side));
        black_box(
            vp.visible_seams(side)
                .iter()
                .map(|&r| y_of(r as usize))
                .collect::<Vec<_>>(),
        );
        black_box(vp.max_top(side));
    }
    black_box(vp.bridges());
    black_box(vp.omit_links());
    black_box(nearest_hunk_index(vp.s() / ROW_H, &layout.hunk_lands));
    visible
}

struct Run {
    build: Duration,
    rows: [usize; 2],
    cold: Duration,
    warm: Duration,
    warm_max: Duration,
    visible: usize,
}

/// Build the Layout, then scroll the whole `s_range` top to bottom in
/// `FRAMES` steps twice (cold: word marks computed as rows appear; warm:
/// memoized) and time each frame.
fn run(lines: u32, hunks: u32, folded: bool) -> Run {
    let (old, new, alignment, fold) = synthetic(lines, hunks);
    let t = Instant::now();
    let layout = Layout::build(old.into(), new.into(), &alignment, folded.then_some(&fold), None);
    black_box((layout.old.max_chars(), layout.new.max_chars()));
    let build = t.elapsed();
    let (lo, hi) = viewport::s_range(&layout, VIEW_H, ROW_H);
    let dy = (hi - lo) / FRAMES as f32;
    let pass = || {
        let mut s = lo;
        let (mut total, mut max, mut visible) = (Duration::ZERO, Duration::ZERO, 0);
        for _ in 0..FRAMES {
            let t = Instant::now();
            visible = visible.max(frame(&layout, &mut s, dy));
            let took = t.elapsed();
            total += took;
            max = max.max(took);
        }
        (total / FRAMES as u32, max, visible)
    };
    let (cold, _, _) = pass();
    let (warm, warm_max, visible) = pass();
    Run {
        build,
        rows: [layout.old.rows(), layout.new.rows()],
        cold,
        warm,
        warm_max,
        visible,
    }
}

fn us(d: Duration) -> f64 {
    d.as_secs_f64() * 1e6
}

#[test]
#[ignore = "timing; run with --release --ignored --nocapture"]
fn frame_cost_is_flat_in_file_length() {
    // Warm the allocator / caches once.
    run(2_000, 50, true);
    let small = run(2_000, 50, true);
    let large = run(20_000, 500, true);
    let small_open = run(2_000, 50, false);
    let large_open = run(20_000, 500, false);
    for (name, r) in [
        ("2k / 50 hunks, folded", &small),
        ("20k / 500 hunks, folded", &large),
        ("2k / 50 hunks, expanded", &small_open),
        ("20k / 500 hunks, expanded", &large_open),
    ] {
        eprintln!(
            "{name}: layout build {:.2}ms, rows {}+{}, per frame cold {:.2}us warm {:.2}us (max {:.2}us), visible <= {} rows",
            r.build.as_secs_f64() * 1e3,
            r.rows[0],
            r.rows[1],
            us(r.cold),
            us(r.warm),
            us(r.warm_max),
            r.visible,
        );
    }
    for (small, large) in [(&small, &large), (&small_open, &large_open)] {
        let ratio = us(large.warm) / us(small.warm).max(0.01);
        eprintln!("warm per-frame ratio 20k / 2k: {ratio:.2}");
        // Visible-rows-only: 10x the file must not cost anywhere near 10x per
        // frame (a few binary searches grow by log n).
        assert!(
            ratio < 2.5,
            "per-frame cost grew {ratio:.2}x with 10x the file"
        );
        // Far under a 120 fps budget (8.3 ms) for the pure part.
        assert!(large.warm < Duration::from_millis(1));
    }
}

const REWRAP_WIDTH_PX: f32 = 640.;

#[test]
#[ignore = "timing; run with --release --ignored --nocapture"]
fn rewrap_cost_report() {
    let (old, new, alignment, fold) = synthetic_wrap(20_000, 500);
    let (small_old, small_new, small_align, small_fold) = synthetic_wrap(2_000, 50);
    let width = REWRAP_WIDTH_PX;
    // Allocator / icache warm-up.
    {
        let mut cw = fake_char_width;
        let plan = WrapPlan {
            old: WrapSide { width_px: width },
            new: WrapSide { width_px: width },
        };
        let _ = Layout::build(
            old.as_str().into(),
            new.as_str().into(),
            &alignment,
            Some(&fold),
            Some((&plan, &mut cw)),
        );
    }
    let folded = time_layout_rewrap(&old, &new, &alignment, Some(&fold), width);
    let expanded = time_layout_rewrap(&old, &new, &alignment, None, width);
    for (name, r) in [
        ("20k wrap synth, folded", &folded),
        ("20k wrap synth, expanded", &expanded),
    ] {
        eprintln!(
            "{name} @ {width}px: layout wrap-off {:.2}ms, rewrap warm median {:.2}ms (max {:.2}ms), rows {}+{}",
            r.wrap_off.as_secs_f64() * 1e3,
            r.wrap_median.as_secs_f64() * 1e3,
            r.wrap_max.as_secs_f64() * 1e3,
            r.rows[0],
            r.rows[1],
        );
    }
    eprintln!(
        "8ms rewrap target (acceptance 8): folded median {:.2}ms, expanded median {:.2}ms — deferral in 05 if resize must stay smooth",
        folded.wrap_median.as_secs_f64() * 1e3,
        expanded.wrap_median.as_secs_f64() * 1e3,
    );

    let small_off = wrap_off_build(&small_old, &small_new, &small_align, Some(&small_fold));
    let large_off = wrap_off_build(&old, &new, &alignment, Some(&fold));
    let ratio = large_off.as_secs_f64() / small_off.as_secs_f64().max(1e-9);
    eprintln!(
        "wrap-off layout build 2k vs 20k (folded): {:.2}ms vs {:.2}ms, ratio {ratio:.2}",
        small_off.as_secs_f64() * 1e3,
        large_off.as_secs_f64() * 1e3,
    );
    assert!(
        ratio < 12.,
        "wrap-off layout build grew {ratio:.2}x for 10x file (expected ~linear)"
    );
    assert!(
        large_off < Duration::from_millis(5),
        "20k wrap-off layout build {:.2}ms regressed",
        large_off.as_secs_f64() * 1e3,
    );
}
