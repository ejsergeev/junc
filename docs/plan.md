# JUNC — delivery plan

## Git rules (every commit, every session)

- Repository identity, set before the first commit in a fresh clone:
  `git config user.name "Evgenii Sergeev"` and
  `git config user.email "ejsergeev@gmail.com"`.
- Author and committer of every commit: `Evgenii Sergeev <ejsergeev@gmail.com>`.
- Every commit carries the DCO sign-off (`git commit -s`):
  `Signed-off-by: Evgenii Sergeev <ejsergeev@gmail.com>`.
- No mention of AI anywhere in git history: no `Co-Authored-By` lines, no
  "Generated with…" footers, no AI tool names in commit messages, branch
  names, tags or PR descriptions. This overrides any default attribution a
  tool adds.
- One logical change per commit (e.g. `core: add graph model`,
  `daemon: tag owned links`, `docs: describe D-Bus API`). Never mix unrelated
  changes, formatting with logic, or generated files with hand-written code.
  Each commit builds and passes tests on its own.
- Commit message: imperative subject ≤ 72 chars prefixed with the area
  (`core`, `daemon`, `cli`, `gui`, `i18n`, `skill`, `packaging`, `ci`,
  `docs`), blank line, body explaining why when it isn't obvious.
- Full text and the DCO: [`CONTRIBUTING.md`](../CONTRIBUTING.md).

## How every working session runs

1. Start by reading `docs/architecture.md` and this file.
2. Pick the next unticked task of the current stage (order matters inside a
   stage; tasks marked *parallel* may be interleaved).
3. Tick tasks off (`- [x]`) in the same branch that completes them, with the
   date and a one-line result, in a `docs:` commit.
4. Keep `docs/architecture.md` current: any change to the D-Bus API, CLI, config
   schema, PipeWire object layout or test gates updates it in its own commit.
5. Ask the owner before anything that contradicts the PRD or needs an
   owner-only action (hardware, sudo, listening, signing key, Omarchy PRs,
   release sign-off). Record the answer here under "Decisions log".

Legend: **[auto]** = done and checked by the developer (CI or the owner's
laptop without the owner's hands); **[owner]** = needs the owner (hardware,
sudo, ears, keys, PRs, sign-off). Each task lists requirement IDs and the
acceptance check that closes it.

---

## Step 0 — validation rerun on the owner's Omarchy laptop

Read-only pre-findings (2026-11-25, observed while writing this plan):

- The owner's machine is an **Apple MacBook Pro 13" M1 (2020) on Arch Linux
  ARM (aarch64)** with Omarchy (quattro), kernel `7.2.11-14-1-ARCH`,
  PipeWire **1.6.9**, WirePlumber **0.5.18**. Primary test platform is
  therefore aarch64, not x86_64.
- Audio stack is the Asahi one: default sink is the DSP filter-chain
  `audio_effect.j293-convolver`, default source `omarchy_asahi_mic` (a
  loopback Omarchy creates), `speakersafetyd` installed. JUNC must route to
  the DSP sink, never to raw speaker nodes (spike S8).
- `jack2` is installed instead of `pipewire-jack` (they conflict), so JACK
  apps do not see PipeWire nodes today. Onboarding must word this as
  "replace jack2 with pipewire-jack", not "install".
- Kernel modules present: `snd-dummy`, `snd-aloop`, `libcomposite`,
  `usb_f_uac2`, `g_audio`. **Missing: `dummy_hcd`**, and `/sys/class/udc` is
  empty → layer 4 (emulated UAC2 gadget) is **not available** without
  building `dummy_hcd` out of tree.
- Omarchy theme files exist at `~/.local/state/omarchy/current/theme/`
  (`colors.toml`, `shell.toml`); no `~/.config/omarchy/language` file yet
  (= English). User plugins live in `~/.config/omarchy/plugins/`.
- Rust toolchain is available via mise; `slint-tr-extractor` is not installed.

Tasks:

