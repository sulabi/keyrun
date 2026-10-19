use anyhow::{Context, Result};
use evdev::{AttributeSet, Device, EventSummary, EventType, InputEvent, KeyCode};
use nix::poll::{PollFd, PollFlags, PollTimeout, poll};
use std::{
    collections::VecDeque,
    io,
    os::fd::AsFd,
    time::{Duration, Instant, SystemTime},
};

pub struct Keyboards {
    devices: Vec<Keyboard>,
    pending: VecDeque<Event>,
}

struct Keyboard {
    device: Device,
    state: Device,
    initial_keys: AttributeSet<KeyCode>,
    captured_since: Option<SystemTime>,
}

pub struct Event {
    pub input: InputEvent,
    pub source: usize,
    // Events treated as native must not be replayed to the desktop
    pub captured: bool,
}

fn is_keyboard(device: &Device) -> bool {
    device.supported_events().contains(EventType::KEY)
        && device.supported_events().contains(EventType::REPEAT)
        && device
            .supported_keys()
            .is_some_and(|keys| keys.contains(KeyCode::KEY_SPACE))
}

fn initial_key(keys: &mut AttributeSet<KeyCode>, event: &InputEvent) -> bool {
    if let EventSummary::Key(_, key, value) = event.destructure()
        && keys.contains(key)
    {
        if value == 0 {
            keys.remove(key);
        }

        return true;
    }

    false
}

impl Keyboards {
    pub fn open() -> Result<Self> {
        let mut keyboards = Self {
            devices: Vec::new(),
            pending: VecDeque::new(),
        };

        for (path, device) in evdev::enumerate().filter(|(_, device)| is_keyboard(device)) {
            device.set_nonblocking(true)?;

            // EVIOCGKEY clears queued key events on the queried descriptor
            // Use a separate descriptor so state checks cannot erase our input
            let state = Device::open(path)?;
            let initial_keys = state.get_key_state()?;
            let source = keyboards.devices.len();
            let mut keyboard = Keyboard {
                device,
                state,
                initial_keys,
                captured_since: None,
            };

            // Drain events seen before the grab as native events, not captured presses
            keyboard.read(source, &mut keyboards.pending)?;
            keyboard.try_grab()?;
            keyboards.devices.push(keyboard);
        }

        anyhow::ensure!(!keyboards.devices.is_empty(), "No keyboards found");

        Ok(keyboards)
    }

    pub fn devices(&self) -> impl Iterator<Item = &Device> {
        self.devices.iter().map(|keyboard| &keyboard.device)
    }

    pub fn next(&mut self, timeout: Option<Duration>) -> Result<Option<Event>> {
        let deadline = timeout.and_then(|duration| Instant::now().checked_add(duration));

        loop {
            if let Some(event) = self.pending.pop_front() {
                return Ok(Some(event));
            }

            let mut descriptors = self
                .devices
                .iter()
                .map(|keyboard| PollFd::new(keyboard.device.as_fd(), PollFlags::POLLIN))
                .collect::<Vec<_>>();

            let wait = deadline.map_or(PollTimeout::NONE, |deadline| {
                PollTimeout::try_from(deadline.saturating_duration_since(Instant::now()))
                    .unwrap_or(PollTimeout::MAX)
            });

            match poll(&mut descriptors, wait) {
                Ok(0) if deadline.is_some_and(|deadline| Instant::now() >= deadline) => {
                    return Ok(None);
                }
                Ok(_) => {}
                Err(nix::errno::Errno::EINTR) => continue,
                Err(error) => return Err(error.into()),
            }

            for descriptor in &descriptors {
                let flags = descriptor.revents().unwrap_or(PollFlags::empty());
                anyhow::ensure!(
                    !flags
                        .intersects(PollFlags::POLLERR | PollFlags::POLLHUP | PollFlags::POLLNVAL),
                    "Keyboard disconnected while listening"
                );
            }

            drop(descriptors);

            for (source, keyboard) in self.devices.iter_mut().enumerate() {
                keyboard.read(source, &mut self.pending)?;
                keyboard.try_grab()?;
            }
        }
    }
}

impl Keyboard {
    fn read(&mut self, source: usize, pending: &mut VecDeque<Event>) -> Result<()> {
        let captured_since = self.captured_since;
        let events = match self.device.fetch_events() {
            Ok(events) => events,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(()),
            Err(error) => return Err(error.into()),
        };

        for input in events {
            let captured = captured_since.is_some_and(|since| input.timestamp() > since);

            if !captured && initial_key(&mut self.initial_keys, &input) {
                continue;
            }

            pending.push_back(Event {
                input,
                source,
                captured,
            });
        }

        Ok(())
    }

    fn try_grab(&mut self) -> Result<()> {
        if self.device.is_grabbed() || self.state.get_key_state()?.iter().next().is_some() {
            return Ok(());
        }

        self.device.grab().context("Failed to grab keyboard")?;

        // Older queued events may already have reached the desktop. Treat the
        // ioctl interval conservatively as native too. Keep the grab afterward:
        // undoing it could split a captured press from its eventual release
        self.captured_since = Some(SystemTime::now());

        Ok(())
    }
}

impl Drop for Keyboards {
    fn drop(&mut self) {
        for keyboard in &mut self.devices {
            if let Err(error) = keyboard.device.ungrab() {
                eprintln!("failed to release keyboard: {error}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_keys_are_ignored_until_their_native_release() {
        let mut keys: AttributeSet<KeyCode> = [KeyCode::KEY_LEFTMETA, KeyCode::KEY_X]
            .into_iter()
            .collect();

        assert!(initial_key(
            &mut keys,
            &InputEvent::new(EventType::KEY.0, KeyCode::KEY_X.0, 2)
        ));
        assert!(!initial_key(
            &mut keys,
            &InputEvent::new(EventType::KEY.0, KeyCode::KEY_S.0, 1)
        ));
        assert!(initial_key(
            &mut keys,
            &InputEvent::new(EventType::KEY.0, KeyCode::KEY_X.0, 0)
        ));
        assert!(initial_key(
            &mut keys,
            &InputEvent::new(EventType::KEY.0, KeyCode::KEY_LEFTMETA.0, 0)
        ));
        assert!(keys.iter().next().is_none());
        assert!(!initial_key(
            &mut keys,
            &InputEvent::new(EventType::KEY.0, KeyCode::KEY_X.0, 1)
        ));
    }
}
