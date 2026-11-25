# Templates and presets

A **template** is a ready-made Junction or Cable with blanks (slots) to
fill: "Mic + App" asks for a mic and an app. A **preset** is one Junction
saved to a file, to share it or bring it to another computer. Both use the
same file format; a template is a preset with slots.

## Templates first

When the user asks for something a template covers, use it instead of
writing the config by hand:

```sh
junc template list --json
junc template show mic-plus-app
```

| Template | Makes | Slots |
| --- | --- | --- |
| `mic-plus-app` | a mic and one app mixed into one input (calls) | `mic` hardware input, `app` app |
| `cable` | one app straight into another (Cable) | `from` app, `to` app |
| `combine-inputs` | two hardware inputs mixed into one | `first`, `second` hardware inputs |
| `separate-tracks` | mic on Ch 1–2, an app on Ch 3–4 (recording) | `mic` hardware input, `app` app |
| `stream-mix` | mic plus what an output plays | `mic` hardware input, `desktop` monitor |
| `start-empty` | an empty Junction | none |

Fill each slot with a value from `junc describe --json` `candidates`: a
hardware input or output by its `node`, an app by its `binary`, a monitor
by the output's `node`, or `default` for an output slot.

```sh
junc create junction "Zoom Mic" --template mic-plus-app --slot mic=alsa_input.usb-Blue_Yeti-00.analog-stereo --slot app=spotify --json
```

This adds one Junction at once and changes nothing else (it never takes
over the default device). It is a direct command: before running it, tell
the user what it makes and which apps it captures ("a Zoom Mic with your
Blue Yeti and Spotify; JUNC will capture Spotify's audio"). A missing or
unknown slot fails with `E_PRESET_FILL`; its hint lists the slots.

To change a template's result (volumes, links, a Listen output), create it,
then edit as usual (direct commands, or the safe change loop in SKILL.md).

## Export and import

```sh
junc preset export "Zoom Mic" -o zoom-mic.toml
junc preset import zoom-mic.toml --json
junc preset import zoom-mic.toml --name "Zoom Mic (laptop)"
```

- Export writes the whole Junction. A comment at the top names the Inputs
  and Listen outputs tied to this computer's devices; on another computer
  they wait (`waiting`) until a device with the same node name appears.
  With `--json`, `data.device_specific` lists them.
- Import adds a new Junction with an id made from its name (`zoom-mic-2`
  if `zoom-mic` exists). If another Junction already is the default input
  or output, the imported one is not made default; the result's `notes`
  say how to switch.
- `junc preset import` also takes a template file, with `--slot` values.
- Import a file only when the user gave it to you or asked for it, and say
  what it captures, as for any change.

## The file format

```toml
schema = 1

[preset]
name = "Mic + App"
description = "Your mic and one app mixed into one input."
example = "e.g. USB Microphone + Spotify"

[[preset.slot]]
id = "mic"
kind = "hardware-input"            # hardware-input | app | output | monitor
prompt = "Pick a mic"

[[preset.slot]]
id = "app"
kind = "app"
prompt = "Pick an app"

[junction]
name = "{app} + {mic}"

[[junction.input]]
id = "mic"
match = { slot = "mic" }

[[junction.input]]
id = "app"
match = { slot = "app" }
```

- `[junction]` is one Junction or Cable, as in junctions.md; `id` may be
  left out (import makes one from the name).
- A slot is used as `match = { slot = "…" }` on an Input (hardware-input,
  app or monitor; a Cable's Input and `cable.to` take app slots), as
  `target = "{…}"` on a Listen output (output slots), and as `{…}` in the
  name, which becomes the device's description or the app's name.
- Every slot must be used. Mistakes fail with `E_PRESET_SLOT` and the line
  of the problem; config rules (`E_CONFIG_…`) apply as in junctions.md.
- Slots are only for presets: `junctions.toml` refuses them
  (`E_CONFIG_SLOT_OUTSIDE_PRESET`).