- [x] **0.1** [owner runs sudo] Install the harness dependencies and replace
  jack2 with pipewire-jack (approved 2026-11-25):
  `sudo pacman -S --needed pipewire-jack orca python-gobject at-spi2-core
  jack-example-tools wtype ydotool noto-fonts-cjk ttf-jetbrains-mono gettext
  powertop xorg-server-xvfb xdotool imagemagick` (pacman asks to remove jack2).
  Acceptance: `pacman -Q` lists them and `jack2` is gone.
  Done 2026-11-25: all installed, jack2 replaced by pipewire-jack; clang added for pipewire-rs bindgen.
- [x] **0.2** [auto] Copy `junc-validation/` into `bench/validation/` (its own
  `ci:` commit, as imported reference), read every script, adapt paths. Build
  `proto` and `bench/juncd` natively on aarch64. Acceptance: both build with
  `--release --locked`.
  Done 2026-11-25: imported unchanged (`77fe001`), then made safe beside a live session and WirePlumber 0.5 (`fddd2ea`); Rust 1.100 via mise, JACK-Client/numpy in a user venv; both prototypes build.
- [x] **0.3** [auto; owner approved interrupting audio for ~1 h on 2026-11-25] `PW_SESSION=systemd
  bench/run-bench.sh juncd-links` and `manual` at quantum 1024 and 256, plus
  `powertop --csv` for real wakeups. Acceptance: results in
  `docs/hardware-matrix.md`; any target missed becomes a risk below.
  Done 2026-11-25 except `powertop` (needs root; ctx-switch counts used instead): all daemon targets met in the isolated instance and the live session; live re-link p95 89 ms is close to the limit (see risks). Results in `docs/hardware-matrix.md`.
- [x] **0.4** [auto] Slint prototype under Hyprland: `interaction_test.sh` with
  `wtype`/`ydotool`, AT-SPI dump, `perf.sh` with femtovg on the Apple GPU
  (Mesa asahi) and software renderer, cold start, CPU hidden (minimized /
  other workspace). Acceptance: numbers recorded; femtovg PSS < 80 MB or a
  renderer decision logged.
  Done 2026-11-25: femtovg 88 ms cold start, 62 MB PSS, but 6.4 % CPU with meters (target 2 %, see risks); AT-SPI tree on Wayland matches the VM; keyboard walk with `wtype` matches the VM (PageUp no-op, unnamed search field, invisible slider focus). Results in `bench/validation/results-m1-a11y/`.
- [x] **0.5** [owner listens, auto drives] `orca_test.sh` under Hyprland; owner
  confirms Orca speaks switch/slider/popover. Acceptance: speech log stored,
  owner's yes/no logged.
  Done 2026-11-25: Orca spoke every control; owner: buttons clear, switches hard to understand ("toggle button pressed"), too fast because key presses cut phrases off. → `JSwitch` on/off description is required; Orca runs use ≥ 3 s between keys.
- [x] **0.6** [owner sudo once, auto drives] Load `snd-dummy` and `snd-aloop`
  (`pcm_substreams`, `channels` up to 32), check they appear in PipeWire with
  ACP profiles including `pro-audio`; record `pw-dump` as fixtures. Layer 4
  (UAC2 gadget) is skipped by owner decision; record why (no UDC, no
  `dummy_hcd`). Acceptance: `docs/hardware-matrix.md` lists each layer as
  available/skipped with evidence.
  Done 2026-11-25: snd-dummy and snd-aloop work; Loopback offers pro-audio with 32-channel nodes; fixtures `45bed80`. Unloading needs WirePlumber stopped, so hot-plug tests switch profiles instead.
- [x] **0.7** [owner plugs in] Native Instruments Komplete Audio 2 (USB),
  Google Pixel Buds 2a and Apple AirPods (Bluetooth), each once, Bluetooth
  in both A2DP and headset (HFP) mode. [auto] capture `pw-dump` of each as
  layer-1 fixtures (personal data such as MAC addresses stripped).
  Acceptance: fixtures committed.
  Done 2026-11-25: Komplete Audio 2 (stereo and pro-audio, both 2 ch), AirPods (A2DP AAC, HFP mSBC), Pixel Buds 2a (A2DP AAC, HFP LC3-SWB) captured as fixtures (`3b18706`), anonymised.
