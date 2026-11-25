# Hardware matrix

What each hardware layer (`architecture.md` §11.1) can cover on which machine,
and the measured results. Raw outputs live next to the bench scripts
(`bench/validation/bench/results-m1-*`).

## Machines

| Machine | Arch | OS / session | Audio stack | Used for |
| --- | --- | --- | --- | --- |
| Owner laptop: MacBook Pro 13" M1 (J293), 8 cores | aarch64 | Arch Linux ARM, Omarchy (quattro), Hyprland, kernel 7.2.11-14-1-ARCH | PipeWire 1.6.9, WirePlumber 0.5.18, pipewire-pulse, pipewire-jack (since 2026-11-25), Asahi DSP (`j293-convolver`, `j293-mic`), speakersafetyd | layers 2, 3, 5; GUI on GPU; Orca |
| CI (planned) | x86_64 + aarch64 | Arch / Arch Linux ARM containers | headless PipeWire + WirePlumber, null devices | layers 1, 2 |
| Validation VM (2026-11-24, reference only) | x86_64 | Ubuntu 24.04, 2 vCPU, no sound card | PipeWire 1.0.5, WirePlumber 0.4.17 | prototype numbers |

## Layers on the owner laptop (step 0, 2026-11-25)

| Layer | Status | Evidence |
| --- | --- | --- |
| 1 · Mocked graph | available | fixtures `tests/fixtures/graphs/asahi-m1-j293.json`, `snd-aloop-{stereo,proaudio}.json` |
| 2 · Virtual PipeWire devices | available | isolated PipeWire instance (own `XDG_RUNTIME_DIR`, WirePlumber 0.5 profile with all hardware monitors disabled) runs beside the desktop session without touching it |
| 3 · Virtual kernel cards | available | `snd-dummy` and `snd-aloop` load (sudo); WirePlumber exposes them with ACP profiles; Loopback has 65 profiles incl. `pro-audio` with 32-channel sinks/sources; Dummy has `pro-audio` too. Unloading needs WirePlumber stopped (it holds the control devices), so hot-plug tests switch the card profile `off`/on instead of `modprobe -r` |
| 4 · Emulated USB audio (UAC2 gadget) | **skipped** (owner, 2026-11-25) | `libcomposite`, `usb_f_uac2`, `g_audio` exist, but `dummy_hcd` is not built and `/sys/class/udc` is empty |
| 5 · Real devices | captured (task 0.7) | NI Komplete Audio 2: stereo profile and `pro-audio`, both 2 channels (F15 negative case). AirPods: A2DP/AAC, HFP/mSBC. Pixel Buds 2a: A2DP/AAC, HFP/LC3-SWB. In A2DP, WirePlumber 0.5 exposes a virtual headset mic (`bluez_input.<addr>`) that switches the headset to HFP when captured. Built-in audio behind the Asahi DSP. No HDMI audio. Fixtures in `tests/fixtures/graphs/` |

Asahi note: the raw speaker and mic-array ALSA nodes are not visible to a
normal PipeWire client (only links to them are), so JUNC cannot route around
the DSP by accident. Listen targets are the DSP sink (`audio_effect.j293-convolver`)
or the headphone jack.

## Daemon prototype (`bench/juncd`, `juncd-links` mode)

Isolated = own PipeWire instance with null devices, real kernel and CPU,
rtkit. Live session = the owner's desktop session (systemd user units,
Asahi DSP chains, desktop apps running), with the null devices added by a
temporary `pipewire.conf.d` drop-in that was removed afterwards; PipeWire
restarts go through `systemctl --user restart pipewire wireplumber`.

