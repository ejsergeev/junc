## JUNC bug report

junc 0.1.0 · juncd <version> · PipeWire 1.6.9 · WirePlumber 0.5.18 · kernel 7.2.11-14-1-ARCH (aarch64)
graph: 1 Junction, 1 Cable · 0 links · 48000 Hz · quantum 1024
config: ~/.config/junc/junctions.toml (valid)

### Checks

```
✓ pipewire        running 1.6.9
✓ wireplumber     running
✓ pipewire-pulse  running
✓ jack            pipewire-jack: JACK apps go through PipeWire
✓ clock           48000 Hz, quantum 1024 (21.3 ms)
✓ rates           every hardware device can run at 48 kHz
✓ asahi           the speakers play through the Asahi DSP (audio_effect.j293-convolver); speakersafetyd running
! juncd           running, but starting
```

### Junctions

```
Virtual Mic for Zoom (virtual-mic-for-zoom): unknown
  default input, volume 100%, 4 Bus channels, node junc.virtual_mic_for_zoom
  Inputs:
    spotify  app spotify                                                    unknown  70%   1→1 2→2
    soundux  app soundux                                                    unknown  100%  1→3 2→4
    usb-mic  device alsa_input.usb-Generic_USB_Microphone-00.mono-fallback  unknown  100%  1→1 1→2
  Listen:
    builtin  default  unknown  80%  1→1 2→2
Firefox → OBS (firefox-to-obs): unknown
  volume 100%, 2 Bus channels, node junc.firefox_to_obs
  Delivers to obs: unknown
  Inputs:
    firefox  app firefox  unknown  100%
```

### Config

```toml
# JUNC configuration. Edit by hand or with `junc`; the daemon picks up changes.
schema = 1

[[junction]]
id = "virtual-mic-for-zoom"        # stable; node.name = "junc.virtual_mic_for_zoom"
name = "Virtual Mic for Zoom"
kind = "junction"
enabled = true
volume = 100
default = "input"
rate = "follow"
bus = { channels = 4 }

[[junction.input]]
id = "spotify"
match = { app = "Spotify", binary = "spotify" }
volume = 70
exclusive = true
links = [[1, 1], [2, 2]]

[[junction.input]]
id = "soundux"
match = { binary = "soundux" }
links = [[1, 3], [2, 4]]

[[junction.input]]
id = "usb-mic"
match = { device = "alsa_input.usb-Generic_USB_Microphone-00.mono-fallback" }
links = [[1, 1], [1, 2]]
suppressed = [[1, 2]]

[[junction.listen]]
id = "builtin"
target = "default"
volume = 80
links = [[1, 1], [2, 2]]

[[junction]]
id = "firefox-to-obs"
name = "Firefox → OBS"
kind = "cable"

[[junction.input]]
id = "firefox"
match = { binary = "firefox" }

[junction.cable]
to = { binary = "obs" }
```

### Log (last 200 lines of `journalctl --user -u juncd`)

```
2026-11-25T10:00:00+0100 <host> juncd[812]: juncd: connected to PipeWire
2026-11-25T10:00:00+0100 <host> juncd[812]: juncd: config ~/.config/junc/junctions.toml
2026-11-25T10:00:01+0100 <host> juncd[812]: juncd: notice link_overridden junction=zoom ref=in/bluez_input.XX:XX:XX:XX:XX:XX/1->1
```

No audio. User and host names, home paths and hardware addresses are replaced.