- [x] **0.8** [auto] Capture the Asahi audio topology (`pw-dump` of the DSP
  chain) as a layer-1 fixture. Acceptance: fixture committed.
  Done 2026-11-25: `1f6cb71`; raw speaker and mic-array nodes are hidden from normal clients.
- [x] **0.9** [auto] Update `architecture.md` and the risks below with what
  step 0 found. Acceptance: `docs:` commit; owner reads it before alpha.
  Done 2026-11-25: hardware matrix complete; architecture updated (Pro Audio counts from ALSA for USB, Bluetooth virtual headset mic, Asahi speakers hidden, profile-based hot-plug, Orca pacing, meter repaints) in `f7ce832`.

---

## Alpha — `juncd` + CLI

Exit criteria (PRD): F1–F5, F9, F10, F16, F17 work from the CLI; all agent
checks green in CI (x86_64 + aarch64) and on the owner's laptop.

- [ ] **A1** [auto] Workspace skeleton: crates per `architecture.md` §2,
  `LICENSE` (GPL-3.0 text, own commit), rustfmt/clippy/deny config,
  dependency-rule check, CI on x86_64 and aarch64 Arch containers.
  Req: — . Acceptance: empty binaries build and CI is green on both arches.
- [ ] **A2** [auto] Spikes S1–S5 and S8 (`architecture.md` §12), each
  ending in a note and, if needed, a design change. Req: F1, F7, F11, F17,
  F20. Acceptance: every spike has a recorded pass or chosen fallback.
- [ ] **A3** [auto] `junc-core` model + config: TOML load/save with
  `toml_edit`, schema v1 types, JSON Schema generation (generated file in
  its own commit), validator with file/line/column diagnostics and
  suggestions, migration framework with a v0→v1 dummy fixture. Req: F22,
  F25. Acceptance: unit tests over valid/invalid fixtures; schema freshness
  check in CI.
- [ ] **A4** [auto] Graph model + identity matcher (apps by binary/name/role,
  devices by `node.name` with fallbacks, monitors, Junctions). Req: F2, F10.
  Acceptance: matcher tests over recorded `pw-dump` fixtures incl. browsers
  with several streams and the Asahi topology.
- [ ] **A5** [auto] Planner + `MockBackend` + layer-1 fixtures (1–64 ch,
  odd maps, appear/disappear, foreign links). Invariant test: no action ever
  targets an untagged object. Req: F3, F4, F17. Acceptance: property-based
  tests pass; invariant holds over all fixtures.
- [ ] **A6** [auto] `junc-pw` backend: registry → graph, create tagged
  lingering nodes and links, metadata, inotify socket watch with the
  validated fallback poll and backoff reset. Req: F1, F9, F10.
  Acceptance: layer-2 test creates a Junction visible to pulse, JACK and
  native clients.
- [ ] **A7** [auto] Daemon reconcile loop and link state machine
  (Waiting/Active/Overridden/Disabled), adoption of surviving objects,
  overridden detection with `core.sync`. Req: F10, F17. Acceptance: layer-2
  churn test ≥ 99 % (target 100 %) links restored, re-link p95 < 100 ms;
  foreign links untouched; `link_overridden` raised.
- [ ] **A8** [auto] Default device handling with save/restore and the
  `default` Listen target following the system default. Req: F16.
  Acceptance: layer-2 tests incl. "restore target missing".
- [ ] **A9** [auto] Config watching, last-good, revision counter,
  comment-preserving writes. Req: F22. Acceptance: hand-edit tests (valid,
  invalid with line numbers, concurrent edit → `E_CONFLICT`).
- [ ] **A10** [auto] D-Bus API v1 (`Daemon`, `Junction`), errors, generated
  `docs/dbus.md`. Req: F12, F23 (property only). Acceptance: zbus round-trip
  tests; `time junc enable X` < 50 ms in CI.
- [ ] **A11** [auto] CLI for alpha scope: list, status, create, rename,
  duplicate, delete, enable/disable/toggle/switch, default, volume (master),
  input/bus/link/listen basics, `--json` envelope, exit codes, localized
  human messages scaffold (English only for now). Req: F1–F5, F12, F16.
  Acceptance: CLI tests against the mock daemon + layer-2 smoke.