| Metric | Target | M1 isolated, q1024 | M1 isolated, q256 | M1 live session, q1024 | VM 2026-11-24 (q1024) |
| --- | --- | --- | --- | --- | --- |
| Build full graph | — | 34 ms | 39 ms | 102 ms | 39 ms |
| Added latency | 0 extra quanta | **0** (`jack_iodelay` 1024 = 1024; impulse 0 = 0) | **0** (`jack_iodelay` 256 = 256; impulse 0 = 0) | **0** (`jack_iodelay` 1024 = 1024; impulse 0 = 0) | 0 (impulse only) |
| ERR on JUNC/device nodes, 10 min | 0 | **0** (apps 0 too) | **0** on JUNC/device nodes (5 min); fake apps 264–301 each, see note | **0** (5 min; apps 0 too) | 0 (apps 12) |
| Daemon PSS idle | < 10 MB | **1.7 MB** | **1.7 MB** | **1.8 MB** | 1.7 MB |
| PSS after 10 min app churn | flat | 1696 → 1696 kB (end 1776 kB after 10 PipeWire restarts) | 1696 → 1712 kB after 5 min churn (end 1760 kB) | 1787 → 1786 kB after 5 min churn (end 1787 kB) | 4.9 → 5.0 MB (loopback mode) |
| Idle wakeups | ~0/s | **0.00/s** | **0.00/s** | 0.10/s (the desktop's own graph events) | 0.00/s |
| Re-link (app appears → linked) | < 100 ms | p50 7.4, p95 22.0, max 22.0 ms (n = 13) | p50 12.1, p95 19.1, max 19.1 ms (n = 7) | p50 19.4, **p95 89.1**, max 89.1 ms (n = 7) | p50 5.1, max 7.4 ms (n = 3) |
| Recovery after PipeWire restart | < 1 s | **388–453 ms** (n = 10) | **396–438 ms** (n = 5) | **303–446 ms** (n = 5) | 368–408 ms |

Note on the live session: re-link p95 of 89 ms is close to the 100 ms target.
The prototype re-links on each registry event without batching, and a real
desktop produces many more events than the isolated instance. Plan task A7
(reconcile at the end of a `core.sync` batch) must bring p95 well under the
target in the live session, measured again in A19.

Note on quantum 256: the three fake app clients are `pw-cat --latency 1024`
processes forced to a 256-frame quantum; their ERR counters grow while the
Junction and device nodes stay at 0. The VM A/B test showed such client ERRs do
not depend on who creates the links; plan task A17 repeats that A/B at
quantum 256 on the M1 before treating it as settled.

Manual reference graph (pw-cli + pw-link + 2× pw-loopback), M1 q1024: 0 extra
quanta, 0 ERR, helpers 9.2 MB PSS and 93.8 wakeups/s, re-link via
WirePlumber 38–51 ms, and it does not recover from a PipeWire restart by
itself (`NA` × 5), as in the VM.

## Slint prototype (`bench/validation/proto`, Hyprland, Apple GPU)

| Metric | Target | femtovg (GPU) | software | VM software |
| --- | --- | --- | --- | --- |
| Cold start | < 300 ms | **88 ms** median (77–498, n = 10) | 24 ms to first loop turn | 51 ms |
| PSS, meters on | < 80 MB | **62 MB** | 26 MB | 24 MB |
| CPU, meters at 7 Hz, visible | < 2 % of a core | **6.4 %** ❌ | 14.1 % ❌ | 3.3 % |
| Wakeups/s, visible | — | 147 | 171 | — |
| CPU, meters on, window hidden (special workspace) | 0 % | 7.0 % ❌ | 14.2 % ❌ | 3.2 % |
| CPU idle, no meters | ~0 % | 0.00 % | 0.03 % | 0.00 % |

The hidden-window result is expected (the prototype never pauses meters) and
confirms the meter pause requirement. The visible-window CPU is a new risk.
Per-thread context switches over 10 s, femtovg, window on a hidden special
workspace:

| Thread | meters on (7 Hz) | meters off |
| --- | --- | --- |
| main (`junc-proto`) | 92.6/s | 1.2/s |
| `smithay-clipboard` (Wayland connection) | 49.3/s | 0.9/s |
| others (`blocking-1`, `agx-bo-cache`) | 3/s | 2/s |

So each meter tick costs ~20 wakeups: a full-window femtovg repaint plus the
Wayland round trips that wake the clipboard thread, even when the compositor
is not showing the window (winit renders without waiting for a frame
callback). About 0.9 % of a core per repaint at HiDPI. Plan task B15 must
(a) stop meters when hidden, (b) repaint only the meter rectangles (Slint
partial rendering: try the skia renderer and the software renderer's
dirty-region path), (c) consider a lower rate when the window is unfocused,
and re-measure against the < 2 % target.

## Accessibility of the Slint prototype under Hyprland

Raw results: `bench/validation/results-m1-a11y/`.

- AT-SPI tree on Wayland is identical to the VM's X11 and Weston trees,
  except that current at-spi names the button role `button`.
- Keyboard walk with `wtype` (`interaction_test_wayland.sh`): same as the VM.
  Tab order follows reading order; Space and Return toggle the switch;
  slider ←/→ ±1, Home/End work, **PageUp does nothing**; the picker takes
  focus into its search field, traps Tab, Esc closes it and returns focus to
  the opener; **the search field has no name**; **slider focus is nearly
  invisible** (`focus-slider.png`).
- Orca 51.1 (`orca_test_wayland.sh`) spoke: "JUNC", "Junctions List with 3
  items, Stream Mix", "New junction button", "Add input… button", "Firefox
  panel, Enable Firefox toggle button pressed", "not pressed", "Volume of
  Firefox horizontal slider 80 percent", "81". Unlike the VM run, the first
  list item was spoken.
- Owner listening (2026-11-25): the voice worked; buttons were clear;
  **switches were hard to understand** ("toggle button pressed / not
  pressed"); speech felt too fast because each key press (1.6 s apart)
  interrupted the previous phrase. Consequences: the `JSwitch` label +
  on/off description is required, and the beta Orca acceptance run uses
  ≥ 3 s between key presses.
