# Graph fixtures (hardware layer 1)

`pw-dump` snapshots of real PipeWire graphs, used by the mocked-graph tests.
Each is scrubbed before commit: only Device, Node, Port, Link and the
`default`/`settings` metadata are kept; application stream nodes (and their
ports and links) are removed; `application.process.{host,machine-id,session-id,user}`
and `pipewire.sec.*` properties are dropped.

| File | Machine | Notes |
| --- | --- | --- |
| `asahi-m1-j293.json` | MacBook Pro 13" M1 (J293), Arch Linux ARM, Omarchy, PipeWire 1.6.9, WirePlumber 0.5.18, 2026-11-25 | Asahi DSP: `audio_effect.j293-convolver` (default sink) → `effect_output.j293-convolver` → hidden speaker node 61; hidden mic-array node 58 → `audio_effect.j293-mic` → `effect_output.j293-mic`; Omarchy's `omarchy_asahi_mic` loopback (default source). Nodes 58 and 61 are not visible to a normal client (links to them are), so JUNC cannot reach raw speakers even by mistake. Headphones/headset jack nodes are visible. |
