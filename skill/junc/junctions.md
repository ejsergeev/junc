# Junctions in the config

`junc config path` prints where the file is (normally
`~/.config/junc/junctions.toml`). Comments and layout are kept when JUNC
edits it. The full JSON Schema: `junc schema`.

```toml
schema = 1

[[junction]]
id = "zoom-mic"              # stable id: lowercase letters, digits, "-"
name = "Zoom Mic"            # what apps show; any text
kind = "junction"            # or "cable"
enabled = true               # off keeps the config, routes nothing
volume = 100                 # percent, 0–150
default = "none"             # "input" or "output" makes it the system default
rate = "follow"              # or a fixed rate, 44100–192000
bus = { channels = 2 }       # 1–64

[[junction.input]]
id = "spotify"
match = { binary = "spotify" }
name = "Spotify"             # optional: what JUNC shows, also while the source is away
enabled = true
volume = 70
exclusive = false            # true: the app is no longer heard on the speakers (apps only)
links = [[1, 1], [2, 2]]     # [input channel, Bus channel]; omit for the default

[[junction.listen]]
id = "hp"
target = "default"           # or an output node name
volume = 100
links = [[1, 1], [2, 2]]     # [Bus channel, output channel]; omit for the default
```

Every key but `id`, the Junction's `name` and `match` can be left out;
the defaults are shown. Without an Input `name`, JUNC shows what the
source calls itself.

## Inputs: `match`

Exactly one kind per Input:

| Kind | Keys | Example |
| --- | --- | --- |
| App | `binary`, `app` (application name), `role`, `node` | `{ binary = "spotify" }` |
| Hardware input | `device` (node name) | `{ device = "alsa_input.usb-…" }` |
| What an output plays | `monitor` (output node name) | `{ monitor = "alsa_output.pci-…" }` |
| Another Junction | `junction` (its id) | `{ junction = "zoom-mic" }` |

An app that is not running yet is fine: the Input waits and is routed when
the app starts, every time it starts.

## Links

Without `links`, JUNC maps channels by default: a mono source feeds Ch 1
and Ch 2; otherwise channel k feeds channel k as far as both go.
`junc status NAME --json` shows the links in use (`links`) and whether
they are custom (`custom_links`). Direct commands:

```sh
junc link zoom-mic mic:1 2
junc unlink zoom-mic mic:1 2
junc link zoom-mic 1 hp:2
junc bus add zoom-mic --count 1
junc bus remove zoom-mic 4
```

`mic:1` is channel 1 of the Input `mic`; a bare number is a Bus channel;
`hp:2` is channel 2 of the Listen output `hp`.

## Reading a plan

`junc apply FILE --dry-run --json` → `data.plan`:

- `summary`: one sentence per change; show these to the user.
- `actions`: what the next step does to the audio graph (nodes and links,
  by name). Usually you do not need them.
- `complete`: `false` when new Junctions are created first and linked a
  moment later; the summary already describes the whole change.
- `deferred`: renames that wait until no app uses the Junction (apps keep
  the old name until then).
- `junctions`: each Junction as it would be, with per-Input `status`,
  `matched` nodes and the `links` in use (`suppressed` lists links the user
  chose to keep removed).

## Cables

`kind = "cable"` with exactly one app Input and the app to deliver to:

```toml
schema = 1

[[junction]]
id = "browser-to-discord"
name = "Browser to Discord"
kind = "cable"

[[junction.input]]
id = "firefox"
match = { binary = "firefox" }

[junction.cable]
to = { binary = "Discord" }
```

JUNC points the target app's recording at the Cable, so the app hears the
Cable instead of its mic. If the user moves that app elsewhere afterwards,
JUNC leaves it there.
