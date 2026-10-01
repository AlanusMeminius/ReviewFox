# 01: Sidebar readability and Software Theme color roles

**What to build:** Pin and Repositories headings, Repository rows, and the surrounding desk remain readable in light and dark appearances, including over translucent window material. Introduce an additive semantic Software Theme palette through this visible sidebar slice so later screens can migrate while the app remains usable.

**Blocked by:** None (can start immediately).

**Status:** fixed

- [x] Pin and Repositories labels meet the spec's ordinary-text contrast target over the composed backing on supported platforms.
- [x] Sidebar idle, hover, selected, and pressed states remain visually distinct and readable in both appearances.
- [x] The sidebar reads semantic surface, text, and control-state roles from the active Software Theme; changing appearance updates open windows.
- [x] Light appearance retains its existing visual direction except where legibility requires an adjustment.
