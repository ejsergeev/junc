# Hardware matrix

What each hardware layer (`architecture.md` §11.1) can cover on which machine,
and the measured results. Raw outputs live next to the bench scripts
(`bench/validation/bench/results-m1-*`).

## Machines

| Machine | Arch | OS / session | Audio stack | Used for |
| --- | --- | --- | --- | --- |
| Owner laptop: MacBook Pro 13" M1 (J293), 8 cores | aarch64 | Arch Linux ARM, Omarchy (quattro), Hyprland, kernel 7.2.11-14-1-ARCH | PipeWire 1.6.9, WirePlumber 0.5.18, pipewire-pulse, pipewire-jack (since 2026-11-25), Asahi DSP (`j293-convolver`, `j293-mic`), speakersafetyd | layers 2, 3, 5; GUI on GPU; Orca |
| CI (GitHub runners) | x86_64 + aarch64 | Arch / Arch Linux ARM containers, plus a systemd-PID-1 container for the user session | PipeWire 1.6.11, WirePlumber 0.5.19, headless, null devices | layers 1, 2; perf gate |
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

## Alpha run on the owner laptop (A19, 2026-11-25)

The release `juncd` and `junc` from main ran in the owner's live desktop
session (Hyprland, Asahi DSP chains, desktop apps), with a separate test
config and state directory; the owner's own config was not touched.
Three test Junctions: **A19 Mic** (Komplete Audio 2 input + a test app,
heard on the Komplete output), **A19 Speakers** (a test app heard on the
default output, i.e. the speakers through `j293-convolver`), **A19 Loop**
(snd-aloop). Test apps ran with `node.autoconnect=false`, so they reached
an output only through JUNC. Cleaned up afterwards: no JUNC objects left,
snd-aloop profile back to `off`, defaults as before.

| Check | Req | Result |
| --- | --- | --- |
| Start: 3 Junctions, 2 real devices, Asahi default sink | F1, F2, F9 | ✅ routed in 22 ms after connecting; idle wakeups 0/s |
| Layer 3: noise → snd-aloop (`pro-audio`, out 0 → in 1, real ALSA path) → Junction → recorder | F2, F4 | ✅ 192 000 of 192 000 samples bit-exact |
| Hand edit of the config in the live session (switch A19 Loop's device) | F22 | ✅ applied at once |
| Owner listens: tone via A19 Speakers → Listen `default` → MacBook speakers (DSP) | F6, F16 | ✅ heard, clean (owner) |
| Owner listens: tone via A19 Mic → Listen → Komplete Audio 2 headphones | F6 | ✅ heard, clean (owner) |
| Re-link in the live session, 30 app restarts × 3 runs | F10 | ⚠️ p50 43–57 ms, **p95 81, 90, 101 ms** (target < 100 ms) |
| Another tool removes a JUNC link (`pw-link -d`) | F17 | ✅ not recreated, `link_overridden` reported; Restore via D-Bus brings it back |
| A19 Mic as default input, then cleared | F16 | ❌ default set and configured value restored, but the mic **in use** did not come back (see below) → fixed, and re-run live: the mic in use returned to `omarchy_asahi_mic` while the Junction still existed ✅ |
| PipeWire restart (`systemctl --user restart pipewire pipewire-pulse wireplumber`) | F9, §3.6 | ✅ juncd survived; all 38 JUNC links back 1.47 s after the restart command (PipeWire, WirePlumber and the Asahi DSP chains starting) |
| Komplete Audio 2 unplugged ~13 s and plugged back | F10 | ✅ all 4 links (input + Listen) back within the 100 ms polling step; owner heard the tone again |
| Suspend and resume | F9, F10 | ✅ juncd survived (same process); Junctions intact. The Komplete Audio 2 itself did **not** come back after resume (kernel: `usb 1-1: USB disconnect`, no re-enumeration until re-plugged) — a USB/Asahi issue, not JUNC; JUNC showed its Input and Listen `waiting` and re-linked within 100 ms of the re-plug; tones clean |

**F16 bug found and fixed.** The owner's configured default input was the
headset jack (unplugged), so WirePlumber was using `omarchy_asahi_mic`.
JUNC saved and restored only the configured value. With the headset still
unavailable, WirePlumber then picked the newest device in its history —
the test Junction — so the mic in use stayed the Junction until it was
deleted. JUNC now also remembers the device in use when it differs from
the configured one, and if WirePlumber still uses a Junction after the
restore, makes that device the default (test
`restores_the_device_in_use_when_the_configured_one_is_unavailable`).
Re-run live with the fix, it ended with `omarchy_asahi_mic` in use, as
before the test. The price: the configured default becomes that device
(the unplugged headset stops being the configured one), since otherwise
WirePlumber keeps choosing the Junction. After the re-run the owner's
original configuration (headset configured, built-in mic in use) was put
back by hand.

**Re-link in the live session** is at the target's edge. These numbers
time from the observer seeing the app's node to it seeing the links; the
observer is itself a JUNC backend that batches and binds, so they include
its own delay, unlike the prototype's lighter observer (p95 89 ms). The
isolated instance gives 6–9 ms in CI and ~45 ms on this laptop. Profiling
where the time goes in a busy session is a beta task.

Also noticed: the log line "graph complete after N ms" counted from the
daemon's start, so it showed 62 s after a hand edit and 190 s after a
PipeWire restart; it now counts from the change or reconnection.

## Pro Audio on the owner laptop (B3, layer 3, 2026-11-25)

Release `juncd` and `junc` from the B3 branch in the owner's live desktop
session, with a test config and state directory (the owner's own config
untouched) and the `snd-aloop` card already loaded. One Junction, **B3
Loop**, with one Input: `device = "alsa_input.platform-snd_aloop.0.pro-input-1"`,
`profile = "pro-audio"`. Cleaned up afterwards: no JUNC objects left,
Loopback profile `off`.

| Check | Req | Result |
| --- | --- | --- |
| Card on its stereo profile when juncd starts | F15 | ✅ juncd switched it to `pro-audio`; the Input became `active` on the 32-channel `pro-input-1` (links 1→1, 2→2) |
| First try | F15 | ❌ the card switched but juncd still saw the stereo profile and the Input stayed `waiting`: a parameter subscription does not report a profile switch → fixed (re-read `Profile`/`EnumProfile` on every parameter change, as `pw-dump` does) and re-run ✅ |
| `junc describe` devices | F15, F25 | ✅ Loopback: `pro-audio`, 32 inputs in groups 1–8 … 25–32, no offer (already Pro Audio). The Asahi built-in cards (UCM `HiFi`) offered Pro Audio on the first try → fixed: UCM cards are never offered or switched (it would bypass the speaker setup) ✅ |
| Noise → `pro-output-0` → snd-aloop (real ALSA path) → `pro-input-1` → Junction → recorder | F2, F15 | ✅ 72 000 of 72 000 frames bit-exact |
| User switches the card to stereo with `wpctl set-profile` | F15, F17 | ✅ `profile_changed` notice; the Input's `profile` saved as `output:analog-stereo+input:analog-stereo` (file formatting kept); not switched back; the Input follows to the stereo capture node |
| User turns the card `off` | F15 | ✅ notice, config kept, left off |

A real re-plug was not simulated (unloading `snd-aloop` needs WirePlumber
stopped); a card JUNC sees for the first time is switched, which is what a
re-plug looks like, and the mock tests cover a new `object.serial`.

