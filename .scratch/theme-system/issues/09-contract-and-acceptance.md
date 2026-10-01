# 09: Retire old color paths and verify theme combinations

**What to build:** The application consistently obtains colors through the resolved Software Theme or Code Theme, and the complete theme matrix is verified before the work is considered done.

**Blocked by:** 02 Find, DraftComment, and overlay colors; 03 Theme overlay and Diff scrollbars; 04 ChangedPath tree interaction and status colors; 06 Readable built-in Code Themes and Diff marks; 07 Settings, pickers, tooltips, and controls; 08 Metadata, Markdown, and feedback colors.

**Status:** fixed

- [x] Unused old color helpers and mode branches are removed after their callers have migrated; remaining component literals have an explicit owned reason.
- [x] The reported sidebar, scrollbar, tree hover, placeholder, and One Dark failures are verified fixed.
- [x] All built-in Code Themes are checked with both Software Themes, including open-window updates and default, explicit, and unknown pairing IDs.
- [x] Interaction states and composed opaque/translucent backgrounds meet the agreed contrast and visual distinction targets; the full test suite passes.

## Acceptance evidence (implementation review)

- Sidebar Repository/Pin labels and selected rows: `ui::theme::software_palette_tests::sidebar_text_survives_light_and_dark_wallpapers_and_row_states` composes the window and sidebar backings over light/dark wallpapers; row indicators are checked against each fill.
- Overlay and Diff scrollbars: `scrollbar_track_and_thumb_stay_visible_on_every_builtin_diff_band` checks both appearances against every built-in Code Theme band. Find and DraftComment placeholders: `find_and_draft_fields_remain_readable_over_either_code_paper` checks the composed overlay/field backgrounds. Changes and Diff ChangedPath rows: `changed_path_tree_roles_are_readable_on_island_and_composed_desk` checks idle, hover, pressed, and selected states.
- Code Theme pairing: `every_code_theme_pairs_with_both_software_appearances` covers all three built-ins in both Software Themes; `code_theme::tests` and `appearance::tests` cover absent, explicit opposite-lightness, unknown, storage, and normalization cases. `appearance::update` remembers the new mode/pairing before publishing `Appearance`; its global observer refreshes every window. DiffView observes `Appearance`, and the Diff pane resolves the active Code Theme on each paint, invalidating cached glyph shapes when the ID changes without rerunning syntax analysis.
- Component color scan: render paths contain no raw `rgb`/`rgba`/`hsla` literals. The remaining literals in `diff/element.rs` are test fixtures. Component `Rgba` struct updates derive only opacity from semantic roles for unpainted idle borders or interaction wash.
- Text directly over window material is bounded by the Software Theme's `chrome_backing`: the AppView comparison label, Diff export feedback, and Diff status band. `chrome_roles_remain_readable_over_material_and_controls` composes root and chrome backing over black/white wallpapers and checks ordinary titlebar/status text at 4.5:1. Other root-stage text is inside the opaque Commit, Changes, Diff, or Settings islands or the bounded sidebar/tree backing.
