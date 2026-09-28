//! Soft-wrap inputs and row expansion helpers for [`super::layout::Layout::build`].
//! See `docs/diffview-architecture.md` §6 and `docs/dual-pane-diff.md` §3.1.1.

use std::collections::HashMap;
use std::ops::Range;

use super::layout::{LineKind, LinePart, SideLayout};
use super::tabs::TabExpansion;
use super::wrap::{WrapBreaks, wrap_display_line};

/// Per-side wrap width in px (code column).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WrapSide {
    pub width_px: f32,
}

/// Inputs for one wrap pass; cached per (Layout, width, font) in the pane (05).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WrapPlan {
    pub old: WrapSide,
    pub new: WrapSide,
}

/// Stored break data when [`super::layout::Layout::build`] is given a wrap plan.
#[derive(Clone, Debug, PartialEq)]
pub struct AppliedWrap {
    pub plan: WrapPlan,
    pub old_breaks: HashMap<u32, WrapBreaks>,
    pub new_breaks: HashMap<u32, WrapBreaks>,
}

impl AppliedWrap {
    #[allow(dead_code)] // 05 shaped rows
    pub fn breaks(&self, side: crate::domain::Side, ln: u32) -> Option<&WrapBreaks> {
        match side {
            crate::domain::Side::Old => self.old_breaks.get(&ln),
            crate::domain::Side::New => self.new_breaks.get(&ln),
        }
    }
}

pub(crate) struct WrapCtx<'a> {
    pub plan: WrapPlan,
    pub old_w: f32,
    pub new_w: f32,
    pub cw: &'a mut dyn FnMut(char) -> f32,
    pub old_breaks: HashMap<u32, WrapBreaks>,
    pub new_breaks: HashMap<u32, WrapBreaks>,
}

fn visual_rows_for_line(
    side_text: &str,
    bytes: &Range<usize>,
    width: f32,
    ln: u32,
    store: &mut HashMap<u32, WrapBreaks>,
    cw: &mut dyn FnMut(char) -> f32,
) -> u32 {
    let line = &side_text[bytes.clone()];
    let display = TabExpansion::new(line);
    let breaks = wrap_display_line(&display.text, width, cw);
    let n = 1 + breaks.breaks.len() as u32;
    store.insert(ln, breaks);
    n
}

fn push_wrapped_rows(
    side: &mut SideLayout,
    ln: u32,
    kind: LineKind,
    bytes: Range<usize>,
    block: Option<u32>,
    content_rows: u32,
    pad_rows: u32,
) {
    for part_ix in 0..content_rows {
        let part = if part_ix == 0 {
            LinePart::First
        } else {
            LinePart::Continuation
        };
        side.push_visual_line(ln, kind, part, bytes.clone(), block);
    }
    for _ in 0..pad_rows {
        side.push_visual_line(ln, kind, LinePart::EqualPad, bytes.clone(), block);
    }
}

impl<'a> WrapCtx<'a> {
    pub(crate) fn push_line_rows(
        &mut self,
        side: &mut SideLayout,
        side_text: &str,
        bytes: Range<usize>,
        ln: u32,
        kind: LineKind,
        block: Option<u32>,
        which: crate::domain::Side,
    ) -> u32 {
        let (width, store) = match which {
            crate::domain::Side::Old => (self.old_w, &mut self.old_breaks),
            crate::domain::Side::New => (self.new_w, &mut self.new_breaks),
        };
        let n = visual_rows_for_line(side_text, &bytes, width, ln, store, self.cw);
        push_wrapped_rows(side, ln, kind, bytes, block, n, 0);
        n
    }

    /// One Equal pair: `max(old_n, new_n)` rows on both sides; shorter side padded.
    pub fn push_equal_pair(
        &mut self,
        old: &mut SideLayout,
        new: &mut SideLayout,
        old_text: &str,
        new_text: &str,
        old_bytes: Range<usize>,
        new_bytes: Range<usize>,
        o_ln: u32,
        n_ln: u32,
        kind: LineKind,
    ) -> u32 {
        let old_n = visual_rows_for_line(
            old_text,
            &old_bytes,
            self.old_w,
            o_ln,
            &mut self.old_breaks,
            self.cw,
        );
        let new_n = visual_rows_for_line(
            new_text,
            &new_bytes,
            self.new_w,
            n_ln,
            &mut self.new_breaks,
            self.cw,
        );
        let pair = old_n.max(new_n);
        push_wrapped_rows(old, o_ln, kind, old_bytes.clone(), None, old_n, pair - old_n);
        push_wrapped_rows(new, n_ln, kind, new_bytes.clone(), None, new_n, pair - new_n);
        pair
    }
}
