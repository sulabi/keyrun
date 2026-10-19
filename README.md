# keyrun

A key based app launcher for Linux. It grabs your keyboard and runs commands for
configured keys.

## Build

```bash
cargo build --release
./target/release/keyrun
```

## Configuration

Create `~/.config/keyrun/config.toml`:

```toml
[keys.f]
app = "firefox"

[keys.u]
app = "wpctl"
args = ["set-volume", "@DEFAULT_AUDIO_SINK@", "5%+"]
times = "infinite"
links_with = ["d"]

[keys.d]
app = "wpctl"
args = ["set-volume", "@DEFAULT_AUDIO_SINK@", "5%-"]
times = "infinite"
links_with = ["u"]
```

See the [configuration reference](docs/configuration.md) for every option.

## Overlay

```bash
cargo build --release --features overlay
./target/release/keyrun --overlay
```

On **Hyprland** and **niri**, use the window rules below to position and style the overlay.

### niri

Add to `~/.config/niri/config.kdl`:

```kdl
window-rule {
    match title="^keyrun overlay$"

    open-floating true
    open-focused false
    default-floating-position x=16 y=16 relative-to="bottom"
    draw-border-with-background false

    border { off; }
    focus-ring { off; }
    shadow { off; }
}
```

See [niri window rules](https://github.com/niri-wm/niri/wiki/Configuration:-Window-Rules).

### Hyprland

For Hyprland 0.55+ using Lua, add to `~/.config/hypr/hyprland.lua`:

```lua
local positions = {
    top_left = { 16, 16 },
    top = { "(monitor_w - window_w) / 2", 16 },
    top_right = { "monitor_w - window_w - 16", 16 },

    left = { 16, "(monitor_h - window_h) / 2" },
    centre = { "(monitor_w - window_w) / 2", "(monitor_h - window_h) / 2" },
    right = { "monitor_w - window_w - 16", "(monitor_h - window_h) / 2" },

    bottom_left = { 16, "monitor_h - window_h - 16" },
    bottom = { "(monitor_w - window_w) / 2", "monitor_h - window_h - 16" },
    bottom_right = { "monitor_w - window_w - 16", "monitor_h - window_h - 16" },
}

hl.window_rule({
    name = "keyrun-overlay",
    match = { title = "^keyrun overlay$" },

    float = true,
    no_initial_focus = true,
    no_focus = true,

    decorate = false,
    no_shadow = true,
    no_blur = true,

    move = positions.bottom,
})
```

See [Hyprland window rules](https://wiki.hypr.land/Configuring/Basics/Window-Rules/)
for your version's syntax.

Choose a position with `move = positions.bottom`, `positions.top_right`, etc.
Edge positions use a 16-pixel gap; adjust `16` for panels or more spacing.

### Other window managers

Rules depend on your window manager. X11 transparency requires a compositor.

## Permissions

Your user needs keyboard access through the `input` group:

```bash
sudo usermod -aG input "$USER"
```

Log out and back in for the group change to take effect.
