//! Settings window and its building blocks, modeled on Zed's `settings_ui` /
//! `ui` crates (metrics copied, theme plumbing not). Only the Settings window
//! uses these; the rest of the app keeps its hand-written controls.

mod button;
mod configured_card;
mod nav;
mod nav_tree;
mod section_header;
mod setting_row;
mod token_row;
mod window;

pub use button::{Button, ButtonSize, ButtonStyle};
pub use configured_card::ConfiguredCard;
pub use section_header::SectionHeader;
pub use setting_row::SettingRow;
pub use window::{init, key_bindings, open_or_focus_settings};
