use anyhow::{Context, Result};
use appcfg::{Config, ConfigDirectory, ConfigError};
use evdev::{Device, EventSummary, InputEvent, KeyCode, uinput::VirtualDevice};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fs::OpenOptions, time::Duration};

mod command;
use command::*;

#[derive(Deserialize, Serialize, Debug)]
struct KeyCommand {
    app: String,
    args: Option<Vec<String>>,
    env: Option<Vec<(String, String)>>,

    pipe: Option<Box<KeyCommand>>,
}

impl Default for KeyCommand {
    fn default() -> Self {
        Self {
            app: "".to_string(),
            args: None,
            env: None,
            pipe: None,
        }
    }
}

#[derive(Deserialize, Serialize, Default, Debug)]
struct KeyConfig {
    keys: HashMap<char, KeyCommand>,
}

impl KeyCommand {
    pub fn build(&self) -> Result<Command> {
        let mut cmd = Command::new(&self.app);

        if let Some(args) = &self.args {
            cmd = cmd.add_args(args);
        }

        if let Some(env) = &self.env {
            for (k, v) in env {
                cmd = cmd.add_env(k, v);
            }
        }

        if let Some(pipe_cmd) = &self.pipe {
            let mut parent = pipe_cmd.build()?;
            parent = parent.pipe(cmd)?;
            Ok(parent)
        } else {
            Ok(cmd)
        }
    }
}

fn get_keyboard() -> Option<Device> {
    evdev::enumerate()
        .filter_map(|(_, device)| {
            let keys = device.supported_keys()?;

            if keys.contains(KeyCode::KEY_A)
                && keys.contains(KeyCode::KEY_ENTER)
                && keys.contains(KeyCode::KEY_SPACE)
            {
                Some(device)
            } else {
                None
            }
        })
        .next()
}

fn acquire_singleton_lock() -> Result<std::fs::File> {
    let dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into());
    let path = format!("{dir}/keyrun.lock");
    let file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(path)?;
    file.try_lock()
        .context("Another keyrun instance is already waiting for a key")?;
    Ok(file)
}

const EXIT_TIMEOUT_MS: u64 = 2000;

fn main() -> Result<()> {
    let _lock = match acquire_singleton_lock() {
        Ok(f) => f,
        Err(_) => std::process::exit(0),
    };

    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_millis(EXIT_TIMEOUT_MS));
        std::process::exit(1);
    });

    let config_handle = std::thread::spawn(|| -> Result<KeyConfig, ConfigError> {
        let config = Config::new(ConfigDirectory::System("keyrun"))?;
        config.read::<KeyConfig>()
    });

    let mut keyboard = match get_keyboard() {
        Some(kb) => kb,
        _ => {
            println!("Couldn't get keyboard");
            std::process::exit(1);
        }
    };

    let keys = keyboard.supported_keys().context("No key capabilities")?;

    let vdev_name = "sink-passthrough-kbd";
    let mut vdev = VirtualDevice::builder()?
        .name(vdev_name)
        .with_keys(keys)?
        .build()?;

    std::thread::sleep(Duration::from_millis(200));

    keyboard.grab()?;

    println!("Listening for keyboard events...");

    let mut batch: Vec<InputEvent> = Vec::new();

    loop {
        for event in keyboard.fetch_events()? {
            match event.destructure() {
                EventSummary::Synchronization(..) => {
                    if !batch.is_empty() {
                        vdev.emit(&batch)?;
                        batch.clear();
                    }
                }
                EventSummary::Key(_, key, 1) => {
                    if let Some(key_str) = format!("{:?}", key)
                        .strip_prefix("KEY_")
                        .map(|k| k.to_lowercase())
                    {
                        let key_char: char = key_str.parse()?;

                        let data = config_handle.join().unwrap()?;

                        if let Some(key_cmd) = data.keys.get(&key_char)
                            && let Ok(mut app) = key_cmd.build()
                        {
                            let _ = app.spawn();
                        }
                        std::process::exit(0);
                    }
                }
                _ => batch.push(event),
            }
        }
    }
}
