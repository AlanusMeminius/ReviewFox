//! Settings window and its building blocks, modeled on Zed's `settings_ui` /
//! `ui` crates (metrics copied, theme plumbing not). Only the Settings window
//! uses these; the rest of the app keeps its hand-written controls.

// Rebuilt GitLab page (settings-redesign issue 03) is their first user.
#[allow(dead_code, unused_imports)]
mod button;
#[allow(dead_code, unused_imports)]
mod configured_card;
mod nav;
mod nav_tree;
mod section_header;
#[allow(dead_code, unused_imports)]
mod setting_row;
mod window;

#[allow(unused_imports)]
pub use button::{Button, ButtonSize, ButtonStyle, TintColor};
#[allow(unused_imports)]
pub use configured_card::ConfiguredCard;
pub use section_header::SectionHeader;
#[allow(unused_imports)]
pub use setting_row::SettingRow;
pub use window::{SettingsView, key_bindings, open_or_focus_settings};
