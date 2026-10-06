# Cross-repository MR inbox — selected design

Status: fixed
Type: prototype

## Question and verdict

User chose A, the chronological time inbox. Keep Repository multiselect and recent update period in the titlebar, a central MR list island, and a right detail island. Single click previews; double click enters the current MR interface.

## Primary source

Throwaway branch: `prototype/gitlab-mr-inbox`, commit `3acad5e`.
Artifact: `prototype/gitlab-mr-inbox.html?variant=A`.
Alternatives B (compact table) and C (Repository grouping) remain on that branch.

## Implementation

Native rewrite on `feature/gitlab-mr-inbox`; ADR-0019 records the validated decision. No HTML or prototype controls are promoted to production.
