# DualPaneElement: visible-only paint with single scroll source

Status: ready-for-agent
Blocked by: 02

## What

- One custom GPUI `Element` for old pane | gutter | new pane.
- `prepaint`: take bounds → `view_h` → Viewport in the same frame (drop the canvas measure + extra notify).
- `paint`: visible rows only; line numbers, row backgrounds, word marks, bridges, gap hatch, omit waves, seams; tops snapped to device pixels.
- Shaped-line cache keyed `(side, visual row)`, keep visible ± one screen, clear on Layout rebuild / font size change.
- Remove native `ScrollHandle` on the panes; wheel / trackpad / momentum write `scroll_s` only.
- Paint scrollbars: old outer left, new outer right; thumb drag maps that side back to `scroll_s`. Styling from `scrollbar.rs` constants.
- Mouse events go through Viewport `hit` / `bridge_at`; `Decorations` (drafting line, commented lines, search hits, hovered bridge) passed separately.

## Done when

All `docs/dual-pane-diff.md` §3 behaviors verified by running the app; large-file scroll is smooth.
