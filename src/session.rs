use crate::config::{KeyCommand, ListenTimes};
use anyhow::Result;
use evdev::{EventSummary, KeyCode};
use std::{
    collections::HashMap,
    sync::mpsc,
    time::{Duration, Instant},
};

struct Session<'a> {
    key: char,
    links: &'a [char],
    remaining: Option<usize>,
}

impl<'a> Session<'a> {
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

fn key_to_char(key: KeyCode) -> Option<char> {
    let name = format!("{key:?}");
    let name = name.strip_prefix("KEY_")?;

    (name.len() == 1)
        .then(|| name.chars().next().map(|c| c.to_ascii_lowercase()))
        .flatten()
}

pub(crate) fn listen(
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
                        .is_none_or(|session| session.contains(key_char))
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
