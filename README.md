# keyrun

`keyrun` is a key based app launcher for Linux. The launcher grabs your keyboard and maps your key to a configured command.

Configure `~/.config/keyrun/config.toml`:

```toml
[keys.f]
app = "firefox"

# Repeat brightness adjustments up to ten times
[keys.b]
app = "brightnessctl"
args = ["set", "+5%"]
times = 10

# Switch between volume up/down until idle
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

- `times` defaults to `1`; use a positive integer or `"infinite"`.
- The first mapped key sets the session's press limit and allowed `links_with` keys.
  Each press runs its own command. List links on both sides to start from either key.
- An unrelated key ends the session and passes through normally, including modifiers.
  The listener waits for forwarded keys to be released so shortcuts work.
- The listener also exits after two seconds without a mapped press, once forwarded
  keys are released. Unmapped keys before a session starts pass through normally.

Your user needs keyboard access through the `input` group:

```bash
sudo usermod -aG input "$USER"
```

Log out and back in for the group change to take effect.
