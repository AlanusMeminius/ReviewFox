# Use crates.io GPUI with a custom diff view

ReviewFox needs a desktop UI and a CLion-like dual-pane diff, and BeadsViewer already shows a viable GPUI app shell on crates.io `gpui`. We will follow that route: depend on crates.io GPUI and build the diff viewer ourselves (Alignment-driven), rather than pulling Zed's `editor`/`diff` stack. Embedding Zed forces a single git-pinned GPUI identity, a large GPL-heavy crate graph, and does not unblock the real hard problem—stable Comparison/Alignment/Anchor semantics. Zed remains a UX reference, not a dependency.
