#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod domain;
mod export;
mod git;
mod gitlab;
mod mr_entry;
mod loaded_browser;
mod reqwest_client;
mod settings_store;
mod syntax;
mod ui;
mod window_geometry_store;
mod workspace_store;

fn main() {
    ui::run();
}
