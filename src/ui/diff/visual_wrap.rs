//! Apply soft-wrap visual row expansion to a [`super::layout::Layout`]. Pure; see
//! `docs/diffview-architecture.md` §6 and `docs/dual-pane-diff.md` §3.1.1.

use std::collections::HashMap;
use std::ops::Range;

use super::layout::{LineKind, LinePart, LineRow, SideLayout};
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

/// Stored break data after [`super::layout::Layout::apply_wrap`].
#[derive(Clone, Debug, PartialEq)]
pub struct AppliedWrap {
    pub plan: WrapPlan,
    pub old_breaks: HashMap<u32, WrapBreaks>,
    pub new_breaks: HashMap<u32, WrapBreaks>,
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
        for part_ix in 0..n {
            let part = if part_ix == 0 {
                LinePart::First
            } else {
                LinePart::Continuation
            };
            side.push_visual_line(ln, kind, part, bytes.clone(), block);
        }
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

        for part_ix in 0..old_n {
            let part = if part_ix == 0 {
                LinePart::First
            } else {
                LinePart::Continuation
            };
            old.push_visual_line(o_ln, kind, part, old_bytes.clone(), None);
        }
        for _ in old_n..pair {
            old.push_visual_line(o_ln, kind, LinePart::EqualPad, old_bytes.clone(), None);
        }

        for part_ix in 0..new_n {
            let part = if part_ix == 0 {
                LinePart::First
            } else {
                LinePart::Continuation
            };
            new.push_visual_line(n_ln, kind, part, new_bytes.clone(), None);
        }
        for _ in new_n..pair {
            new.push_visual_line(n_ln, kind, LinePart::EqualPad, new_bytes.clone(), None);
        }
        pair
    }
}

impl LineRow {
    /// Line number column: first visual row of a logical line only (§3.1.1).
    #[allow(dead_code)] // 05 line numbers
    pub fn shows_line_number(&self) -> bool {
        self.part == LinePart::First
    }

    /// Equal padding beside a longer wrapped partner: blank, no hatch (§3.1.1).
    #[allow(dead_code)] // 05 paint
    pub fn is_equal_padding(&self) -> bool {
        self.part == LinePart::EqualPad
    }
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
