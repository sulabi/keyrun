use anyhow::{Context, Result};
use configfs::{Config, ConfigDirectory};
use evdev::{AttributeSet, Device, EventSummary, EventType, KeyCode, uinput::VirtualDevice};
use serde::{Deserialize, Serialize};
use std::thread;
use std::time::Instant;
use std::{collections::HashMap, fs::OpenOptions, sync::mpsc, time::Duration};
use udev::Device as UDevDevice;

mod command;
use command::*;

#[derive(Deserialize, Serialize, Debug)]
struct KeyCommand {
    app: String,
    #[serde(default)]
    times: ListenTimes,
    args: Option<Vec<String>>,
    env: Option<Vec<(String, String)>>,

    pipe: Option<Box<KeyCommand>>,
}

impl Default for KeyCommand {
    fn default() -> Self {
        Self {
            app: "".to_string(),
            times: ListenTimes::default(),
            args: None,
            env: None,
            pipe: None,
        }
    }
}

#[derive(Deserialize, Serialize, Debug)]
#[serde(rename_all = "lowercase")]
enum Unlimited {
    Infinite,
}

#[derive(Deserialize, Serialize, Debug)]
#[serde(untagged)]
enum ListenTimes {
    Limited(std::num::NonZeroUsize),
    Unlimited(Unlimited),
}

impl Default for ListenTimes {
    fn default() -> Self {
        Self::Limited(std::num::NonZeroUsize::new(1).unwrap())
    }
}

struct Session {
    key: char,
    remaining: Option<usize>,
}

impl Session {
    fn new(key: char, command: &KeyCommand) -> Self {
        Self {
            key,
            remaining: match command.times {
                ListenTimes::Limited(count) => Some(count.get()),
                ListenTimes::Unlimited(_) => None,
            },
        }
    }

    fn pressed(&mut self) -> bool {
        if let Some(remaining) = &mut self.remaining {
            *remaining -= 1;
            *remaining == 0
        } else {
            false
        }
    }
}

#[derive(Deserialize, Serialize, Debug)]
struct KeyConfig {
    #[serde(default = "default_timeout_seconds")]
    timeout_seconds: f64,
    keys: HashMap<char, KeyCommand>,
}

fn default_timeout_seconds() -> f64 {
    1.0
}

impl Default for KeyConfig {
    fn default() -> Self {
        Self {
            timeout_seconds: default_timeout_seconds(),
            keys: HashMap::new(),
        }
    }
}

