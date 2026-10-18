use anyhow::{Context, Result};
use configfs::{Config, ConfigPath};
use evdev::{AttributeSet, Device, EventSummary, EventType, KeyCode, uinput::VirtualDevice};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap, fs::OpenOptions, sync::mpsc, thread, time::Duration, time::Instant,
};
use udev::Device as UDevDevice;

mod command;
use command::*;

#[derive(Deserialize, Serialize, Debug, Default)]
struct KeyCommand {
    app: String,
    #[serde(default)]
    times: ListenTimes,
    #[serde(default)]
    links_with: Vec<char>,
    args: Option<Vec<String>>,
    env: Option<Vec<(String, String)>>,

    pipe: Option<Box<KeyCommand>>,
}

#[derive(Deserialize, Serialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum Unlimited {
    Infinite,
}

#[derive(Deserialize, Serialize, Debug, PartialEq, Eq)]
#[serde(untagged)]
enum ListenTimes {
    Limited(std::num::NonZeroUsize),
    Unlimited(Unlimited),
}
impl Default for ListenTimes {
    fn default() -> Self {
        ListenTimes::Limited(std::num::NonZeroUsize::new(1).unwrap())
    }
}

struct KeySession {
    keys: Box<[char]>,
    remaining: Option<usize>,
}

