# Recipes

Each recipe is a whole config to merge into the copy you are editing (keep
the user's existing Junctions), then validate, dry-run and apply as in
SKILL.md. Replace the `device` and `binary` values with the ones
`junc describe --json` lists under `candidates`. An app shows up there
only while it plays; if it is not running, ask the user to start it once,
or use its usual binary name and check after applying that its Input
turns `active` when the app plays (Flatpak and Snap builds can use other
names).

## A mic for calls: hardware mic plus an app

"Make a Zoom mic with my USB mic and Spotify, Spotify a bit quieter."

```toml
schema = 1

[[junction]]
id = "zoom-mic"
name = "Zoom Mic"

[[junction.input]]
id = "mic"
match = { device = "alsa_input.usb-Blue_Yeti-00.analog-stereo" }

[[junction.input]]
id = "spotify"
match = { binary = "spotify" }
volume = 70
```

`volume` is a percentage of full level (100 = unchanged); 70 is a
moderate cut (the scale is cubic, like PulseAudio's: 70 % is about −9 dB).
In the call app, pick "Zoom Mic" as the microphone. To have every app use
it, ask the user first, then add `default = "input"` to the Junction.

## Hear the mix yourself

Add a Listen output to any Junction: `default` follows the system output.

```toml
schema = 1

[[junction]]
id = "stream"
name = "Stream Mix"

[[junction.input]]
id = "game"
match = { binary = "steam" }

[[junction.listen]]
id = "headphones"
target = "default"
```

## One app straight into another (Cable)

"Send Spotify into OBS." OBS's own input picker then shows the Cable, and
OBS hears only the Cable, not the mic.

```toml
schema = 1

[[junction]]
id = "spotify-to-obs"
name = "Spotify to OBS"
kind = "cable"

[[junction.input]]
id = "spotify"
match = { binary = "spotify" }

[junction.cable]
to = { binary = "obs" }
```

## Separate tracks for recording

A four-channel Junction: the mic on Ch 1–2, the game on Ch 3–4.

```toml
schema = 1

[[junction]]
id = "tracks"
name = "Tracks"
bus = { channels = 4 }

[[junction.input]]
id = "mic"
match = { device = "alsa_input.usb-Blue_Yeti-00.analog-stereo" }
links = [[1, 1], [1, 2]]

[[junction.input]]
id = "game"
match = { binary = "steam" }
links = [[1, 3], [2, 4]]
```

## A Junction inside another

Nesting reuses a mix: here the "Zoom Mic" from the first recipe also feeds
the stream. JUNC refuses loops (`E_CONFIG_CYCLE`).

```toml
schema = 1

[[junction]]
id = "zoom-mic"
name = "Zoom Mic"

[[junction.input]]
id = "mic"
match = { device = "alsa_input.usb-Blue_Yeti-00.analog-stereo" }

[[junction]]
id = "stream"
name = "Stream Mix"

[[junction.input]]
id = "voice"
match = { junction = "zoom-mic" }

[[junction.input]]
id = "desktop"
match = { monitor = "alsa_output.pci-0000_00_1f.3.analog-stereo" }
```

## Quick changes without a config edit

These apply at once; say what will happen first.

```sh
junc create junction "Podcast"
junc input add podcast --device alsa_input.usb-Blue_Yeti-00.analog-stereo --id mic
junc input add podcast --app firefox --exclusive
junc volume podcast firefox 60
junc listen add podcast default
junc disable podcast
junc enable podcast
junc status podcast
```
