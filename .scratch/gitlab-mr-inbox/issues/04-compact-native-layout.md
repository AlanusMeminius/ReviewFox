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
