use anyhow::{Context, Result};
use slint::ComponentHandle;

use crate::AppWindow;
use crate::config::resolve_runtime_config;
use crate::controller::DesktopController;
use crate::data::DesktopDataSource;

pub fn run() -> Result<()> {
    let ui = AppWindow::new().context("failed to create Slint window")?;
    let runtime = resolve_runtime_config()?;
    let source = DesktopDataSource::new(runtime.clone())?;
    let controller = DesktopController::new(&ui, source, runtime.app_paths.prefs_path.clone());
    let _controller = controller;

    ui.run().context("desktop window exited with an error")
}
