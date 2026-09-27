//! Settings window building blocks, modeled on Zed's `settings_ui` / `ui`
//! crates (metrics copied, theme plumbing not). Only the Settings window uses
//! these; the rest of the app keeps its hand-written controls.

// Not wired up until the window moves here (settings-redesign issues 02/03).
#![allow(dead_code, unused_imports)]

mod button;
mod configured_card;
mod section_header;
mod setting_row;

pub use button::{Button, ButtonSize, ButtonStyle, TintColor};
pub use configured_card::ConfiguredCard;
pub use section_header::SectionHeader;
pub use setting_row::SettingRow;
