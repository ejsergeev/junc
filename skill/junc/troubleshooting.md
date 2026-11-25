# Troubleshooting

Start with:

```sh
junc doctor --json
```

Each check has `status` (`ok`, `warn`, `missing`, `down`), a `detail`, and
for problems a `fix`: show it to the user and run it only if they agree
(`sudo` commands always need the user).

| Check | Problem | Fix |
| --- | --- | --- |
| `pipewire` | not running or older than 1.0 | `systemctl --user start pipewire`, or update |
| `wireplumber` | not running | `systemctl --user start wireplumber` |
| `pipewire-pulse` | missing | apps using PulseAudio cannot be routed: `sudo pacman -S pipewire-pulse` |
| `jack` | jack2 installed | JACK apps bypass PipeWire: `sudo pacman -S pipewire-jack` |
| `clock` | shows rate and quantum | latency ≈ quantum / rate |
| `juncd` | down | `systemctl --user start juncd`; config errors are shown here too |

## Common situations

- **`E_DAEMON_UNAVAILABLE`**: juncd is not running. `junc service status`
  tells whether it starts at login; `junc service enable` turns that on.
- **An Input stays `waiting`**: the app is not playing, or the device is
  unplugged, or the `match` names something that does not exist. Check
  `candidates` in `junc describe --json` for the exact name. Right after
  an apply, a status can lag for a moment while the new nodes appear;
  look again before concluding anything.
- **`link_overridden`**: another tool removed one of JUNC's links. JUNC
  does not put it back on its own; the user can restore it.
- **The config file has errors**: `junc status` and `junc doctor` show
  them with file, line and column; JUNC keeps using the last good config
  meanwhile. `junc config validate` checks the file.
- **`E_CONFIG_CONFLICT`**: the config changed since you read it. Read it
  again, redo the edit, dry-run, apply.
- **The skill is missing for an agent**: `junc skill status`, then
  `junc skill link`.
- **The user wants JUNC gone**: `junc teardown --yes` stops and disables
  juncd, removes every JUNC device and link, restores the defaults and
  unlinks this skill; the config stays. Ask first: every Junction stops.
- **After an update**: nothing to do. juncd restarts itself in place and
  audio keeps playing.

Logs: `journalctl --user -u juncd`.
