use anyhow::{Context, Result};
use std::fs::OpenOptions;

mod command;
mod config;
mod input;
mod session;

fn acquire_singleton_lock() -> Result<std::fs::File> {
    let dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into());
    let file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(format!("{dir}/keyrun.lock"))?;

    file.try_lock()
        .context("Another keyrun instance is already waiting for a key")?;

    Ok(file)
}

fn main() -> Result<()> {
    let Ok(_lock) = acquire_singleton_lock() else {
        return Ok(());
    };

    let config = config::KeyConfig::read()?;
    let timeout = config.timeout()?;

    input::listen(&config.keys, timeout)
}
