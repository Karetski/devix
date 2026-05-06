use std::path::PathBuf;

use anyhow::Result;

mod app;
mod clipboard;
mod events;
mod plugin;
mod render;
mod runtime;
mod watcher;

use crate::app::App;
use crate::runtime::Application;

fn main() -> Result<()> {
    let path = std::env::args().nth(1).map(PathBuf::from);

    let runtime = Application::new();
    let app = App::new(path, Some(runtime.waker()))?;
    runtime.run(app)
}
