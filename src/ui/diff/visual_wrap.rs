//! Soft-wrap inputs and row expansion helpers for [`super::layout::Layout::build`].
//! See `docs/diffview-architecture.md` §6 and `docs/dual-pane-diff.md` §3.1.1.

use std::borrow::Cow;
use std::collections::HashMap;
use std::ops::Range;

use super::layout::{LineKind, LinePart, SideLayout};
use super::tabs::TabExpansion;
use super::wrap::{WrapBreaks, wrap_breaks_for_line};

/// Per-side wrap width in px (code column).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WrapSide {
    pub width_px: f32,
}

/// Inputs for one wrap pass; cached per (Layout, width, font) in the pane (05).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WrapPlan {
    pub preimage: WrapSide,
    pub postimage: WrapSide,
}

/// Stored break data when [`super::layout::Layout::build`] is given a wrap plan.
#[derive(Clone, Debug, PartialEq)]
pub struct AppliedWrap {
    pub plan: WrapPlan,
    pub preimage_breaks: HashMap<u32, WrapBreaks>,
    pub postimage_breaks: HashMap<u32, WrapBreaks>,
}

impl AppliedWrap {
    pub fn breaks(&self, side: crate::domain::Side, ln: u32) -> Option<&WrapBreaks> {
        match side {
            crate::domain::Side::Preimage => self.preimage_breaks.get(&ln),
            crate::domain::Side::Postimage => self.postimage_breaks.get(&ln),
        }
    }
}

pub(crate) struct WrapCtx<'a> {
    pub plan: WrapPlan,
    pub preimage_w: f32,
    pub postimage_w: f32,
    pub cw: &'a mut dyn FnMut(char) -> f32,
    pub preimage_breaks: HashMap<u32, WrapBreaks>,
    pub postimage_breaks: HashMap<u32, WrapBreaks>,
}

fn display_line<'a>(line: &'a str) -> Cow<'a, str> {
    if line.contains('\t') {
        Cow::Owned(TabExpansion::new(line).text)
    } else {
        Cow::Borrowed(line)
    }
}

fn store_breaks_if_needed(store: &mut HashMap<u32, WrapBreaks>, ln: u32, breaks: &WrapBreaks) {
    if !breaks.breaks.is_empty() || breaks.continuation_indent_px != 0. {
        store.insert(ln, breaks.clone());
    }
}

fn visual_rows_from_breaks(breaks: &WrapBreaks) -> u32 {
    1 + breaks.breaks.len() as u32
}

/// Compute breaks once (or reuse `precalc`), store when paint needs them, return row count.
fn line_visual_rows(
    side_text: &str,
    bytes: &Range<usize>,
    width: f32,
    ln: u32,
    store: &mut HashMap<u32, WrapBreaks>,
    cw: &mut dyn FnMut(char) -> f32,
    precalc: Option<&WrapBreaks>,
) -> u32 {
    let breaks = match precalc {
        Some(b) => b.clone(),
        None => {
            let line = &side_text[bytes.clone()];
            let display = display_line(line);
            wrap_breaks_for_line(display.as_ref(), width, cw)
        }
    };
    store_breaks_if_needed(store, ln, &breaks);
    visual_rows_from_breaks(&breaks)
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
            crate::domain::Side::Preimage => (self.preimage_w, &mut self.preimage_breaks),
            crate::domain::Side::Postimage => (self.postimage_w, &mut self.postimage_breaks),
        };
        let n = line_visual_rows(side_text, &bytes, width, ln, store, self.cw, None);
        push_wrapped_rows(side, ln, kind, bytes, block, n, 0);
        n
    }

    /// One Equal pair: `max(preimage_n, postimage_n)` rows on both sides; shorter side padded.
    pub fn push_equal_pair(
        &mut self,
        preimage: &mut SideLayout,
        postimage: &mut SideLayout,
        preimage_text: &str,
        postimage_text: &str,
        preimage_bytes: Range<usize>,
        postimage_bytes: Range<usize>,
        o_ln: u32,
        n_ln: u32,
        kind: LineKind,
    ) -> u32 {
        let shared_breaks = if self.preimage_w == self.postimage_w
            && preimage_text[preimage_bytes.clone()] == postimage_text[postimage_bytes.clone()]
        {
            let display = display_line(&preimage_text[preimage_bytes.clone()]);
            Some(wrap_breaks_for_line(display.as_ref(), self.preimage_w, self.cw))
        } else {
            None
        };
        let preimage_n = line_visual_rows(
            preimage_text,
            &preimage_bytes,
            self.preimage_w,
            o_ln,
            &mut self.preimage_breaks,
            self.cw,
            shared_breaks.as_ref(),
        );
        let postimage_n = line_visual_rows(
            postimage_text,
            &postimage_bytes,
            self.postimage_w,
            n_ln,
            &mut self.postimage_breaks,
            self.cw,
            shared_breaks.as_ref(),
        );
        let pair = preimage_n.max(postimage_n);
        push_wrapped_rows(
            preimage,
            o_ln,
            kind,
            preimage_bytes.clone(),
            None,
            preimage_n,
            pair - preimage_n,
        );
        push_wrapped_rows(
            postimage,
            n_ln,
            kind,
            postimage_bytes.clone(),
            None,
            postimage_n,
            pair - postimage_n,
        );
        pair
    }
}
