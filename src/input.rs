use crate::config::KeyCommand;
use anyhow::Result;
use evdev::{AttributeSet, Device, EventType, KeyCode, uinput::VirtualDevice};
use std::{
    collections::HashMap,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};
use udev::Device as UDevDevice;

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

pub(crate) fn listen(config_keys: &HashMap<char, KeyCommand>, timeout: Duration) -> Result<()> {
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

    crate::session::listen(&rx, config_keys, timeout, |batch| {
        vdev.emit(batch).map_err(Into::into)
    })
}
