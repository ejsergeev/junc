# Apps and devices

`junc describe --json` lists them under `data.candidates`:

| `kind` | Use as | Key to copy |
| --- | --- | --- |
| `app_playback` | an Input (the app's sound) | `binary`, or `app` |
| `app_capture` | a Cable target (an app that records) | `binary`, or `app` |
| `source` | an Input (a mic or line in) | `node` → `match.device` |
| `sink` | a Listen target, or an Input via `monitor` | `node` |
| `junction` | an Input of another Junction | its id (`node` is `junc.<id>`) |

Prefer `binary` for apps: it stays the same across restarts and versions.
`description` is the human name to show the user.

Apps appear only while they make or record sound; ask the user to start
playback if an app is missing, or match it by binary anyway: the Input
waits for it.

## Status of each Input

`junc status NAME --json` shows per Input `status`:
`active` (routed), `waiting` (app or device not there now), `disabled`
(turned off), `unknown` (the daemon has no PipeWire graph yet), and in
`matched` the nodes it found.

## Bluetooth headsets

A headset's microphone only exists in its call profile (HFP/HSP), which
lowers playback quality; in high-quality playback (A2DP) there is no mic.
Tell the user this before using a Bluetooth mic.

## Apple Silicon (Asahi Linux)

The built-in speakers are reached only through the system's speaker
processing; their raw device nodes are hidden. Use `target = "default"`
for Listen outputs to the built-in speakers; never a raw `alsa_output`
node of the internal speakers.
