#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod domain;
mod export;
mod git;
mod gitlab;
mod loaded_browser;
mod mr_entry;
mod publication;
mod reqwest_client;
mod review_store;
mod settings_store;
mod syntax;
mod theme_defaults;
mod ui;
mod window_geometry_store;
mod workspace_store;

fn main() {
    ui::run();
}
