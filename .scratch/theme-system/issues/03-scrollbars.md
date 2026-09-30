# 03: Theme overlay and Diff scrollbars

**What to build:** Every scrollbar follows the active Software Theme, including the custom-painted Diff scrollbars.

**Blocked by:** 01 Sidebar readability and Software Theme color roles.

**Status:** fixed

- [x] Overlay and Diff scrollbar thumbs use the same semantic idle, hover, and drag roles, with visible states in both appearances.
- [x] Dark appearance has no leftover light scrollbar thumb; scrollbars remain visible over both light and dark Code Theme paper.
- [x] Appearance switching updates visible scrollbars without reopening their windows.
