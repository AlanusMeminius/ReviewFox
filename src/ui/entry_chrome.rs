//! Exclusive Entry chrome decisions (ADR-0011, prototype A; open restore ADR-0012).
//!
//! Pure seam: which Entry kind is active, what a kind hit does, empty-MR /
//! open-picker-on-enter, and what opening a Workspace does with its stored `mr`
//! label. The kind track is always shown; the value pill hides only while a
//! picker is open. Rendering stays in `app_view`.

use crate::workspace_store::MrEntryLabel;

/// Selected Entry kind in the Workspace (exactly one at a time).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    Branch,
    Mr,
    Uncommitted,
}

/// Result of clicking a kind hit on the kind track.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KindSwitchAction {
    /// Already on that kind.
    Stay,
    /// Select Branch Browser; clear selected MR Entry (keep last-MR memory).
    SelectBranch,
    /// Read the checkout against HEAD. No picker, no last-Uncommitted memory.
    SelectUncommitted,
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

/// Active kind. Uncommitted wins; otherwise MR when an Entry is selected or empty-MR chrome is held.
pub fn active_entry_kind(mr_selected: bool, empty_mr: bool, uncommitted: bool) -> EntryKind {
    if uncommitted {
        EntryKind::Uncommitted
    } else if mr_selected || empty_mr {
        EntryKind::Mr
    } else {
        EntryKind::Branch
    }
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
        EntryKind::Uncommitted => KindSwitchAction::SelectUncommitted,
        EntryKind::Mr => match last_mr {
            Some(label) => KindSwitchAction::RestoreMr(label.clone()),
            None => KindSwitchAction::EnterEmptyMr,
        },
    }
}

/// What opening a Workspace does with its stored selected Entry (ADR-0012).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenWorkspace {
    pub entry: OpenEntry,
    /// Refetch failure for this open. Not the kind-switch empty-MR picker.
    pub refetch_failure: RestoreFailureAction,
}

/// Selected Entry when a Workspace is opened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OpenEntry {
    /// Activate this MR Entry on the existing MR path (refetch `diff_refs`).
    ActivateMr(MrEntryLabel),
    /// Display Branch Browser. A stored `mr` label stays on the Workspace.
    ShowBranchKeepLabel,
}

/// Given the stored `mr` label and whether GitLab chrome is visible.
/// Absent `mr` is Branch Browser. Hidden chrome shows Branch and keeps the label.
pub fn opening_workspace(mr: Option<&MrEntryLabel>, gitlab_chrome_visible: bool) -> OpenWorkspace {
    let entry = match (mr, gitlab_chrome_visible) {
        (Some(label), true) => OpenEntry::ActivateMr(label.clone()),
        _ => OpenEntry::ShowBranchKeepLabel,
    };
    OpenWorkspace {
        entry,
        refetch_failure: RestoreFailureAction::KeepFailedDetail,
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
pub fn value_label(kind: EntryKind, branch: &str, mr_iid: Option<u64>, checkout: &str) -> String {
    match kind {
        EntryKind::Branch => branch.to_string(),
        EntryKind::Uncommitted => checkout.to_string(),
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
        assert_eq!(active_entry_kind(true, true, true), EntryKind::Uncommitted);
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
    fn switching_to_uncommitted_does_not_restore_mr() {
        assert_eq!(
            kind_switch_action(
                EntryKind::Branch,
                EntryKind::Uncommitted,
                Some(&label("acme/app", 42))
            ),
            KindSwitchAction::SelectUncommitted
        );
        assert_eq!(
            kind_switch_action(EntryKind::Uncommitted, EntryKind::Uncommitted, None),
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
    fn opening_without_mr_shows_branch_browser() {
        assert_eq!(
            opening_workspace(None, true).entry,
            OpenEntry::ShowBranchKeepLabel
        );
        assert_eq!(
            opening_workspace(None, false).entry,
            OpenEntry::ShowBranchKeepLabel
        );
        assert_eq!(
            opening_workspace(None, true).refetch_failure,
            RestoreFailureAction::KeepFailedDetail
        );
    }

    #[test]
    fn opening_with_mr_but_no_gitlab_chrome_shows_branch_and_keeps_label() {
        let stored = label("acme/app", 42);
        let open = opening_workspace(Some(&stored), false);
        assert_eq!(open.entry, OpenEntry::ShowBranchKeepLabel);
        assert_eq!(open.refetch_failure, RestoreFailureAction::KeepFailedDetail);
    }

    #[test]
    fn opening_with_mr_and_gitlab_chrome_activates_that_entry() {
        let stored = label("acme/app", 42);
        let open = opening_workspace(Some(&stored), true);
        assert_eq!(open.entry, OpenEntry::ActivateMr(stored));
        assert_eq!(open.refetch_failure, RestoreFailureAction::KeepFailedDetail);
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
            value_label(EntryKind::Uncommitted, "feature/mr", None, "main"),
            "main"
        );
    }
}
