# 02: Find, DraftComment, and overlay colors

**What to build:** Find and DraftComment fields and their floating surfaces stay readable in both Software Themes, even when the Code Theme has the opposite lightness.

**Blocked by:** 01 Sidebar readability and Software Theme color roles.

**Status:** fixed

- [x] Empty Find and DraftComment fields use readable placeholder text; populated fields, selection, caret, and focus state remain clear.
- [x] Floating field surfaces, borders, and shadows use Software Theme roles and meet the spec's contrast targets.
- [x] A light Code Theme in dark chrome and a dark Code Theme in light chrome retain readable field chrome.
