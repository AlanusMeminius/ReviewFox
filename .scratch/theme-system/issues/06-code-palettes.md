# 06: Readable built-in Code Themes and Diff marks

**What to build:** Diff code, status bands, search, selection, and comment marks are readable with each built-in Code Theme, regardless of the Software Theme paired with it.

**Blocked by:** 05 Code Theme Pairing and One Dark default.

**Status:** fixed

- [x] One Light, Atom One Light, and One Dark meet the spec's text contrast target on their actual paper, change bands, word marks, and search fills.
- [x] Line numbers, comment marks, hunk/omission marks, selection, and focus cues remain discernible in each built-in palette.
- [x] Derived Diff colors are safe for dark as well as light palettes; authored roles replace derivations that cannot preserve the intended meaning or contrast.
- [x] The Code Theme governs Diff code and marks; surrounding fields and scrollbars continue to follow Software Theme.
