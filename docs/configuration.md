# keyrun configuration

Configuration is read from `~/.config/keyrun/config.toml` in TOML format.
Optional settings use their defaults when omitted.

For language-server completion, copy [config.schema.json](../assets/config.schema.json)
next to your config and put `#:schema ./config.schema.json` on the first line.

## Main section

These options belong at the top of the file, before any `[section]`.

### timeout_seconds

Idle timeout in seconds. Accepts a finite number greater than zero, including
fractional seconds. The timer resets after each accepted keypress. Forwarded keys
are allowed to release before the listener exits.

Default: `1.0`.

## [keys.<key>]

Define one section per key, for example `[keys.f]`. Key names are single
characters, normally lowercase letters or digits. Each accepted press runs the
binding's command. Arguments are passed directly, without shell expansion.

The first accepted key starts a session. Unmapped keys before that pass through
normally. During a session, a key outside the allowed keys ends the session and
passes through, including modifiers.

At startup, a keyboard with held keys stays ungrabbed until those keys are
released, preserving the desktop's press/release pairs. Keys already held when
the device opens are ignored as launcher input. New presses observed before the
grab may still run configured commands, but their native events are not replayed
or suppressed. The grab handoff remains best effort: presses during the handoff
or before the device opens can still be missed or reach the desktop.

### app

Executable name or path.

Required. No default.

### args

Array of command arguments, for example `["set", "+5%"]`.

Default: no arguments.

### env

Array of environment variable name/value pairs, for example
`[["NAME", "value"], ["OTHER", "value"]]`. These override or extend the inherited
environment.

Default: inherit the environment without overrides.

### times

Press limit for the session, including the starting press. Accepts a positive
integer or `"infinite"`. The starting binding sets the limit shared by its linked
keys; those keys still run their own commands.

`"infinite"` removes the press limit. The idle timeout and unrelated-key exit
behavior still apply. Holding a key does not count as additional presses.

Default: `1`.

### links_with

Array of other configured keys allowed in the same session, for example `["d"]`.
Only the starting binding's direct links apply. List links on both bindings to
allow starting from either key. Every linked key must have a configured command.

Default: `[]`.

### pipe

Nested command receiving this command's output. Uses the same command options
and can be nested again to extend a pipeline.

For example:

```toml
[keys.c]
app = "printf"
args = ["hello"]

[keys.c.pipe]
app = "wl-copy"
```

Default: no pipe.

## [overlay]

Used when running with `--overlay`. Build with `--features overlay` to enable it.
Window placement and compositor borders are controlled by your window manager.

Consecutive presses of the same accepted key update one notification. Finite
sessions show a red remaining-press countdown such as `(↓4)`; infinite sessions
show a repeat count such as `(x2)`.

### window_background_opacity

Opacity of the whole window's background. Accepts a number from `0.0`
(transparent) to `1.0` (opaque). Does not dim notification text.

Default: `0.0`.

### row_background_opacity

Opacity of each notification's background. Accepts a number from `0.0`
(transparent) to `1.0` (opaque). Does not dim notification text.

Default: `1.0`.

### row_borders

Show colored notification borders. Accepts `true` or `false`. This does not
control the window manager's outer border or focus ring.

Default: `true`.

### show_command

Show the executable and arguments beneath the key in smaller text. Accepts
`true` or `false`. Long commands are truncated.

When disabled, command text and the finite countdown are hidden. Infinite repeat
counts remain beside the key.

Default: `true`.
