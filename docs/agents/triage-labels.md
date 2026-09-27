# Triage Labels

The skills speak in terms of five canonical triage roles. This file maps those roles to the actual label strings used in this repo's issue tracker.

| Label in mattpocock/skills | Label in our tracker | Meaning                                  |
| -------------------------- | -------------------- | ---------------------------------------- |
| `needs-triage`             | `needs-triage`       | Maintainer needs to evaluate this issue  |
| `needs-info`               | `needs-info`         | Waiting on reporter for more information |
| `ready-for-agent`          | `ready-for-agent`    | Fully specified, ready for an AFK agent  |
| `ready-for-human`          | `ready-for-human`    | Requires human implementation            |
| `wontfix`                  | `wontfix`            | Will not be actioned                     |
| —                          | `fixed`              | Fixed; verified, nothing left to action  |

`fixed` is repo-local: it has no counterpart among the skills' five roles,
which all describe issues that still need someone. A diagnosed-and-patched
issue is neither open nor abandoned, so it needs a state of its own. Apply it
once the fix is verified. (`resolved` is not a substitute — that status belongs
to `/wayfinder` map child tickets, not to issues.)

When a skill mentions a role (e.g. "apply the AFK-ready triage label"), use the corresponding label string from this table.

Edit the right-hand column to match whatever vocabulary you actually use.
