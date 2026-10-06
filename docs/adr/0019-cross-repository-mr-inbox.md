# Cross-repository MR discovery uses the time inbox

The chosen prototype is A (time inbox): a top-level Merge requests destination above Pin and Repositories, with Repository multiselect and an updated-time range in the titlebar, a chronological list island and a detail island. This extends ADR-0006's single-project discovery scope to all registered Repositories matching the configured GitLab host, including pinned Workspaces. All MR states are discoverable; the default is all matching Repositories and the last week. Time ranges use local civil days, including today; one month starts on the previous month's corresponding date, clamped at month-end.

Selecting a row reads a discovery preview without changing the selected Workspace or Comparison. Double-click or Enter opens that Repository's MR Entry through the existing resolver (ADR-0015); only a successful resolution installs the forge's `diff_refs` Comparison. The preview is not an MR Entry context or Publication target. Returning through the sidebar keeps the in-memory filters and selection. Results are not persisted as forge snapshots.

Design source: `prototype/gitlab-mr-inbox` branch, commit `3acad5e`, `prototype/gitlab-mr-inbox.html?variant=A`. B and C remain on that throwaway branch; no variant switcher is part of the application.

API contract: GitLab's [project MR listing](https://docs.gitlab.com/api/merge_requests/#list-project-merge-requests) supports update-time filtering and all states; [REST pagination](https://docs.gitlab.com/api/rest/#pagination) supplies the next page. Discovery reads all matching pages before treating a Repository as loaded.

The inbox list uses compact commit-style rounded selection: status, Repository, IID and author share the first line with update time; the second line is the single-line MR title. Inbox and opened MR details share one content renderer, with identity metadata omitted from the inbox detail because it is already in the list. The list/detail splitter follows ADR-0004 with independent session-only width and a 1:1 initial layout.
