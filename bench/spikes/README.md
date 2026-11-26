# Alpha spikes

Throwaway experiments behind `docs/architecture.md` §12. Each runs in an
isolated PipeWire + WirePlumber + pipewire-pulse instance (`lib.sh`: own
`XDG_RUNTIME_DIR`, the CI null devices, no hardware monitors), so it never
touches the desktop session.

| Script | Spike |
| --- | --- |
| `s1-s2-visibility.sh` | S1/S2: which node classes PulseAudio, JACK and `wpctl` list as sinks/sources |
| `s1b-capture-routing.sh` | S1: whether WirePlumber links recording apps to each candidate class |
| `s3-link-tags.sh` | S3: custom `junc.*` props and `object.linger` on links after the creator exits |
| `s4-bit-exact.sh` (+ `s4_wav.py`, needs numpy) | S4: gain node + Junction bit-exact at 100 % and exact ×0.5 |
| `s5-exclusive.sh` | S5: Exclusive through the `target.object` metadata key |
| `s9-global-props.sh` | S9: which properties registry globals carry, and which need a bound proxy |
| `s10-rename.sh` | S10: renaming a node, and where recording apps go while a node is recreated |
| `s11-cable-target.sh` | S11: a Cable's target app hearing only the Cable through `target.object` |
| `s7-visibility.sh` | S7: the GUI meters stopping when the window leaves the screen on Hyprland (needs a Hyprland session; uses a temporary headless monitor) |