- [ ] **A12** [auto] Agent-facing CLI: `schema`, `config validate`,
  `apply [--dry-run]`, `describe`, `doctor` (PipeWire, WirePlumber,
  pipewire-pulse, pipewire-jack/jack2, quantum, rate). Req: F19, F25.
  Acceptance: golden JSON outputs; dry-run never changes the graph (asserted).
- [ ] **A13** [auto] Cable: one app → another, target app's capture linked.
  Req: F5. Acceptance: layer-2 test with two `pw-cat` clients, bit-exact.
- [ ] **A14** [auto] Nesting with cycle detection. Req: F2. Acceptance: nested
  depth 1–4 works; a cycle is rejected with `E_CYCLE` before any change.
- [ ] **A15** [auto] `juncd.service`, D-Bus activation, `junc service`,
  sd_notify. Req: F9. Acceptance: in a systemd-user container, Junctions exist
  after login with no GUI; restore < 1 s for 10 Junctions.
- [ ] **A16** [auto] Layer-2 integration suite in CI using the validation
  `ci-config` (fake devices, no bluez/logind). Req: F1–F5, F9, F10, F16,
  F17. Acceptance: runs on every PR in < 10 min on both arches.
- [ ] **A17** [auto] Port `bench/` to drive the real `juncd`; daemon perf
  gates with a stored baseline (PSS, wakeups, re-link, recovery, CLI latency,
  binary size, 0 extra quanta, bit-exact). Req: PRD performance. Acceptance:
  gate fails on an injected regression.
- [ ] **A18** [auto] *parallel* Skill draft: `SKILL.md`, `recipes.md`,
  `troubleshooting.md`, `reference.md` generator, `skill-check`, `junc skill
  link/unlink`. Req: F25. Acceptance: skill-check green; a coding agent with
  only the skill creates a Zoom mic from the CLI (dry-run shown first).
- [ ] **A19** [owner plugs + sudo, auto runs] Alpha run on the owner's laptop:
  layers 2–3 (snd-dummy/snd-aloop), real USB interface as Input and Listen,
  Asahi default sink as Listen target, re-plug, PipeWire restart,
  suspend/resume. Req: F1–F5, F9, F10, F16, F17. Acceptance: results in
  `docs/hardware-matrix.md`; all pass.
- [ ] **A20** [auto] Alpha exit review: tick criteria, list known issues,
  tag `v0.1.0-alpha` only after the owner says go. [owner] go/no-go.

---

## Beta — GUI in the testing channel

Exit criteria (PRD): all v1 features in the GUI; all agent checks green;
owner walkthrough with the real USB interface. Packages in `junc-testing`.

Daemon features first (they gate GUI screens):

- [ ] **B1** [auto] Gain nodes: per-Input and per-Listen volume, on/off,
  master volume; Listen routing of all or selected Bus channels. Req: F6,
  F7, F8. Acceptance: layer-2 tests; bit-exact at unity (or S4 fallback).
- [ ] **B2** [auto] Exclusive via `target.object` metadata. Req: F11.
  Acceptance: layer-2 test with WirePlumber: app leaves speakers while
  captured, returns when turned off.
- [ ] **B3** [auto] Pro Audio detection/switching, channel groups, profile
  memory on re-plug. Req: F15. Acceptance: layer-1 fixtures (18-ch) + layer-3
  `snd-aloop` on the owner laptop.
- [ ] **B4** [auto] Bluetooth profile detection and warning data. Req: F18.
  Acceptance: layer-1 fixtures from the owner's headset dump (0.7).
- [ ] **B5** [auto] Import of null-sink / pw-loopback setups. Req: F13.
  Acceptance: layer-2 test with `pactl load-module module-null-sink` and
  `pw-loopback` setups.
- [ ] **B6** [auto] Upgrade re-exec, `junc teardown`, pre_remove hook. Req:
  F20. Acceptance: counter-signal continuity test across re-exec; teardown
  leaves no `junc.managed` object.
- [ ] **B7** [auto] Capture indicator + `junc watch --json`. Req: F23.
  Acceptance: event stream tests.
