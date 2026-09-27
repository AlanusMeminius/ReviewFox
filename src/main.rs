#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod domain;
mod export;
mod git;
mod gitlab;
mod reqwest_client;
mod settings_store;
mod ui;
mod workspace_store;

fn main() {
    ui::run();
}
