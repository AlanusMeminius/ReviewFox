//! Exclusive Entry chrome decisions (ADR-0011, prototype A3).
//!
//! Pure seam: which Entry kind is active, what a kind hit does, and whether
//! titlebar shows the unified capsule vs the Branch-only pill. Rendering stays
//! in `app_view`; ticket 03 owns empty-MR / open-picker-on-enter.

use crate::workspace_store::MrEntryLabel;

/// Selected Entry kind in the Workspace (exactly one at a time).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    Branch,
    Mr,
}

/// Titlebar Entry chrome layout when a Workspace is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryChromeMode {
    /// No GitLab chrome — Branch Browser pill only.
    BranchPillOnly,
    /// GitLab chrome — unified A3 capsule (kind hits + value hit).
    UnifiedCapsule,
}

/// Result of clicking a kind hit in the unified capsule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KindSwitchAction {
    /// Already on that kind.
    Stay,
    /// Select Branch Browser; clear selected MR Entry (keep last-MR memory).
    SelectBranch,
    /// Restore MR Entry from last-MR memory via the existing restore path.
    RestoreMr(MrEntryLabel),
    /// No last-MR to restore — stay on Branch (ticket 03 opens the picker).
    StayOnBranchNoMemory,
}

/// Active kind from whether an MR Entry is currently selected.
pub fn active_entry_kind(mr_selected: bool) -> EntryKind {
    if mr_selected {
        EntryKind::Mr
    } else {
        EntryKind::Branch
    }
}

/// Chrome layout from whether GitLab host matching applies.
pub fn chrome_mode(gitlab_visible: bool) -> EntryChromeMode {
    if gitlab_visible {
        EntryChromeMode::UnifiedCapsule
    } else {
        EntryChromeMode::BranchPillOnly
    }
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
        EntryKind::Mr => match last_mr {
            Some(label) => KindSwitchAction::RestoreMr(label.clone()),
            None => KindSwitchAction::StayOnBranchNoMemory,
        },
    }
}

/// Value-hit label for the active kind (ticket 03 may replace empty MR copy).
pub fn value_label(kind: EntryKind, branch: &str, mr_iid: Option<u64>) -> String {
    match kind {
        EntryKind::Branch => branch.to_string(),
        EntryKind::Mr => match mr_iid {
            Some(iid) => format!("!{iid}"),
            None => "Merge requests".into(),
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
    fn active_kind_follows_whether_mr_entry_is_selected() {
        assert_eq!(active_entry_kind(false), EntryKind::Branch);
        assert_eq!(active_entry_kind(true), EntryKind::Mr);
    }

    #[test]
    fn gitlab_visible_uses_unified_capsule_otherwise_branch_pill() {
        assert_eq!(chrome_mode(true), EntryChromeMode::UnifiedCapsule);
        assert_eq!(chrome_mode(false), EntryChromeMode::BranchPillOnly);
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
            kind_switch_action(EntryKind::Mr, EntryKind::Branch, Some(&label("acme/app", 42))),
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
    fn switching_to_mr_without_last_mr_stays_on_branch_until_ticket_03() {
        assert_eq!(
            kind_switch_action(EntryKind::Branch, EntryKind::Mr, None),
            KindSwitchAction::StayOnBranchNoMemory
        );
    }

    #[test]
    fn value_label_shows_branch_or_mr_iid() {
        assert_eq!(
            value_label(EntryKind::Branch, "feature/mr", None),
            "feature/mr"
        );
        assert_eq!(
            value_label(EntryKind::Mr, "feature/mr", Some(42)),
            "!42"
        );
    }
}
