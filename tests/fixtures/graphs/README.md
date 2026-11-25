# Graph fixtures (hardware layer 1)

`pw-dump` snapshots of real PipeWire graphs, used by the mocked-graph tests.
Each is scrubbed before commit: only Device, Node, Port, Link and the
`default`/`settings` metadata are kept; application stream nodes (and their
ports and links) are removed; `application.process.{host,machine-id,session-id,user}`
and `pipewire.sec.*` properties are dropped.

| File | Machine | Notes |
| --- | --- | --- |
| `asahi-m1-j293.json` | MacBook Pro 13" M1 (J293), Arch Linux ARM, Omarchy, PipeWire 1.6.9, WirePlumber 0.5.18, 2026-11-25 | Asahi DSP: `audio_effect.j293-convolver` (default sink) → `effect_output.j293-convolver` → hidden speaker node 61; hidden mic-array node 58 → `audio_effect.j293-mic` → `effect_output.j293-mic`; Omarchy's `omarchy_asahi_mic` loopback (default source). Nodes 58 and 61 are not visible to a normal client (links to them are), so JUNC cannot reach raw speakers even by mistake. Headphones/headset jack nodes are visible. |
| `snd-aloop-stereo.json` | same laptop, `snd-aloop` loaded (hardware layer 3) | Loopback card in its default `output:analog-stereo+input:analog-stereo` profile: 2-channel nodes; `EnumProfile` lists 65 profiles including `pro-audio` (index 64). The F15 "device exposes a stereo pair but has more channels" case. |
| `snd-aloop-proaudio.json` | same, after `wpctl set-profile <card> 64` | `pro-audio` profile: two 32-channel sinks and two 32-channel sources (`AUX0…AUX31`). |
| `komplete-audio-2-stereo.json`, `komplete-audio-2-proaudio.json` | same laptop, NI Komplete Audio 2 on USB | Class-compliant 2 in / 2 out. `pro-audio` exists but exposes the same 2 channels (`AUX0`, `AUX1`), so F15 must **not** offer "Use all channels" for it (negative case). |
| `bt-airpods-a2dp.json`, `bt-airpods-hfp.json` | same laptop, Apple AirPods | A2DP with AAC; HFP (`headset-head-unit`) with mSBC. In A2DP, WirePlumber 0.5 already exposes a virtual headset mic `bluez_input.<addr>` (with `bluez_capture_internal`) that switches the headset to HFP when captured: the F18 trigger. MAC address replaced with `00:00:5E:00:53:01`, owner name removed. |
| `bt-pixel-buds-2a-a2dp.json`, `bt-pixel-buds-2a-hfp.json` | same laptop, Google Pixel Buds 2a | A2DP with AAC; HFP with **LC3-SWB** (super-wideband), so "call quality" differs by codec and the F18 warning text should name the codec class. MAC replaced with `00:00:5E:00:53:02`. |
| `apps-browser-two-streams.json` | isolated PipeWire 1.6.9 instance on the same laptop (`bench/spikes/lib.sh`), null devices | Synthetic apps via `pw-cat` with real app properties: Firefox playing two streams (same `node.name`, `application.process.binary=firefox`), Spotify playing, Discord recording. Covers multi-stream matching and capture streams as Cable targets. |
