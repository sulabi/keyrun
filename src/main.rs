use anyhow::{Context, Result};
use clap::Parser;
use configfs::{Config, ConfigPath};
use evdev::{EventSummary, KeyCode};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs::OpenOptions,
    sync::mpsc,
    time::{Duration, Instant},
};

#[cfg(feature = "overlay")]
mod overlay;

mod input;
mod passthrough;

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

struct KeySession<'a> {
    key: char,
    links: &'a [char],
    remaining: Option<usize>,
}

impl<'a> KeySession<'a> {
    fn new(key: char, command: &'a KeyCommand) -> Self {
        Self {
            key,
            links: &command.links_with,
            remaining: match command.times {
                ListenTimes::Limited(count) => Some(count.get()),
                ListenTimes::Unlimited(_) => None,
            },
        }
    }

    fn contains(&self, key: char) -> bool {
        self.key == key || self.links.contains(&key)
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
    #[cfg(feature = "overlay")]
    #[serde(default)]
    overlay: overlay::OverlayConfig,
    keys: HashMap<char, KeyCommand>,
}

fn default_timeout_seconds() -> f64 {
    1.
}

impl Default for KeyConfig {
    fn default() -> Self {
        Self {
            timeout_seconds: default_timeout_seconds(),
            #[cfg(feature = "overlay")]
            overlay: overlay::OverlayConfig::default(),
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
    fn label(&self) -> String {
        let command = std::iter::once(self.app.as_str())
            .chain(self.args.iter().flatten().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" ");

        match &self.pipe {
            Some(pipe) => format!("{command} | {}", pipe.label()),
            None => command,
        }
    }

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

#[derive(Parser)]
#[command(version, about)]
struct Args {
    /// Show keypress feedback (requires the overlay build feature)
    #[arg(long)]
    overlay: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();

    #[cfg(not(feature = "overlay"))]
    if args.overlay {
        anyhow::bail!("Rebuild with `cargo build --features overlay` to use --overlay");
    }

    let Ok(lock) = acquire_singleton_lock() else {
        return Ok(());
    };

    // Open input before configuration or graphics setup so early presses are queued
    let keyboards = input::Keyboards::open()?;

    let config: Config<KeyConfig> = Config::new(ConfigPath::System("keyrun"))?;
    let config = config.read()?;
    let timeout = config.timeout()?;
    let config_keys = config.keys;

    for command in config_keys.values() {
        for key in &command.links_with {
            anyhow::ensure!(
                config_keys.contains_key(key),
                "Missing linked binding {key:?}"
            );
        }
    }

    let run_listener = move |feedback| {
        let result = listen(feedback, keyboards, config_keys, timeout);

        // The listener has released its keyboards; allow another invocation
        // while this overlay finishes its closing animation.
        drop(lock);

        result
    };

    #[cfg(feature = "overlay")]
    if args.overlay {
        return overlay::run(config.overlay, run_listener);
    }

    run_listener(None)
}

#[cfg_attr(not(feature = "overlay"), allow(dead_code))]
struct KeyFeedback {
    key: KeyCode,
    valid: bool,
    command: Option<String>,
    remaining: Option<usize>,
}

struct Feedback {
    tx: mpsc::Sender<KeyFeedback>,
    #[cfg(feature = "overlay")]
    context: std::sync::Arc<std::sync::OnceLock<eframe::egui::Context>>,
}

impl Feedback {
    fn send(&self, event: KeyFeedback) -> Result<(), mpsc::SendError<KeyFeedback>> {
        self.tx.send(event)?;

        #[cfg(feature = "overlay")]
        if let Some(context) = self.context.get() {
            context.request_repaint();
        }

        Ok(())
    }
}

fn listen(
    feedback: Option<Feedback>,
    mut keyboards: input::Keyboards,
    config_keys: HashMap<char, KeyCommand>,
    timeout: Duration,
) -> Result<()> {
    let mut passthrough = passthrough::Passthrough::new(keyboards.devices())?;

    run_session(
        feedback,
        &config_keys,
        timeout,
        |wait| keyboards.next(wait),
        |events| passthrough.emit(events),
    )
}

fn run_session(
    feedback: Option<Feedback>,
    config_keys: &HashMap<char, KeyCommand>,
    timeout: Duration,
    mut next: impl FnMut(Option<Duration>) -> Result<Option<input::Event>>,
    mut emit: impl FnMut(&[evdev::InputEvent]) -> Result<()>,
) -> Result<()> {
    let mut batch = Vec::new();
    let mut sinked_keys = Vec::new();
    let mut held_keys = Vec::new();
    let mut finished = false;
    let mut session: Option<KeySession> = None;
    let mut deadline = Instant::now()
        .checked_add(timeout)
        .context("timeout_seconds is too large")?;

    loop {
        let wait = if held_keys.is_empty() && sinked_keys.is_empty() {
            Some(deadline.saturating_duration_since(Instant::now()))
        } else {
            None
        };
        let Some(input::Event {
            input: event,
            source,
            captured,
        }) = next(wait)?
        else {
            break;
        };

        match event.destructure() {
            EventSummary::Synchronization(..) => {
                if !batch.is_empty() {
                    emit(&batch)?;
                    batch.clear();
                }

                if finished && sinked_keys.is_empty() && held_keys.is_empty() {
                    break;
                }
            }

            // release sinked
            EventSummary::Key(_, key, 0) if sinked_keys.contains(&(source, key)) => {
                sinked_keys.retain(|held| *held != (source, key));
            }

            EventSummary::Key(_, key, 0) => {
                held_keys.retain(|held| *held != (source, key));

                if captured {
                    batch.push(event);
                }
            }

            // press
            EventSummary::Key(_, key, 1) => {
                if !finished
                    && let Some(key_char) = key_to_char(key)
                    && let Some(key_cmd) = config_keys.get(&key_char)
                    && session.as_ref().is_none_or(|s| s.contains(key_char))
                {
                    if captured && !sinked_keys.contains(&(source, key)) {
                        sinked_keys.push((source, key));
                    }

                    let active = session.get_or_insert_with(|| KeySession::new(key_char, key_cmd));
                    finished = active.pressed();

                    if let Some(tx) = &feedback {
                        let _ = tx.send(KeyFeedback {
                            key,
                            valid: true,
                            command: Some(key_cmd.label()),
                            remaining: active.remaining,
                        });
                    }

                    deadline = Instant::now()
                        .checked_add(timeout)
                        .context("timeout_seconds is too large")?;

                    if let Err(error) = key_cmd.build().and_then(|mut app| app.spawn()) {
                        eprintln!("command failed: {error:#}")
                    }
                } else {
                    if !finished && let Some(tx) = &feedback {
                        let _ = tx.send(KeyFeedback {
                            key,
                            valid: false,
                            command: None,
                            remaining: None,
                        });
                    }

                    if !finished && session.is_some() {
                        finished = true;
                    }

                    if captured {
                        if !held_keys.contains(&(source, key)) {
                            held_keys.push((source, key));
                        }

                        batch.push(event);
                    }
                }
            }
            EventSummary::Key(_, key, 2) if sinked_keys.contains(&(source, key)) => {}
            _ if captured => batch.push(event),
            _ => {}
        }
    }

    if !batch.is_empty() {
        emit(&batch)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key_event(source: usize, key: KeyCode, value: i32, captured: bool) -> input::Event {
        input::Event {
            input: evdev::InputEvent::new(evdev::EventType::KEY.0, key.0, value),
            source,
            captured,
        }
    }

    fn sync_event(source: usize, captured: bool) -> input::Event {
        input::Event {
            input: evdev::InputEvent::new(evdev::EventType::SYNCHRONIZATION.0, 0, 0),
            source,
            captured,
        }
    }

    #[test]
    fn mapped_press_repeats_and_release_are_consumed_on_the_same_keyboard() {
        let config: KeyConfig = toml::from_str("[keys.s]\napp='true'").unwrap();
        let mut events = std::collections::VecDeque::from([
            key_event(0, KeyCode::KEY_S, 1, true),
            key_event(0, KeyCode::KEY_S, 2, true),
            key_event(1, KeyCode::KEY_S, 0, false),
            sync_event(1, false),
            key_event(0, KeyCode::KEY_S, 0, true),
            sync_event(0, true),
        ]);
        let mut forwarded = Vec::new();
        let mut waits = Vec::new();

        run_session(
            None,
            &config.keys,
            Duration::from_millis(1),
            |wait| {
                waits.push(wait);
                Ok(events.pop_front())
            },
            |batch| {
                forwarded.extend_from_slice(batch);
                Ok(())
            },
        )
        .unwrap();

        assert!(events.is_empty(), "wait for the consumed key's own release");
        assert!(
            forwarded.is_empty(),
            "consumed repeats must never reach uinput"
        );
        assert!(
            waits[1..waits.len() - 1].iter().all(Option::is_none),
            "a held key must survive the idle timeout"
        );
    }

    #[test]
    fn native_startup_press_runs_once_without_replaying_its_events() {
        for release_captured in [false, true] {
            let config: KeyConfig = toml::from_str("[keys.s]\napp='true'").unwrap();
            let mut events = std::collections::VecDeque::from([
                key_event(0, KeyCode::KEY_S, 1, false),
                key_event(0, KeyCode::KEY_S, 2, false),
                key_event(0, KeyCode::KEY_S, 0, release_captured),
                sync_event(0, release_captured),
            ]);
            let (tx, rx) = mpsc::channel();
            let feedback = Feedback {
                tx,
                #[cfg(feature = "overlay")]
                context: std::sync::Arc::new(std::sync::OnceLock::new()),
            };
            let mut forwarded = Vec::new();

            run_session(
                Some(feedback),
                &config.keys,
                Duration::from_secs(1),
                |_| Ok(events.pop_front()),
                |batch| {
                    forwarded.extend_from_slice(batch);
                    Ok(())
                },
            )
            .unwrap();

            let feedback = rx.try_iter().collect::<Vec<_>>();
            assert_eq!(feedback.len(), 1);
            assert!(feedback[0].valid);
            assert_eq!(feedback[0].key, KeyCode::KEY_S);
            let expected = if release_captured {
                vec![key_event(0, KeyCode::KEY_S, 0, true).input]
            } else {
                Vec::new()
            };
            assert_eq!(forwarded, expected, "forward only the captured release");
        }
    }
}
