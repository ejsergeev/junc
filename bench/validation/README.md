# junc-validation

Throwaway prototype + reproducible scripts behind `report.md`.

```
namecheck/check_names.sh          registry probe (AUR, Arch, crates.io, npm, Flathub; GitHub via gh if authed)
namecheck/trademarks-*.md         manual USPTO / EUIPO search log (sites are WAF/JS-protected)
proto/                            Slint 1.19.1 prototype (main screen mock)
  a11y/env-up.sh                  headless X (Xvfb :99) + session D-Bus + AT-SPI bus
  a11y/atspi_dump.py              dump AT-SPI tree / focused object (python3 + gi Atspi)
  a11y/keyboard_walk.sh           press a key N times, print focused object each time
  a11y/interaction_test.sh        switch / slider / popover keyboard checks + screenshots
  a11y/orca_test.sh               drive the UI under Orca, extract what Orca speaks (debug log)
  i18n/make_catalogs.py           .po/.mo for 15 languages + en + en_XA pseudo-locale (+40%)
  i18n/screenshots.sh             one PNG per language (Window::take_snapshot)
  i18n/live_switch_test.sh        rewrite ~/.config/omarchy/language at runtime (throwaway HOME)
  perf.sh                         cold start, PSS, CPU with 7 Hz meters, CPU when hidden
bench/
  pw-session.sh                   start/stop/restart PipeWire (+ systemd mode for real machines)
  graph-manual.sh                 reference graph by hand (pw-cli, pw-link, pw-loopback)
  juncd/                          minimal pipewire-rs daemon (juncd) + independent observer (pwobs)
  apps.sh                         3 fake app streams; `churn` restarts one per minute
  latency.sh, impulse_latency.py  jack_iodelay + impulse round-trip through the junction
  run-bench.sh                    full harness: build, latency, idle PSS/wakeups, pw-top, churn, recovery
  ab-applink.sh                   A/B: app ERRs with juncd-made vs WirePlumber-made links (two PipeWire instances)
  ci-config/                      CI-only PipeWire/WirePlumber config (fake devices, no bluez/logind)
  results/, results-reconnect-fix/  raw outputs of the runs quoted in report.md
screenshots/                      a11y, i18n, live-switch, trademark search evidence
```

## Rerun

```sh
# deps (Arch): pipewire pipewire-jack wireplumber jack-example-tools orca python-gobject at-spi2-core
#              xorg-server-xvfb xdotool imagemagick noto-fonts noto-fonts-cjk ttf-jetbrains-mono gettext rust
cargo build --release --manifest-path proto/Cargo.toml
cargo build --release --manifest-path bench/juncd/Cargo.toml
python3 proto/i18n/make_catalogs.py
proto/a11y/env-up.sh            # CI only; on a desktop session skip this
proto/a11y/interaction_test.sh; proto/a11y/orca_test.sh; proto/i18n/screenshots.sh; proto/perf.sh
# real machine: PW_SESSION=systemd; CI: default (spawns pipewire/wireplumber itself)
STABLE_SEC=600 CHURN_SEC=3600 bench/run-bench.sh juncd
STABLE_SEC=600 CHURN_SEC=300  bench/run-bench.sh manual
STABLE_SEC=600                bench/run-bench.sh juncd-links
```
On a real box `graph-manual.sh devices` creates the fake devices only if they don't exist; point `fake.*`
names at real hardware nodes (or keep the null-sink stand-ins) before comparing numbers.