- [ ] **B8** [auto] Presets/templates: export, import with slots, built-in
  set. Req: F22, F24. Acceptance: round-trip tests; every built-in template
  validates.
- [ ] **B9** [auto] Doctor complete (Asahi DSP, rates, jack2) + bug report
  with path scrubbing. Req: F19. Acceptance: golden outputs; scrub test.

GUI foundation:

- [x] **B10** [auto] Reference renders of all canvas boards in `docs/design/`
  (2026-11-25: 51 PNGs rendered locally from the artifact's files with
  headless Chromium; regeneration steps in `docs/design/README.md`).
- [ ] **B11** [auto] `junc-l10n`: language resolution + Omarchy watcher,
  bundled translations, ICU4X numbers, `en_XA`, extraction and catalog
  checks; spike S6. Req: F14. Acceptance: live switch test; CI catalog checks.
- [ ] **B12** [auto] `junc-gui` skeleton: IPC client, AppState, ThemeService
  with live Omarchy reload and fallback themes, window sizing. Req: F21.
  Acceptance: theme switch test on the owner laptop (`omarchy-theme-set`)
  retints without restart.
- [ ] **B13** [auto] Component library per Spec with the a11y fixes and a
  component-gallery scenario. Acceptance: gallery screenshots match the Spec
  board; AT-SPI dump shows labels, switch description, focus rings.
- [ ] **B14** [auto] Scenario harness (`--scenario`, mock IPC), screenshot
  pipeline with goldens, AT-SPI CI job. Acceptance: one scenario end to end
  in CI on both arches.
- [ ] **B15** [auto] Meter pipeline + VisibilityService; spike S7. Req: F7,
  performance. Acceptance: CPU hidden 0 % gate in CI; on the owner laptop
  meters stop within 1 s when minimized or on another workspace.

Screens, in design-canvas order (each: scenario fixture, golden in en/en_XA/
de/ja, AT-SPI dump, keyboard test):

- [ ] **B16** [auto] Main, Flexoki, Narrow; source off, offline, link hover.
  Req: F4, F7, F8, F10.
- [ ] **B17** [auto] Add Input picker; onboarding check + templates (+
  Flexoki); empty none / empty Junction (+ Flexoki). Req: F2, F24, UX
  onboarding.
- [ ] **B18** [auto] Cable selected, link drag, link keyboard, Input Options
  (+ narrow), add Bus, add Listen, Junction off. Req: F3, F4, F5, F6, F8, F11.
- [ ] **B19** [auto] Tray (DBusMenu tree test), service down, context menu
  (duplicate), delete confirmation, app closed, import, settings, rename,
  new Cable. Req: F1, F5, F9, F10, F13, F21.
- [ ] **B20** [auto] Language picker, German, Japanese (Arabic shows English).
  Req: F14.
- [ ] **B21** [auto] Pro Audio prompt, multichannel Input, use as default,
  default restored, link overridden, Bluetooth warning. Req: F15, F16, F17,
  F18.
- [ ] **B22** [auto] Bug report, doctor issue, updated, export preset, config
  error, new menu, template slots. Req: F19, F20, F22, F24.

Omarchy and packaging:

- [ ] **B23** [auto] Bar plugin (QML) + popup + capture dot, plugin catalogs
  from `.po`, `junc omarchy install/uninstall`, menu extension entry,
  keybinding suggestion. Req: F21, F23. Acceptance: `omarchy plugin
  validate` passes; `omarchy agent look plugin` screenshot matches the
  `barModule` board; menu row appears and opens the GUI.
- [ ] **B24** [owner] Create the repository signing key (offline primary,
  signing subkey), add the subkey as a GitHub Actions secret in a protected
  environment, publish the fingerprint. Create the repositories
  `ejsergeev/junc` and `ejsergeev/junc-pkgs` (GitHub Pages) on the owner's
  personal account; no organisation.
- [ ] **B25** [auto] PKGBUILD (split `junc`/`junc-gui`), `junc-keyring`,
  CI publish to `junc-testing` for x86_64 + aarch64, install smoke test in a
  fresh container. Acceptance: `pacman -S junc-gui` from the testing repo on
  the owner laptop works with signature checking on.
