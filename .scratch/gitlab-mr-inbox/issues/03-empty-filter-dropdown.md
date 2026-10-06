# MR Inbox filter dropdowns render empty

Status: fixed

## Root cause

The fixed-size dropdown container was a block layout, but `scrollbar::overlay_flex` requires a flex parent to allocate its height. Its absolutely positioned scroll content therefore received a 252×0 px viewport and every option was clipped.

## Reproduction and fix

`cargo test --quiet inbox_filter_menus_have_visible_options` rendered the real dropdown through GPUI and failed with a zero-height scroll viewport. Adding `.flex().flex_col()` to the dropdown container makes both menus render their options. The regression checks the first and last options and the distinct option counts after switching menus, without loading credentials or Workspace state.

## Verification

- `cargo test --quiet inbox`: 7 passed.
- `cargo build --quiet`: passed.
- Inspected both real native dropdowns with isolated fixture data: five periods and all Repository options are visible.
