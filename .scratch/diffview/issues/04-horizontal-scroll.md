# Per-pane horizontal scroll

Status: resolved
Blocked by: 03

## What

Each side has its own `x_offset`, bounded by its longest visible-line width. Only horizontal input over that pane (shift+wheel, trackpad X) moves it; sides are not coupled. Line-number columns and gutter do not move horizontally.

## Done when

Long lines are reachable on either side independently; vertical sync unaffected.

## Comments

**2026-09-27 — resolved (agent).**

What changed:

- `DualPane` has `x_offsets: [f32; 2]` (old, new), one per side. The sides are independent of each other and of `scroll_s`. `scroll_x_by(side, dx)` clamps to `0..=max_x[side]` and notifies only when the value changes. Nothing else writes the offsets: jumps, thumb drag and vertical wheel leave them alone.
- Input routing is `viewport::route_wheel(dx, dy, shift) -> (h, v)` (pure). Each event moves one axis only. With shift and no X delta, the vertical delta becomes horizontal (a plain mouse wheel; wheel down scrolls right). Otherwise the axis with the larger magnitude wins and the other is dropped, so a mostly-horizontal trackpad swipe does not touch `scroll_s`. A tie goes to vertical. GPUI on Windows already sends shift+wheel and tilt wheel as X, so those take the dominant-axis path. Horizontal travel applies to the code pane under the pointer (`frame.code[side]` hitbox). Over the gutter it is consumed and does nothing.
- Bound: `max_x = max(0, text_extent(longest) − pane width)`. `text_extent` is text inset + comment-bar room + line + right inset. `longest` covers the **whole side**, not just the visible rows: `SideLayout::max_chars()` (lazy `OnceCell`, one pass over the shown lines, folded lines excluded) × the mono advance of `'0'` (cached per font px). That value is raised by `widest_seen`, the widest shaped line seen since the last Layout rebuild or font change, and `widest_seen` only grows. So the bound does not jump while scrolling vertically. Wide glyphs (CJK) or tabs can make the estimate too small until the row is shaped, and then the bound only grows.
- Prepaint re-clamps both offsets every frame, which covers resize, fold / expand, ignore-whitespace and font changes. It hands the frame the offsets snapped to device pixels. `open` resets both offsets to 0. `set_font_size` scales the offsets by the font ratio, so the same columns stay in view, and then the clamp applies.
- Paint (`Frame::paint_code`): only the code text and word marks move (`text_x = x0 + TEXT_PAD − x_offset`). Everything else stays put: row / drafting backgrounds (full pane width), the comment bar, gap hatch, seams, empty-side seam, gutter, line numbers, bridges, omit waves and scrollbars. Text is clipped by the existing per-pane content mask, so it slides under the pane edge and never reaches the gutter.
- `docs/diffview-architecture.md` §4 records the axis rule and the bound.

Deviations:

- `Viewport` does not take `x_offset`. The horizontal state is a small pure trio (`route_wheel`, `max_x`, `clamp_x`) in viewport.rs, and the offset is applied at paint. Hit testing is y-only, so Viewport does not need the offset.
- The comment bar is a row marker and does not scroll. It is now painted after the text so it sits on top of text that has scrolled under it.
- No horizontal scrollbar (not required). Horizontal input does not reveal the vertical bars.
- The ticket said "longest visible-line width". The implementation uses the whole side's longest line, as the plan asked, so the bound is stable.

Notes for 05:

- Per frame, the horizontal cost is one `max` per visible row plus two clamps. `max_chars` runs once per Layout. The frame trace could log `x_offsets` / `max_x` if that is useful.
- Nothing from this issue is dead code.

Verification: `cargo build` shows no new warnings (17, all pre-existing). `cargo test`: 120 passed, 2 failed (`ui::splitter::tests::{sidebar_clamp_keeps_commits_strip, diff_tree_clamp_keeps_dual_pane}`, pre-existing). New tests: `route_wheel` axis table, `max_x`, `clamp_x`, `SideLayout::max_chars`. `cargo run` starts and stays up. No visual check: a human needs to check that long lines are reachable on both sides independently, shift+wheel / trackpad direction, clipping at the gutter edge, and that vertical sync is unaffected.
