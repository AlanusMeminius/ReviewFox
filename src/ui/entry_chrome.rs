//! Exclusive Entry chrome decisions (ADR-0011, prototype A).
//!
//! Pure seam: which Entry kind is active, what a kind hit does, empty-MR /
//! open-picker-on-enter, and whether titlebar shows the two-piece GitLab chrome
//! (kind track + value pill) vs the Branch-only pill. Rendering stays in `app_view`.

use crate::workspace_store::MrEntryLabel;

/// Selected Entry kind in the Workspace (exactly one at a time).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    Branch,
    Mr,
    Worktree,
}

/// Titlebar Entry chrome layout when a Workspace is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryChromeMode {
    /// Kind track + value pill. Shown with or without GitLab (ADR-0013).
    KindTrackAndValuePill,
}

/// Result of clicking a kind hit on the kind track.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KindSwitchAction {
    /// Already on that kind.
    Stay,
    /// Select Branch Browser; clear selected MR Entry (keep last-MR memory).
    SelectBranch,
    /// Read the checkout against HEAD. No picker, no last-Worktree memory.
    SelectWorktree,
    /// Restore MR Entry from last-MR memory via the existing restore path.
    /// Does not auto-open the picker.
    RestoreMr(MrEntryLabel),
    /// No last-MR to restore — hold empty MR kind and open the picker.
    EnterEmptyMr,
}

/// What chrome should do when restoring a last-MR label fails.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RestoreFailureAction {
    /// Keep the MR Entry and show the failure in detail (picker select path).
    KeepFailedDetail,
    /// Clear the phantom Entry, hold empty MR kind, open the picker.
    EnterEmptyMrOpenPicker,
}

/// Active kind. Worktree wins; otherwise MR when an Entry is selected or empty-MR chrome is held.
pub fn active_entry_kind(mr_selected: bool, empty_mr: bool, worktree: bool) -> EntryKind {
    if worktree {
        EntryKind::Worktree
    } else if mr_selected || empty_mr {
        EntryKind::Mr
    } else {
        EntryKind::Branch
    }
}

/// Chrome layout. The kind track is always on so Worktree is reachable without GitLab.
pub fn chrome_mode(_gitlab_visible: bool) -> EntryChromeMode {
    EntryChromeMode::KindTrackAndValuePill
}

/// While a picker is open, hide only the value pill (kind track stays usable).
pub fn value_pill_hidden(picker_open: bool) -> bool {
    picker_open
}

/// What the kind switch should do (hard-exclusive; never co-selects Branch+MR).
pub fn kind_switch_action(
    current: EntryKind,
    target: EntryKind,
    last_mr: Option<&MrEntryLabel>,
) -> KindSwitchAction {
    if current == target {
        return KindSwitchAction::Stay;
    }
    match target {
        EntryKind::Branch => KindSwitchAction::SelectBranch,
        EntryKind::Worktree => KindSwitchAction::SelectWorktree,
        EntryKind::Mr => match last_mr {
            Some(label) => KindSwitchAction::RestoreMr(label.clone()),
            None => KindSwitchAction::EnterEmptyMr,
        },
    }
}

/// Failed restore from a kind-switch opens empty MR + picker; picker-select keeps detail.
pub fn restore_failure_action(from_kind_switch: bool) -> RestoreFailureAction {
    if from_kind_switch {
        RestoreFailureAction::EnterEmptyMrOpenPicker
    } else {
        RestoreFailureAction::KeepFailedDetail
    }
}

/// Value-hit label for the active kind.
pub fn value_label(
    kind: EntryKind,
    branch: &str,
    mr_iid: Option<u64>,
    checkout: &str,
) -> String {
    match kind {
        EntryKind::Branch => branch.to_string(),
        EntryKind::Worktree => checkout.to_string(),
        EntryKind::Mr => match mr_iid {
            Some(iid) => format!("!{iid}"),
            None => "Select MR…".into(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn label(project: &str, iid: u64) -> MrEntryLabel {
        MrEntryLabel {
            project: project.into(),
            iid,
        }
    }

    #[test]
    fn active_kind_follows_mr_entry_or_empty_mr_hold() {
        assert_eq!(active_entry_kind(false, false, false), EntryKind::Branch);
        assert_eq!(active_entry_kind(true, false, false), EntryKind::Mr);
        assert_eq!(active_entry_kind(false, true, false), EntryKind::Mr);
        assert_eq!(active_entry_kind(true, true, false), EntryKind::Mr);
        assert_eq!(active_entry_kind(true, true, true), EntryKind::Worktree);
    }

    #[test]
    fn kind_track_shows_with_or_without_gitlab() {
        assert_eq!(chrome_mode(true), EntryChromeMode::KindTrackAndValuePill);
        assert_eq!(chrome_mode(false), EntryChromeMode::KindTrackAndValuePill);
    }

    #[test]
    fn value_pill_hides_only_while_picker_open() {
        assert!(!value_pill_hidden(false));
        assert!(value_pill_hidden(true));
    }

    #[test]
    fn kind_hit_same_kind_stays() {
        assert_eq!(
            kind_switch_action(EntryKind::Branch, EntryKind::Branch, None),
            KindSwitchAction::Stay
        );
        assert_eq!(
            kind_switch_action(EntryKind::Mr, EntryKind::Mr, Some(&label("acme/app", 42))),
            KindSwitchAction::Stay
        );
    }

    #[test]
    fn switching_to_branch_selects_branch_browser() {
        assert_eq!(
            kind_switch_action(
                EntryKind::Mr,
                EntryKind::Branch,
                Some(&label("acme/app", 42))
            ),
            KindSwitchAction::SelectBranch
        );
    }

    #[test]
    fn switching_to_mr_with_last_mr_restores_that_entry() {
        let remembered = label("acme/app", 42);
        assert_eq!(
            kind_switch_action(EntryKind::Branch, EntryKind::Mr, Some(&remembered)),
            KindSwitchAction::RestoreMr(remembered)
        );
    }

    #[test]
    fn switching_to_mr_without_last_mr_enters_empty_mr() {
        assert_eq!(
            kind_switch_action(EntryKind::Branch, EntryKind::Mr, None),
            KindSwitchAction::EnterEmptyMr
        );
    }

    #[test]
    fn switching_to_worktree_does_not_restore_mr() {
        assert_eq!(
            kind_switch_action(
                EntryKind::Branch,
                EntryKind::Worktree,
                Some(&label("acme/app", 42))
            ),
            KindSwitchAction::SelectWorktree
        );
        assert_eq!(
            kind_switch_action(EntryKind::Worktree, EntryKind::Worktree, None),
            KindSwitchAction::Stay
        );
    }

    #[test]
    fn kind_switch_restore_failure_opens_empty_mr_picker() {
        assert_eq!(
            restore_failure_action(true),
            RestoreFailureAction::EnterEmptyMrOpenPicker
        );
        assert_eq!(
            restore_failure_action(false),
            RestoreFailureAction::KeepFailedDetail
        );
    }

    #[test]
    fn value_label_shows_branch_mr_iid_or_select_prompt() {
        assert_eq!(
            value_label(EntryKind::Branch, "feature/mr", None, "main"),
            "feature/mr"
        );
        assert_eq!(
            value_label(EntryKind::Mr, "feature/mr", Some(42), "main"),
            "!42"
        );
        assert_eq!(
            value_label(EntryKind::Mr, "feature/mr", None, "main"),
            "Select MR…"
        );
        assert_eq!(
            value_label(EntryKind::Worktree, "feature/mr", None, "main"),
            "main"
        );
    }
}
