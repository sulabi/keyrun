# keyrun

`keyrun` is a key based app launcher, it grabs your keyboard and waits for next key, to map it to a configured application.

## Configuration

`keyrun` reads a config file from `~/.config/keyrun/config.toml`. The config is written in a toml format for clear mappings.

`timeout_seconds` sets how long to wait for a key, starting after keyboard setup. (default: 1)

`times` accepts a positive press count or `"infinite"`. It defaults to `1`.
`links_with` lists other configured keys allowed in the same session. The starting
binding sets the shared press limit and allowed keys, links are not transitive.

### Config example
```toml
timeout_seconds = 1

# Single app
[keys.f]
app = "firefox"

# Example with args
[keys.k]
app = "notify-send"
args = ["This", "example", "-t", "1000"]

# Linked volume controls
[keys.9]
app = "setvolume"
args = ["dec", "10"]
times = "infinite"
links_with = ["0"]

[keys.0]
app = "setvolume"
args = ["inc", "10"]
times = "infinite"
links_with = ["9"]
```

## Note
This requires your user to be apart of the `input` group in order for the `evdev` crate to grab your keyboard.
You can add your current user to the group with
```bash
sudo usermod -aG input $USER
```

Then just log back in for the group permissions to take effect.
