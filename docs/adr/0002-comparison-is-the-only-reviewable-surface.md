# Comparison is the only reviewable surface

DraftComments attach only to a Comparison `(repository, base_oid, head_oid)`. Choosing one commit or a contiguous commit chain is UI that folds into that pair (`C^..C` or `C1^..Cn`); changing base explicitly creates a new Comparison and does not silently migrate comments. Branch names and GitLab MRs are entry labels only and are out of v1. This keeps Alignment, Anchor rebinding, and Export on one identity instead of splitting "per-commit review" and "range review" into two comment spaces.
