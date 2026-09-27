# Per-pane horizontal scroll

Status: ready-for-agent
Blocked by: 03

## What

Each side has its own `x_offset`, bounded by its longest visible-line width. Only horizontal input over that pane (shift+wheel, trackpad X) moves it; sides are not coupled. Line-number columns and gutter do not move horizontally.

## Done when

Long lines are reachable on either side independently; vertical sync unaffected.