impl KeySession {
    fn new(key: char, command: &KeyCommand) -> Self {
        Self {
            keys: std::iter::once(key)
                .chain(command.links_with.iter().copied())
                .collect(),
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

// i will add modifiers also later for combinations

fn key_to_char(key: KeyCode) -> Option<char> {
    let name = format!("{key:?}");
    let char = name.strip_prefix("KEY_")?;

    // return only keys matching KEY_A to KEY_Z
    (char.len() == 1)
        .then(|| char.to_ascii_lowercase().chars().next())
        .flatten()
}

fn build_virtual_device(keyboards: &[Device]) -> Result<VirtualDevice> {
    let keys = keyboards
        .iter()
        .filter_map(|kb| kb.supported_keys())
        .flat_map(|keys| keys.iter())
        .collect::<AttributeSet<KeyCode>>();

    let vdev_name = "sink-passthrough-kbd";

    VirtualDevice::builder()?
        .name(vdev_name)
        .with_keys(&keys)?
        .build()
        .map_err(Into::into)
}

const EXIT_TIMEOUT: Duration = Duration::from_millis(2000);

fn main() -> Result<()> {
    let Ok(_lock) = acquire_singleton_lock() else {
        return Ok(());
    };

    let mut keyboards = get_keyboards();
    if keyboards.is_empty() {
        anyhow::bail!("No keyboards found");
    }

    for keyboard in keyboards.iter_mut() {
        keyboard.grab()?;
    }

    let config: Config<KeyConfig> = Config::new(ConfigPath::System("keyrun"))?;
    let config_keys = config.read()?.keys;
    for command in config_keys.values() {
        for key in &command.links_with {
            anyhow::ensure!(
                config_keys.contains_key(key),
                "Missing linked binding {key:?}"
            );
        }
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
        "Took {}ms to initialize virtual keyboard",
        now.elapsed().as_millis()
    );

    println!("Listening for keyboard events...");

    let (tx, rx) = mpsc::channel();

    for mut keyboard in keyboards {
        let tx = tx.clone();

        thread::spawn(move || {
            loop {
                match keyboard.fetch_events() {
                    Ok(events) => {
                        for event in events {
                            if tx.send(event).is_err() {
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

    let mut batch = Vec::new();
    let mut sinked_keys = Vec::new();
    let mut held_keys = Vec::new();
    let mut finished = false;
    let mut session: Option<KeySession> = None;
    let mut deadline = Instant::now() + EXIT_TIMEOUT;

    loop {
        let event = if held_keys.is_empty() {
            rx.recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .ok()
        } else {
            rx.recv().ok()
        };

        let Some(event) = event else { break };

        match event.destructure() {
            EventSummary::Synchronization(..) => {
                if !batch.is_empty() {
                    vdev.emit(&batch)?;
                    batch.clear();
                }
                if finished && sinked_keys.is_empty() && held_keys.is_empty() {
                    break;
                }
            }

            // release sinked
            EventSummary::Key(_, key, 0) if sinked_keys.contains(&key) => {
                sinked_keys.retain(|held| *held != key);
            }

            EventSummary::Key(_, key, 0) => {
                held_keys.retain(|held| *held != key);
                batch.push(event);
            }

            // press
            EventSummary::Key(_, key, 1) => {
                println!("{:?}", key);

                if !finished
                    && let Some(key_char) = key_to_char(key)
                    && let Some(key_cmd) = config_keys.get(&key_char)
                    && session.as_ref().is_none_or(|s| s.keys.contains(&key_char))
                {
                    if !sinked_keys.contains(&key) {
                        sinked_keys.push(key);
                    }

                    finished = session
                        .get_or_insert_with(|| KeySession::new(key_char, key_cmd))
                        .pressed();

                    deadline = Instant::now() + EXIT_TIMEOUT;

                    if let Err(error) = key_cmd.build().and_then(|mut app| app.spawn()) {
                        eprintln!("command failed: {error:#}")
                    }
                } else {
                    if !finished && session.is_some() {
                        finished = true;
                    }
                    if !held_keys.contains(&key) {
                        held_keys.push(key);
                    }
                    batch.push(event);
                }
            }
            _ => batch.push(event),
        }
    }

    if !batch.is_empty() {
        vdev.emit(&batch)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeat_count_defaults_and_validation() {
        let default: KeyCommand = toml::from_str(
            r#"
            app = "true"
            "#,
        )
        .unwrap();
        assert_eq!(default.times, ListenTimes::default());
        let repeated: KeyCommand = toml::from_str(
            r#"
            app = "true"
            times = 10
            "#,
        )
        .unwrap();
        assert_eq!(
            repeated.times,
            ListenTimes::Limited(std::num::NonZeroUsize::new(10).unwrap())
        );
        assert!(toml::from_str::<KeyCommand>("app = 'true'\ntimes = 0").is_err());
    }

    #[test]
    fn session_uses_starting_limit_and_direct_links() {
        let config: KeyConfig = toml::from_str(
            r#"
            [keys.u]
            app = "true"
            times = 3
            links_with = ["d"]

            [keys.d]
            app = "false"
            links_with = ["u"]
            times = 10
            "#,
        )
        .unwrap();
        let mut session = KeySession::new('u', &config.keys[&'u']);
        assert!(session.keys.contains(&'u'));
        assert!(session.keys.contains(&'d'));
        assert!(!session.keys.contains(&'x'));
        assert!(!session.pressed());
        assert!(!session.pressed());
        assert!(session.pressed());
        assert_eq!(KeySession::new('d', &config.keys[&'d']).remaining, Some(10));
    }

    #[test]
    fn infinite_session_never_exhausts() {
        let command = toml::from_str::<KeyCommand>(
            r#"
            app = "true"
            times = "infinite"
            "#,
        )
        .unwrap();

        let mut session = KeySession::new('u', &command);

        for _ in 0..1000 {
            assert!(!session.pressed());
        }

        assert_eq!(session.remaining, None);
        assert!(
            toml::from_str::<KeyCommand>(
                r#"
                app = true
                times = "what the"
                "#
            )
            .is_err()
        );
    }

    #[test]
    fn named_keys_do_not_match_letter_bindings() {
        assert_eq!(key_to_char(KeyCode::KEY_F), Some('f'));
        assert_eq!(key_to_char(KeyCode::KEY_1), Some('1'));
        assert_eq!(key_to_char(KeyCode::KEY_F1), None);
        assert_eq!(key_to_char(KeyCode::KEY_ENTER), None);
        assert_eq!(key_to_char(KeyCode::KEY_VOLUMEUP), None);
    }
}
