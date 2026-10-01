# Theme system — implemented design

Status: implemented

This design follows the existing `Software Theme`, `Code Theme`, and `Code Theme Pairing` terms in `CONTEXT.md`. It supersedes the v1 limits in `.scratch/code-theme/spec.md` that assumed a permanently light interface and only one visible Code Theme choice.

## Observed failures

- In dark mode, the Pin and Repositories section labels have low contrast on the translucent sidebar.
- The shared scrollbar uses fixed light thumb colors.
- The Changes tree uses a fixed light hover fill on island rows.
- Find and DraftComment placeholders use fixed translucent black.
- The Code Theme picker always edits the light pairing; the active dark pairing still defaults to One Light, so choosing One Dark in dark mode does not recolor the current Diff.

## Decisions settled, 2026-09-30

1. Keep Software Theme and Code Theme independently selectable. Either Code Theme may be paired with either Software Theme.
2. An absent dark Code Theme choice resolves to One Dark. An explicit stored choice retains its meaning. The existing absent light choice remains One Light.
3. Show both pairing choices in Settings, marked so the user can see which is currently active. Each control edits its corresponding stored choice.
4. Keep ReviewFox's existing deep-gray background and blue accent as the dark visual direction. Use Zed's semantic color-role organization as a reference, rather than copying its visual palette.
5. Software Theme remains a manual light/dark choice for this work. Following the operating system's appearance is a separate future setting.
6. Find, DraftComment input chrome, and scrollbars follow Software Theme, including when the chosen Code Theme has the opposite lightness. Code Theme governs the Diff paper, syntax, and Diff marks.
7. Preserve frosted/acrylic material while giving text-bearing areas enough stable backing for readable foregrounds across wallpapers.
8. Target at least 4.5:1 contrast for ordinary text, including placeholders, sidebar section labels, and code on its actual painted backgrounds. Necessary non-text visual marks (icons, borders, interaction indicators) target at least 3:1. Validate both appearances, interactive states, and translucent surfaces. This raises the existing Code Theme v1 syntax target of 3:1.
9. Adjust existing light and Code Theme colors where the contrast target fails, preserving the overall light appearance where possible. Old hex values are not acceptance criteria when they prevent readability.
10. ChangedPath statuses and Diff change bands share the meaning of addition/deletion/modification, but may use different colors appropriate to text, icons, and broad backgrounds. One Light's neutral deleted band remains a valid visual direction.
11. Theme importing and reading from Zed are a separate feature. This work consolidates the app's color design and use for built-in themes only.

## Reference and current architecture

Zed groups theme colors by purpose (surface, text, element state, scrollbar, editor, syntax) and exposes one active theme to components. See Zed's [theme model](https://github.com/zed-industries/zed/blob/main/crates/theme/src/theme.rs) and [color roles](https://github.com/zed-industries/zed/blob/main/crates/theme/src/styles/colors.rs). ReviewFox currently distributes light/dark branches through `src/ui/theme.rs`, keeps Software Theme state in `src/ui/code_theme.rs`, and has direct colors in component painters. The theme audit must account for opaque and translucent surfaces, state changes, and Code Theme/Software Theme boundaries.

## Color model

The resolved Software Theme owns semantic roles for application surfaces and controls; components request a role instead of branching on light/dark or selecting a literal color. The two built-in Software Theme palettes should have the same role set. Roles are named for what the color does, not for a hue or today's appearance (`surface_elevated`, not `white`; `text_placeholder`, not `faint`). A component can select roles according to its state and surface, but must not synthesize its own light/dark palette.

| Role family | Required distinctions | Existing examples to consolidate |
| --- | --- | --- |
| Surfaces | window material/backing, desk, island/panel, floating overlay, input, tooltip | `frost`, `sidebar`, `white`, `capsule`, `find_bar_bg`, tooltip literal |
| Text and icons | primary, secondary, section label, placeholder, disabled, text on selected/accent surface, icon idle/active | `text`, `muted`, `faint`, fixed black placeholder, `on_sidebar_selected` |
| Controls | idle, hover, pressed, selected, focused, disabled; separate control fill and border where needed | `hover`, `element_active`, sidebar row fills, tree `ROW_HOVER`, window controls |
| Structure | ordinary/subtle/focus borders, splitter, shadows | `line`, `border_variant`, `border_focused`, fixed shadow ink |
| Feedback | success/error/warning/info foreground, background, border; ChangedPath status text/icon | `success`, `error`, literal Git status and error text colors |
| Scrollbars | track if visible, idle/hover/drag thumb | fixed `THUMB_IDLE` and `THUMB_ACTIVE`, used by both overlay and custom Diff painters |

The resolved Code Theme continues to own Diff paper, row bands, code foregrounds, line numbers, hunk and omission marks, in-line word marks, search hits, selection, and comment markers. Derived colors must be safe for each palette; the current One Light-calibrated RGB shifts and comment-pad channel ratios cannot be treated as universal dark-theme rules. An authored role is preferable where a derivation cannot preserve intended contrast or visual meaning. The Diff's custom painter receives one resolved palette; it does not branch on Software Theme for its code colors. Nearby Find/DraftComment controls and scrollbars use Software Theme roles even for opposite-lightness Code Themes.

Each role is evaluated on its *actual* background. Translucent material needs a bounded, stable backing or another explicit guarantee of the contrast target; checking foreground against a nominal hex background is insufficient. Hover, selected, pressed, and focus must remain visually distinct on both desk and island surfaces. No state may rely on color alone when an existing icon, position, or border conveys the same meaning.

## Settings and migration

- The Appearance page shows a Light Code Theme row and a Dark Code Theme row, both populated by all built-in Code Themes and marked with the currently active mode. Each row edits the matching pairing slot; changing the active Code Theme refreshes existing Diff windows without rerunning syntax analysis.
- An absent light choice resolves to One Light; an absent dark choice resolves to One Dark. A stored explicit `one-light` in the dark slot must remain distinct from absence, including across save/load and relaunch. The current settings normalization, which drops `one-light` from both slots, must become mode-specific.
- An unknown saved ID remains stored for forward compatibility but resolves to the relevant mode's default. Returning to a mode's default clears only that slot. A valid choice from the opposite appearance remains allowed and persists.
- The manual Software Theme choice remains app-wide. A change updates every window and any custom painter through the same resolved theme state; Code Theme pairing remains app-wide and independent of Repository and Workspace.

## Audit and acceptance

The implementation audit covers sidebar and Repository/Pin labels; app/Settings shells, pickers, buttons and tooltips; Changes and Diff trees; MR/commit metadata and markdown; Find and DraftComment fields; all overlay and Diff-painted scrollbars; custom Diff lines, search, selection, comments and hunk marks; status/error text; and native window material on macOS/Windows. A raw color in a component needs an explicit justification, such as an asset's own color or a test fixture. Literal colors belong in built-in palette definitions or other explicitly owned assets, not in ordinary component render paths.

Acceptance scenarios include the five reported failures; every Code Theme paired with either Software Theme (including light code in dark chrome and dark code in light chrome); default, explicit, and unknown IDs; light/dark switching while multiple windows are open; hover, selected, focus and disabled states; empty and populated fields; and opaque/translucent backgrounds. Contrast checks use the composed rendered background. Automated checks cover pure palette resolution and measurable contrast; visual review covers material composition, surface hierarchy and state distinction.

## Excluded from this work

Importing or reading external themes, following the operating system's appearance, per-Repository or per-Workspace theme settings, changing the syntax engine, and changing Export content.
