# Dual-axis splitter: Beads shell, Zed row semantics

Pane resize uses a small dual-axis `splitter` (BeadsViewer `canvas` + `ResizeState` / `ResizeHandler`, 5px hit, session memory only, no double-click reset). Leading vertical rules resize from pointer `x`; trailing vertical rules from `viewport.width − x`; horizontal rules resize south-pane height via `cursor_row_resize` and `viewport.height − y` (Zed dock idea, window-local coords — never `window.bounds()`, which is screen-global). v1 wires all three main-window splits: sidebar|commits, commits|Changes, and Changes file-tree|Head-meta.
