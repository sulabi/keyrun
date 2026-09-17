# keyrun

`keyrun` is a key based app launcher, it grabs your keyboard and waits for next key, to map it to a configured application.

## Configuration

`keyrun` reads a config file from `~/.config/keyrun/config.toml`. The config is written in a toml format for clear mappings.

### Config example
```toml

# Single app
[keys.f]
app = "firefox"

# Example with args
[keys.k]
app = "notify-send"
args = ["This", "example", "-t", "1000"]
```

## Note
This requires your user to be apart of the `input` group in order for the `evdev` crate to grab your keyboard.
You can add your current user to the group with
```bash
sudo usermod -aG input $USER
```

Then just log back in for the group permissions to take effect.