impl KeyConfig {
    fn timeout(&self) -> Result<Duration> {
        let timeout = Duration::try_from_secs_f64(self.timeout_seconds)
            .context("timeout_seconds must be a finite, positive number")?;

        anyhow::ensure!(
            !timeout.is_zero(),
            "timeout_seconds must be greater than zero"
        );
        Instant::now()
            .checked_add(timeout)
            .context("timeout_seconds is too large")?;

        Ok(timeout)
    }
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
fn is_keyboard(device: &Device) -> bool {
    let events = device.supported_events();
    events.contains(EventType::KEY)
        && events.contains(EventType::REPEAT)
        && device
            .supported_keys()
            .is_some_and(|keys| keys.contains(KeyCode::KEY_SPACE))
}

fn get_keyboards() -> Vec<Device> {
    evdev::enumerate()
        .map(|(_, device)| device)
        .filter(is_keyboard)
        .collect()
}

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

fn key_to_char(key: KeyCode) -> Option<char> {
    let name = format!("{key:?}");
    let name = name.strip_prefix("KEY_")?;

    (name.len() == 1)
        .then(|| name.chars().next().map(|c| c.to_ascii_lowercase()))
        .flatten()
}

fn build_virtual_device(keyboards: &[Device]) -> Result<VirtualDevice> {
    let keys = keyboards
        .iter()
        .filter_map(|kb| kb.supported_keys())
        .flat_map(|keys| keys.iter())
        .collect::<AttributeSet<KeyCode>>();

    let vdev_name = "sink-passthrough-kbd";
    let vdev = VirtualDevice::builder()?
        .name(vdev_name)
        .with_keys(&keys)?
        .build()?;

    Ok(vdev)
}

fn main() -> Result<()> {
    let Ok(_lock) = acquire_singleton_lock() else {
        return Ok(());
    };

    let config: Config<KeyConfig> = Config::new(ConfigDirectory::System("keyrun"))?;
    let config = config.read()?;
    let timeout = config.timeout()?;
    let config_keys = config.keys;

    let mut keyboards = get_keyboards();
    if keyboards.is_empty() {
        anyhow::bail!("No keyboards found");
    }

    let mut vdev = build_virtual_device(&keyboards)?;

    let sys_path = vdev.get_syspath()?;
    let now = Instant::now();
    for _ in 0..50 {
        if let Ok(d) = UDevDevice::from_syspath(&sys_path)
            && d.is_initialized()
        {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    println!(
        "Took {}ms to initlaize virtual keyboard",
        now.elapsed().as_millis()
    );

    for keyboard in keyboards.iter_mut() {
        keyboard.grab()?;
    }

    println!("Listening for keyboard events...");

    let (tx, rx) = mpsc::channel();

    for (source, mut keyboard) in keyboards.into_iter().enumerate() {
        let tx = tx.clone();

        thread::spawn(move || {
            loop {
                match keyboard.fetch_events() {
                    Ok(events) => {
                        for event in events {
                            if tx.send((source, event)).is_err() {
                                return;
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("keyboard error: {e}");
                        return;
                    }
                }
            }
        });
    }

    drop(tx);

    listen(&rx, &config_keys, timeout, |batch| {
        vdev.emit(batch).map_err(Into::into)
    })
}

fn listen(
    rx: &mpsc::Receiver<(usize, evdev::InputEvent)>,
    config_keys: &HashMap<char, KeyCommand>,
    timeout: Duration,
    mut emit: impl FnMut(&[evdev::InputEvent]) -> Result<()>,
) -> Result<()> {
    let mut batch = Vec::new();
    let mut consumed = Vec::new();
    let mut held = Vec::new();
    let mut session: Option<Session> = None;
    let mut finished = false;
    let mut deadline = Instant::now() + timeout;

    loop {
        let input = if consumed.is_empty() && held.is_empty() {
            rx.recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .ok()
        } else {
            rx.recv().ok()
        };

        let Some((source, event)) = input else { break };

        match event.destructure() {
            EventSummary::Synchronization(..) => {
                if !batch.is_empty() {
                    emit(&batch)?;
                    batch.clear();
                }

                if finished && consumed.is_empty() && held.is_empty() {
                    break;
                }
            }
            EventSummary::Key(_, key, 0) if consumed.contains(&(source, key)) => {
                consumed.retain(|held| *held != (source, key));
            }
            EventSummary::Key(_, key, 0) => {
                held.retain(|held| *held != (source, key));
                batch.push(event);
            }
            EventSummary::Key(_, key, 1) => {
                if !finished
                    && let Some(key_char) = key_to_char(key)
                    && let Some(key_cmd) = config_keys.get(&key_char)
                    && session
                        .as_ref()
                        .is_none_or(|session| session.key == key_char)
                {
                    if !consumed.contains(&(source, key)) {
                        consumed.push((source, key));
                    }

                    finished = session
                        .get_or_insert_with(|| Session::new(key_char, key_cmd))
                        .pressed();
                    deadline = Instant::now() + timeout;

                    if let Err(error) = key_cmd.build().and_then(|mut app| app.spawn()) {
                        eprintln!("command failed: {error:#}");
                    }
                } else {
                    finished |= session.is_some();

                    if !held.contains(&(source, key)) {
                        held.push((source, key));
                    }

                    batch.push(event);
                }
            }
            EventSummary::Key(_, key, 2) if consumed.contains(&(source, key)) => {}
            _ => batch.push(event),
        }
    }

    if !batch.is_empty() {
        emit(&batch)?;
    }

    Ok(())
}
