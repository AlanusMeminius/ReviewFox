# 05: Code Theme Pairing and One Dark default

**What to build:** Reviewers can choose separate Code Themes for the light and dark Software Themes, and a fresh dark appearance shows One Dark by default.

**Blocked by:** None (can start immediately).

**Status:** fixed

- [x] Appearance shows both Code Theme pairing rows, identifies the active mode, and each row edits only its own choice.
- [x] Unset light and dark choices resolve to One Light and One Dark respectively; unknown IDs retain their stored value but resolve to the matching mode's default.
- [x] An explicit One Light choice for dark survives save/load and relaunch; returning either mode to its own default clears only that choice.
- [x] All built-in Code Themes remain selectable in either row; changing the active choice updates existing Diff windows without re-highlighting.