- [ ] **B26** [auto] GUI perf gates: cold start, PSS, CPU with meters, CPU
  hidden, frame time while dragging (owner laptop for GPU numbers).
- [ ] **B27** [owner] Beta walkthrough with the real USB interface, once,
  following a script the developer writes (every screen, every F-ID). Owner
  reports issues; developer files them.
- [ ] **B28** [auto] Beta exit review; [owner] go/no-go; tag `v0.x.0-beta`.

---

## v1 — stable channel

Exit criteria (PRD): all 46 LTR languages present and marked machine-drafted;
no open P0 bugs; owner listening test and release sign-off. Full F1–F25.

- [ ] **V1** [auto] Machine-drafted translations for the 45 non-English LTR
  languages (GUI, CLI, tray, bar plugin), marked machine-drafted; catalog
  checks green. Req: F14.
- [ ] **V2** [auto] Nightly all-language × all-scenario screenshots clean
  (no overflow, no tofu); per-language font choices for `ja`, `zh-*`, `ko`.
  Req: F14.
- [ ] **V3** [auto] 24 h soak on the owner laptop (xruns, PSS flat, wakeups
  via `powertop`), scaling runs 1/8/16/64 ch. Req: PRD performance.
- [ ] **V4** [auto drives, owner listens] Orca under Hyprland on the final
  GUI. Req: UX accessibility.
- [ ] **V5** [auto] Skill final: all topic guides, reference regenerated,
  agent acceptance run on the owner laptop with three requests (Zoom mic,
  stream mix, Firefox → OBS cable). Req: F25.
- [ ] **V6** [auto] Docs: README (install from the signed repo), user manual,
  `man junc`, `man juncd`, D-Bus reference, `docs/compatibility.md`.
- [ ] **V7** [auto] Bug bash against the PRD: every F-ID has a passing
  automated test; P0 list empty.
- [ ] **V8** [auto] Housekeeping: CODE_OF_CONDUCT.md, issue and PR
  templates, translation review rules.
- [ ] **V9** [owner] Listening test in Discord/OBS (no clicks, sounds right)
  and release sign-off; approve the stable promotion environment.
- [ ] **V10** [auto] Tag `v1.0.0`, promote to `junc` (stable), release notes
  incl. D-Bus API and config schema versions.
- [ ] **V11** [auto prepares, owner opens] Upstream PRs after stable:
  omacom/omarchy-pkgs (`pkgbuilds/junc`), omacom/omarchy (menu entry +
  `menu.json` key, skill pointer in `system.md`).

---

## Risks

