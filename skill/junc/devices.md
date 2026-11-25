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

## More channels: Pro Audio

Many audio interfaces show only a stereo pair until their card runs the
Pro Audio profile. `data.devices` in `junc describe --json` lists each
card with its `profile`, `input_channels` and `channel_groups` (groups of
eight, as the app shows them). A card with `pro_audio` has more inputs in
Pro Audio: `{"now": 2, "all": 18}`, or `"all": null` when only
"available" is known. Ask the user before switching: it renames the
card's devices and other apps using them lose them.

```sh
junc input add studio --device alsa_input.usb-Focusrite_Scarlett_18i8-00.analog-stereo --pro-audio
```

This saves `profile = "pro-audio"` with the device match. JUNC switches
the card now and every time it is plugged in, and the Input takes the
card's widest Pro Audio input (status `waiting` until the switch shows
up). If the user picks another profile elsewhere while the card stays
plugged in, JUNC keeps their choice and saves it as the Input's
`profile`; a card turned off is left off. Default links take the first
Bus channels; use `junc link` for the others. Built-in audio set up by
UCM (Apple Silicon included) never gets Pro Audio: it would bypass the
speaker protection.

## Bluetooth headsets

A headset's microphone works only in its call profile (HFP/HSP), which
lowers playback quality. In high-quality playback (A2DP) the mic is still
listed (`bluez_input.<address>`), and recording from it makes the system
switch the headset to the call profile.

Candidates and `devices` carry `bluetooth`: `profile`, `mode`
(`playback` or `call`), `call_codec` and `call_quality` (`narrowband`,
`wideband`, `super-wideband`). When `switches_to_call` is true, tell the
user before adding the mic: their music or call audio in the headset
drops to call quality while the Junction uses it. Offer another mic if
they have one. `junc input add` repeats the warning (`"warning":
"bluetooth_call_profile"` in JSON). JUNC never switches the headset's
profile itself.

## Apple Silicon (Asahi Linux)

The built-in speakers are reached only through the system's speaker
processing; their raw device nodes are hidden. Use `target = "default"`
for Listen outputs to the built-in speakers; never a raw `alsa_output`
node of the internal speakers.
