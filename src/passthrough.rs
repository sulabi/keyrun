use anyhow::Result;
use evdev::{AttributeSet, Device, InputEvent, KeyCode, uinput::VirtualDevice};
use std::{
    thread,
    time::{Duration, Instant},
};

const READY_TIMEOUT: Duration = Duration::from_millis(500);
const READY_POLL_INTERVAL: Duration = Duration::from_millis(10);

pub struct Passthrough {
    device: VirtualDevice,
    created: Instant,
    ready: bool,
}

impl Passthrough {
    pub fn new<'a>(keyboards: impl Iterator<Item = &'a Device>) -> Result<Self> {
        let keys = keyboards
            .filter_map(Device::supported_keys)
            .flat_map(|keys| keys.iter())
            .collect::<AttributeSet<KeyCode>>();

        let device = VirtualDevice::builder()?
            .name("sink-passthrough-kbd")
            .with_keys(&keys)?
            .build()?;

        Ok(Self {
            device,
            created: Instant::now(),
            ready: false,
        })
    }

    pub fn emit(&mut self, events: &[InputEvent]) -> Result<()> {
        // Mapped commands can run immediately. Wait for udev only when forwarding
        // keys, so the desktop has discovered the virtual keyboard before we emit

        if !self.ready {
            let path = self.device.get_syspath()?;

            while self.created.elapsed() < READY_TIMEOUT {
                if let Ok(device) = udev::Device::from_syspath(&path)
                    && device.is_initialized()
                {
                    break;
                }

                thread::sleep(READY_POLL_INTERVAL);
            }

            self.ready = true;
        }

        self.device.emit(events)?;

        Ok(())
    }
}