| Risk | Impact | Recommendation |
| --- | --- | --- |
| Owner hardware is aarch64 Asahi; x86_64 is only tested in CI | x86_64-only bugs (other ALSA drivers, Intel/AMD HDA quirks) reach users | Keep both arches in CI with layers 1–2; ask early adopters on x86_64 via the testing channel; compatibility table. |
| Asahi DSP/speaker safety | Routing around the DSP could produce harsh sound or, with raw speaker access, hardware risk | Spike S8; never link to raw speaker nodes; Listen `default` follows the DSP sink; doctor flags it. |
| Layer 4 (UAC2 gadget) skipped | USB hot-plug and multichannel USB only tested via snd-aloop + one real device | Accepted by the owner (2026-11-25); hot-plug via `modprobe -r/modprobe snd-aloop`; real interface in A19/B27. |
| The real USB interface has 2 channels | F15 (Pro Audio, more channels) never runs on real multichannel hardware before release | snd-aloop multichannel cards in layer 3; ask early adopters with 8+ channel interfaces via the testing channel; compatibility table. |
| Pi 4 targets cannot be measured (no Pi) | "< 2 % CPU on Pi 4" unverifiable | Best-effort (owner, 2026-11-25); measure on the M1 and in CI aarch64. |
| Bluetooth on Asahi | AirPods / Pixel Buds HFP mic may behave differently from x86 BlueZ setups | F18 logic is tested on recorded fixtures (layer 1); real devices only confirm detection and the warning. |
| GUI CPU with meters: 6.4 % of a core on the M1 with femtovg (target < 2 %), ~20 wakeups per meter tick from full-window repaints and Wayland round trips | Battery drain while JUNC is open; fails the PRD GUI target | B15: pause when hidden, repaint only meter regions (skia or software partial rendering), lower rate when unfocused; measure early in beta, before building all screens on one renderer. |
| Re-link in the live session: p95 89 ms with the prototype (target < 100 ms) | Target missed on busier desktops | A7 batches reconcile per `core.sync` instead of per registry event; A19 re-measures in the live session. |
| Audio/Duplex visibility differs across pulse/JACK/apps | Junctions missing from some apps' device lists | Spike S1 with real apps (Zoom/Discord/OBS in Chromium, Firefox); two-node fallback. |
| Gain-stage bit-exactness | "bit-exact" metric fails | Spike S4; direct link at 100 % fallback. |
| WirePlumber policy changes (0.5.x) | Exclusive/defaults behave differently | CI on current Arch packages (rolling); layer-2 tests pin behaviour; doctor reports WP version. |
| Slint gaps (RTL, dialog role, visibility events) | a11y and v1.x RTL work | Components direction-aware from day one; upstream issues; Hyprland IPC for visibility. |
| Omarchy plugin/menu APIs are young and change on `quattro` | Bar plugin breaks after an Omarchy update | `omarchy plugin validate` in CI against the latest `quattro`; keep the plugin thin (data via `junc watch`). |
| Uninstall cannot reliably reach user sessions | Orphan nodes until PipeWire restarts | Best-effort pre_remove; `junc teardown` documented; harmless leftovers. |
| Signing key handling | A leaked subkey compromises users | Offline primary, CI-only subkey, protected environment, documented revocation. |
| Machine-drafted translations | Wrong or awkward UI text | Marked machine-drafted; short strings; Weblate after v1. |

## Open questions

None open. New questions go here with a recommendation, and move to the
decisions log once the owner answers.

## Decisions log

| Date | Decision | By |
| --- | --- | --- |
| 2026-11-24 | Fixed decisions from the planning brief (Rust workspace, server-side nodes + links, PipeWire ≥ 1.0, 46 LTR languages, TOML config, agent skill, own signed repo, hardware layers, no telemetry, stages without dates, F1–F25 in v1) | owner |
| 2026-11-25 | Design renders: made locally from the artifact's files (no PNG export needed) | owner |
| 2026-11-25 | No GitHub organisation: personal account `ejsergeev`; D-Bus name `io.github.ejsergeev.Junc1`; package repo on `ejsergeev/junc-pkgs` Pages (replaces the PRD's `junc-audio` org) | owner |
| 2026-11-25 | Step 0 may install harness dependencies with sudo and replace jack2 with pipewire-jack | owner |
| 2026-11-25 | Step 0 may interrupt audio (~1 h) for PipeWire restart benchmarks | owner |
| 2026-11-25 | Real devices: NI Komplete Audio 2 (USB), Google Pixel Buds 2a, Apple AirPods (Bluetooth) | owner |
| 2026-11-25 | Hardware layer 4 (emulated UAC2 gadget) skipped | owner |
| 2026-11-25 | Raspberry Pi 4 performance targets are best-effort | owner |
| 2026-11-25 | Full F1–F25 in v1; the PRD's Delivery table is updated to match (F13 and F18 move from v1.1 to v1) | owner |
| 2026-11-25 | Split packages `junc` (daemon, CLI, skill) and `junc-gui`; Omarchy's install entry installs `junc-gui`; no `junc-bin` | owner |
| 2026-11-25 | The language picker lists the 46 v1 languages; the four RTL languages are added when RTL ships | owner |
| 2026-11-25 | The Junction node class follows spike S1 without another owner decision, unless S1 ends in the two-node fallback | owner |
| 2026-11-25 | Spike S1 ended in the two-node fallback; every Junction is always both a source (`Audio/Source/Virtual`) and a sink (`Audio/Sink`), like a Loopback device | owner |
| 2026-11-25 | Public repository `ejsergeev/junc` on GitHub | owner |
