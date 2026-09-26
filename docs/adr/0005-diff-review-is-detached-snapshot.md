# Diff Review is a detached snapshot

Opening Diff binds a Review to the Comparison captured at that moment. Changing the main window’s Comparison via the Branch Browser (or any future entry) does not refresh Diff or migrate DraftComments; the user re-opens Diff explicitly when they want to review a new Comparison. Live-syncing would surprise comment authors and conflate entry navigation with Review identity—rejected in favor of a user-managed snapshot (see also ADR-0002, ADR-0003).
