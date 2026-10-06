# Compact MR inbox and shared details

Status: fixed

## Confirmed design

User confirmed row order: status, Repository, IID, author on the first line, update time at the right; a single-line MR title on the second line. Use commit-row margins, rounded selection and colors, without separators. Selected dropdown values use dots, and dropdown outer edges align with capsule outer edges.

Both MR surfaces share the 14px semibold title, metadata and selectable Markdown component. Inbox metadata omits identity already present in the list, retaining branches and available merge/pipeline/approval context. The opened MR keeps necessary identity fields. Missing checks are not manufactured, and approved=true remains a fallback when an approval label is absent.

The two inbox islands use the existing splitter with independent session-only width, a half-and-half default, and minimum widths clamped to available space. No prototype code or fixture data is shipped.

## Validation

- GPUI dropdown regression covers nonzero content, first/last options and capsule-to-menu outer-edge alignment for both menus.
- GPUI mouse dispatch test drags the real splitter by 50px, checks the resulting pane width and verifies the original Changes width remains unchanged.
- Geometry coverage checks extreme drags and window shrinking.
- Native fixture screenshots inspected for compact rows and shared detail layout. Dropdown placement is checked by the real GPUI rendering regression.
- `cargo test --quiet`: 515 passed, 4 previously ignored; `cargo build --quiet` passed.

## Comments

- Follow-up: “更早” groups updates before yesterday within the selected time window. Keep that grouping unchanged.
- Move Merge, Pipeline and Approvals below branch metadata in the shared renderer. Reserve the same 24px row for loading and completed checks; allow horizontal scrolling in narrow panes rather than wrapping and shifting the description. Error notices can still expand to show their full message.
- Actual GPUI layout regression verifies the body keeps its vertical position when checks complete at both 600px and 216px widths. Full suite: 516 passed, 4 ignored. Build and diff checks passed.
- Subsequent user decision supersedes keeping date groups: remove 今天 / 昨天 / 更早 headers. Keep the titlebar time filter, descending update order and each MR timestamp. Adjust keyboard scroll indices to account only for preceding error rows, since group headers no longer exist. Inbox tests: 5 passed; build and diff checks passed.
- Repository filter bulk actions are now two buttons at the upper left, outside the scrolling options. Repository option indices and keyboard navigation no longer include bulk actions. Both filters use 4px row gaps and 12px horizontal padding, matching the MR picker's spacing and selected hover/pressed colors. Existing GPUI coverage now checks the button placement, row gaps, visibility, and alignment. Inbox tests: 5 passed; build and diff checks passed.
- Follow-up polish: bulk buttons use the theme's light neutral gray background. Remove the repository hint line and its reserved height, updating keyboard scroll offsets and existing layout checks accordingly. Inbox tests: 5 passed; build and diff checks passed.
