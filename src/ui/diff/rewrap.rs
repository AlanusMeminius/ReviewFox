//! Soft-wrap timing and scroll landing policy. Layout construction and glyph
//! measurement remain in the pane; Viewport remains unaware of pending lands.

use super::layout::{Layout, WrapPlan};
use super::viewport::{self, AnchorCap};
use crate::domain::HunkJumpTarget;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Land {
    Anchor(AnchorCap),
    Line(HunkJumpTarget),
    Match { target: HunkJumpTarget, byte: usize },
}

#[derive(Clone, Copy, Debug)]
pub enum Event {
    Widths([f32; 2], WrapPlan),
    FileOpen,
    Search {
        target: HunkJumpTarget,
        byte: Option<usize>,
        expanded: bool,
    },
    Reveal {
        target: HunkJumpTarget,
        expanded: bool,
    },
    Fold,
    SoftWrapOn,
    SoftWrapOff,
    FontSize,
    FontFamily,
    Rebuilt,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct State {
    soft_wrap: bool,
    stale: bool,
    previous_widths: Option<[f32; 2]>,
    pending: Option<Land>,
    rebuilding_land: Option<Land>,
    restore_unmeasured: bool,
}

/// Values measured/captured by the pane before sending an event.
pub struct Input<'a> {
    pub layout: Option<&'a Layout>,
    pub can_measure: bool,
    pub anchor: Option<AnchorCap>,
    pub view_h: f32,
    pub row_h: f32,
    pub scroll_s: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Effect {
    /// `Some(false)` builds without wrap, `Some(true)` includes wrap.
    pub rebuild: Option<bool>,
    pub scroll_s: Option<f32>,
    pub jumped: bool,
    /// A changed width needs another frame to observe whether it settled.
    pub observe_again: bool,
}

impl State {
    pub fn new(soft_wrap: bool) -> Self {
        Self {
            soft_wrap,
            stale: soft_wrap,
            ..Self::default()
        }
    }

    /// All timing state changes pass through this interface, including the
    /// second call after the pane has constructed the requested Layout.
    pub fn event(mut self, event: Event, input: Input<'_>) -> (Self, Effect) {
        let mut effect = Effect::default();
        match event {
            Event::Widths(widths, plan) => {
                if self.soft_wrap {
                    let stable = self.previous_widths == Some(widths);
                    self.previous_widths = Some(widths);
                    let applied = input.layout.and_then(|l| l.wrap.as_ref());
                    if self.stale || applied.is_none() {
                        self.rebuild(&input, false, &mut effect);
                    } else if !stable {
                        effect.observe_again = true;
                    } else if applied.is_some_and(|a| a.plan != plan) {
                        self.rebuild(&input, false, &mut effect);
                    }
                }
            }
            Event::FileOpen => {
                self.pending = None;
                self.rebuilding_land = None;
                self.previous_widths = None;
                self.stale = self.soft_wrap;
                effect.rebuild = Some(self.soft_wrap && input.can_measure);
                self.restore_unmeasured = false;
            }
            Event::SoftWrapOff => {
                self.soft_wrap = false;
                self.stale = false;
                self.pending = None;
                self.rebuilding_land = input.anchor.map(Land::Anchor);
                self.restore_unmeasured = true;
                effect.rebuild = Some(false);
            }
            Event::SoftWrapOn | Event::FontSize | Event::FontFamily => {
                if matches!(event, Event::SoftWrapOn) {
                    self.soft_wrap = true;
                }
                if self.soft_wrap {
                    self.stale = true;
                    self.rebuild(&input, true, &mut effect);
                }
            }
            Event::Fold => {
                self.stale = self.soft_wrap;
                self.rebuild(&input, false, &mut effect);
            }
            Event::Search {
                target, expanded, ..
            }
            | Event::Reveal { target, expanded } => {
                let byte = match event {
                    Event::Search { byte, .. } => byte,
                    _ => None,
                };
                let land = byte
                    .map(|byte| Land::Match { target, byte })
                    .unwrap_or(Land::Line(target));
                if expanded {
                    self.stale = self.soft_wrap;
                }
                if self.soft_wrap && (self.stale || input.layout.is_none_or(|l| l.wrap.is_none())) {
                    self.pending = Some(land);
                    if expanded {
                        self.rebuild(&input, false, &mut effect);
                    }
                } else if expanded {
                    self.rebuilding_land = Some(land);
                    self.restore_unmeasured = true;
                    effect.rebuild = Some(false);
                } else {
                    effect.scroll_s = land.scroll(&input);
                    effect.jumped = effect.scroll_s.is_some();
                }
            }
            Event::Rebuilt => {
                let wrapped = input.layout.is_some_and(|l| l.wrap.is_some());
                self.stale = self.soft_wrap && !wrapped;
                if wrapped || !self.soft_wrap {
                    let land = self.pending.take().or(self.rebuilding_land.take());
                    effect.scroll_s = land.and_then(|land| land.scroll(&input));
                    effect.jumped = land.is_some_and(|land| !matches!(land, Land::Anchor(_)))
                        && effect.scroll_s.is_some();
                } else if self.restore_unmeasured {
                    // Preserve a deferred jump, but restore the current anchor
                    // now for toggle/font changes, and again after measurement.
                    effect.scroll_s = input
                        .anchor
                        .and_then(|cap| Land::Anchor(cap).scroll(&input));
                }
                self.restore_unmeasured = false;
            }
        }
        (self, effect)
    }

    fn rebuild(&mut self, input: &Input<'_>, restore_unmeasured: bool, effect: &mut Effect) {
        let anchor = input.anchor.map(Land::Anchor);
        self.rebuilding_land = self.pending.or(anchor);
        if self.soft_wrap && self.pending.is_none() {
            self.pending = anchor;
        }
        self.restore_unmeasured = restore_unmeasured;
        effect.rebuild = Some(self.soft_wrap && input.can_measure);
    }
}

impl Land {
    fn scroll(self, input: &Input<'_>) -> Option<f32> {
        let layout = input.layout?;
        let s = match self {
            Self::Anchor(cap) => {
                viewport::s_for_rewrap(layout, cap, input.view_h, input.row_h, input.scroll_s)
            }
            Self::Line(target) => {
                viewport::s_for_target(layout, target, input.row_h, input.scroll_s)
            }
            Self::Match { target, byte } => viewport::s_for_match_byte(
                layout,
                target.side,
                target.ln,
                byte,
                input.view_h,
                input.row_h,
                input.scroll_s,
            )
            .or_else(|| viewport::s_for_target(layout, target, input.row_h, input.scroll_s)),
        }?;
        Some(viewport::clamp_s(layout, s, input.view_h, input.row_h))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(layout: Option<&Layout>, can_measure: bool) -> Input<'_> {
        Input {
            layout,
            can_measure,
            anchor: None,
            view_h: 60.,
            row_h: 10.,
            scroll_s: 0.,
        }
    }

    #[test]
    fn stale_rebuilds_without_waiting_and_unmeasured_stays_stale() {
        let (state, effect) = State::default().event(Event::SoftWrapOn, input(None, false));
        assert_eq!(effect.rebuild, Some(false));
        let (state, _) = state.event(Event::Rebuilt, input(None, false));
        let (_, effect) = state.event(Event::Widths([90., 90.], plan(90.)), input(None, true));
        assert_eq!(effect.rebuild, Some(true));
    }

    fn plan(width: f32) -> WrapPlan {
        use super::super::visual_wrap::WrapSide;
        WrapPlan {
            preimage: WrapSide { width_px: width },
            postimage: WrapSide { width_px: width },
        }
    }
}
