mod app;
mod compat;
mod config;
mod connectors;
mod controller;
mod data;
mod storage;
mod view_models;
#[cfg(test)]
mod ui_contract_tests;

slint::include_modules!();

fn main() {
    if let Err(error) = app::run() {
        eprintln!("distill-desktop failed: {error:?}");
        std::process::exit(1);
    }
}
