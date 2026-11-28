---
name: junc
description: Route audio on Linux with JUNC (PipeWire). Use for virtual microphones ("make a mic for Zoom with my USB mic and Spotify"), routing one app's audio into another app, combining or mixing inputs, sending a mix to headphones, separate tracks for streaming, the default input or output, saving or sharing a setup as a preset, templates, and anything about junc, juncd, Junctions or Cables.
---

# JUNC

JUNC builds virtual audio devices on PipeWire. Apps see each one as a normal
microphone and speaker. Everything goes through the `junc` command; the
daemon `juncd` keeps the routing alive while apps and devices come and go.

## Words

- **Junction**: a virtual device. Apps record from it like a mic and can play
  into it like a speaker.
- **Input**: what feeds a Junction: an app (by binary), a hardware input
  (by node name), what an output plays (monitor), or another Junction.
- **Bus**: the Junction's channels (2 by default; Ch 1–2 is stereo).
- **Link**: one channel of an Input to one Bus channel. Without explicit
  links JUNC uses the default mapping (mono feeds Ch 1 and 2).
- **Listen**: an output where you hear the Junction (`default` follows the
  system output).
- **Cable**: one app straight into another app.
- **Exclusive**: the app's audio goes only into the Junction, not to the
  speakers too (app Inputs only).

## First, always

```sh
junc describe --json
```

It returns the daemon state, every Junction with its status, the default
devices, and `candidates`: the apps and devices you can use, with the exact
`node`, `app` and `binary` values to put in a config. Use those values;
never guess node names. Match the user's words ("my USB mic", "the Blue
Yeti") against each candidate's `description`; if more than one could fit,
ask which one they mean. If there is only one candidate of the right kind,
name it in the plan ("your mic, FakeMic") so the user can object.

If it fails with `E_DAEMON_UNAVAILABLE`, run `junc doctor` and follow its
`fix` lines (see troubleshooting.md).

## The safe change loop

Never edit `junctions.toml` in place: the daemon applies every saved edit at
once. Work on a copy:

```sh
edit=$(mktemp --suffix=.toml)
cfg=$(junc config path)
if [ -f "$cfg" ]; then cp "$cfg" "$edit"; else printf 'schema = 1\n' > "$edit"; fi
junc config validate "$edit" --json
junc apply "$edit" --dry-run --json
junc apply "$edit" --expect-revision 7 --json
```

1. Copy the config to a fresh temporary file (no file yet means no
   Junctions yet: start from `schema = 1`). Edit the copy. Shell variables
   do not survive between separate tool calls: note the file's path.
2. Validate it. On errors, read `code`, `location` and `hint`, fix, repeat.
3. Dry-run it. Show the user the `data.plan.summary` lines in plain words.
   Note the top-level `revision` of that answer (not one inside `data`).
4. Only after the user agrees, apply with `--expect-revision` set to that
   number (7 above is only an example). If it fails with
   `E_CONFIG_CONFLICT`, someone else changed the config: start again.

When you show the plan, also say what does not change if the user cares
(the summary lists default-device changes only when there are some), and
for each app you capture, that you capture it (see the safety rules).

Config format and examples: junctions.md and recipes.md. The JSON Schema:
`junc schema`.

## Templates first

For a new Junction or Cable that a built-in template covers (a mic plus an
app, app to app, combining inputs, separate tracks, a stream mix), use the
template: `junc template list --json`, then `junc create junction NAME
--template T --slot ID=VALUE`. See presets.md, which also covers exporting
and importing presets.

## Direct commands

Small, obvious changes may use direct commands instead (`junc enable`,
`junc volume`, `junc input add`, …); say what will happen first. They apply
immediately; there is no dry run for them.

## Safety rules

- Never delete a Junction, change the default device, or capture an app the
  user did not name, without asking first.
- Always validate and dry-run before applying; show the planned change in
  plain words.
- Capturing an app's audio shows up in the capture indicator; say so when
  you capture an app ("JUNC will capture Spotify's audio for Zoom Mic; you
  still hear it on your speakers"). `junc status` lists captured apps.
- Edit only `~/.config/junc/`. Never touch PipeWire or WirePlumber configs.
- Destructive commands need `--yes` (`junc delete`); pass it only after the
  user confirmed in the conversation.

## Omarchy

`junc omarchy install` adds the JUNC bar module (Junctions on, the capture
dot, a switch for each) and an "Audio routing · JUNC" row under Setup in
the Omarchy menu; `junc omarchy uninstall` takes both out again and
`junc omarchy status` shows what is there. It changes the user's desktop:
ask first. JUNC binds no key; if the user wants one, suggest Super+Shift+J
for `junc-gui` (free in Omarchy's defaults) and let them add it.

## Reading results

Every command with `--json` prints one document:
`{"junc": 1, "ok": true, "revision": N, "data": {…}}` or
`{"junc": 1, "ok": false, "error": {"code", "message", "hint", "location"}}`.
Exit codes and every error code: reference.md. What a plan contains:
junctions.md.

## In this version

- `volume` (Junction, Input, Listen) changes loudness at once, without
  touching the app's own volume. JUNC owns these volumes: a change made in
  another mixer to a JUNC device is not saved and is replaced by the
  config's value the next time JUNC sets it.
- `exclusive` takes the app off the speakers while the Junction captures
  it, and gives it back when turned off. If the user then moves the app to
  another output themselves, JUNC reports it and does not move it back.
- Templates and presets: presets.md. A template makes the Junction at once
  (no dry run); for anything it does not cover, write the config from
  recipes.md.

## More

- presets.md: templates, and exporting and importing presets.
- recipes.md: Zoom mic, stream mix, app to app, combine inputs.
- junctions.md: the config format in detail.
- devices.md: finding apps and devices; Bluetooth and Apple Silicon notes.
- defaults.md: default input and output.
- troubleshooting.md: `junc doctor` and common problems.
- reference.md: every command, flag, JSON shape and error code (generated).
