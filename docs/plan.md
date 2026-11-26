# JUNC — delivery plan

## Git rules

- Every commit carries the DCO sign-off (`git commit -s`).
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

- [x] **A1** [auto] Workspace skeleton: crates per `architecture.md` §2,
  `LICENSE` (GPL-3.0 text, own commit), rustfmt/clippy/deny config,
  dependency-rule check, CI on x86_64 and aarch64 Arch containers.
  Req: — . Acceptance: empty binaries build and CI is green on both arches.
  Done 2026-11-25: PR #1; CI green on x86_64 and aarch64 (run 45591546618). The aarch64 image needed pacman's sandbox off and several mirrors with retries.
- [x] **A2** [auto] Spikes S1–S5 and S8 (`architecture.md` §12), each
  ending in a note and, if needed, a design change. Req: F1, F7, F11, F17,
  F20. Acceptance: every spike has a recorded pass or chosen fallback.
  Done 2026-11-25: S2–S5, S8 pass; S1 took the two-node fallback (owner: always source + sink, like Loopback). Results in architecture §12.1; scripts in `bench/spikes/`.
- [x] **A3** [auto] `junc-core` model + config: TOML load/save with
  `toml_edit`, schema v1 types, JSON Schema generation (generated file in
  its own commit), validator with file/line/column diagnostics and
  suggestions, migration framework with a v0→v1 dummy fixture. Req: F22,
  F25. Acceptance: unit tests over valid/invalid fixtures; schema freshness
  check in CI.
  Done 2026-11-25: model, loader with file/line/column diagnostics and hints, migrations (v0→v1 keeps comments), JSON Schema with ranges and id pattern, 20 invalid + 3 valid fixtures; every code tested.
- [x] **A4** [auto] Graph model + identity matcher (apps by binary/name/role,
  devices by `node.name` with fallbacks, monitors, Junctions). Req: F2, F10.
  Acceptance: matcher tests over recorded `pw-dump` fixtures incl. browsers
  with several streams and the Asahi topology.
  Done 2026-11-25: graph from pw-dump (nodes, ports, links, devices, profiles, defaults); matcher for apps (multi-stream), devices, monitors, Junctions, Listen `default`; plumbing streams excluded. Tested on the Asahi, snd-aloop, Komplete, Bluetooth and browser fixtures.
- [x] **A5** [auto] Planner + `MockBackend` + layer-1 fixtures (1–64 ch,
  odd maps, appear/disappear, foreign links). Invariant test: no action ever
  targets an untagged object. Req: F3, F4, F17. Acceptance: property-based
  tests pass; invariant holds over all fixtures.
  Done 2026-11-25: planner + MockBackend; 9 scenario tests and a property test (64 random configs × 6 recorded graphs) for the untagged-object invariant, convergence and cleanup; a mutation that removes foreign links is caught.
- [x] **A6** [auto] `junc-pw` backend: registry → graph, create tagged
  lingering nodes and links, metadata, inotify socket watch with the
  validated fallback poll and backoff reset. Req: F1, F9, F10.
  Acceptance: layer-2 test creates a Junction visible to pulse, JACK and
  native clients.
  Done 2026-11-25: `junc-pw` thread mirrors the registry (binding nodes/links, spike S9), applies actions, batches snapshots per `core.sync`, reconnects via inotify. Live test (`scripts/ci/live-pipewire.sh`, also in CI) builds a Junction in 2 rounds, visible to PulseAudio and JACK, adopted by a fresh backend, cleaned up exactly. Spike S10 led to deferred renames.
- [x] **A7** [auto] Daemon reconcile loop and link state machine
  (Waiting/Active/Overridden/Disabled), adoption of surviving objects,
  overridden detection with `core.sync`. Req: F10, F17. Acceptance: layer-2
  churn test ≥ 99 % (target 100 %) links restored, re-link p95 < 100 ms;
  foreign links untouched; `link_overridden` raised.
  Done 2026-11-25: `junc-core::reconcile` + `juncd` main loop. Live test (isolated PipeWire, M1): 20/20 app restarts re-linked, re-link p50 14–21 ms, p95 40–45 ms; a link removed with `pw-cli` is not recreated and `link_overridden` is logged; foreign links untouched. Still to verify in the live desktop session in A19.
- [x] **A8** [auto] Default device handling with save/restore and the
  `default` Listen target following the system default. Req: F16.
  Acceptance: layer-2 tests incl. "restore target missing".
  Done 2026-11-25: `junc-core::defaults` + backend `SetDefault` + saved state in `juncd`. Mock tests (7) and live tests: default input set and restored across daemon restarts, key cleared when the previous device is gone, Listen `default` follows the default output. Live suite stable over 10 runs after fixing create/snapshot and metadata races found along the way.
- [x] **A9** [auto] Config watching, last-good, revision counter,
  comment-preserving writes. Req: F22. Acceptance: hand-edit tests (valid,
  invalid with line numbers, concurrent edit → `E_CONFLICT`).
  Done 2026-11-25: `junc-core::store` (7 tests: one-value edits keep comments byte for byte, add/remove entries, suppressed lists, hand edits with positions, comment-only edits, own writes, conflicts) and live reload in `juncd` (live test: rename-style edit applied, invalid edit reported at 5:1 with a hint, restart falls back to the last good config).
- [x] **A10** [auto] D-Bus API v1 (`Daemon`, `Junction`), errors, generated
  `docs/dbus.md`. Req: F12, F23 (property only). Acceptance: zbus round-trip
  tests; `time junc enable X` < 50 ms in CI.
  Done 2026-11-25: contract in `junc-ipc` (constants, proxies, error enum with JSON bodies, `view` JSON types shared with `--json`); server in `juncd` (calls run on the main loop, properties from a published snapshot, Junction objects added and removed live, `GraphChanged`/`Notice` signals, single instance via `DO_NOT_QUEUE`, exit 4). Tests on a private bus: every call and error class, comments kept through edits, hand edits reflected in properties, second daemon refused, call latency p95 ≈ 2 ms (budget 50 ms; the CLI half of the check moves to A11), live statuses and `GraphChanged` on the isolated PipeWire. Introspection XML generated from the daemon with a freshness test; reference in `docs/dbus.md`. v1.0 is the surface above; per-entry methods arrive as minor versions (architecture §4).
- [x] **A11** [auto] CLI for alpha scope: list, status, create, rename,
  duplicate, delete, enable/disable/toggle/switch, default, volume (master),
  input/bus/link/listen basics, `--json` envelope, exit codes, localized
  human messages scaffold (English only for now). Req: F1–F5, F12, F16.
  Acceptance: CLI tests against the mock daemon + layer-2 smoke.
  Done 2026-11-25: `junc` with list, status, create junction/cable, rename, duplicate, delete (asks on a terminal, else needs `--yes`), enable/disable/toggle/switch, default, volume (master, Input, Listen), input add/remove/enable/disable, bus add/remove (later links shift down), link/unlink, listen add/remove; `--json` envelope and the §5.3 exit codes; English messages through the `junc-l10n` scaffold (`tr`/`trf`/`trn`, language resolution). The "mock daemon" is juncd on a private bus without PipeWire: the real config handling with no audio graph. Tests: unit tests of every config edit; end-to-end runs of every command with comments kept and defaults not written; every error class with its exit code; `junc enable` p95 ≈ 5 ms (budget 50 ms); a layer-2 smoke test routing `fake.hwin` through the CLI on the isolated PipeWire. Found and fixed on the way: new Junctions with Inputs were placed under the wrong Junction by the store (refused by its read-back check, so never written).
- [x] **A12** [auto] Agent-facing CLI: `schema`, `config validate`,
  `apply [--dry-run]`, `describe`, `doctor` (PipeWire, WirePlumber,
  pipewire-pulse, pipewire-jack/jack2, quantum, rate). Req: F19, F25.
  Acceptance: golden JSON outputs; dry-run never changes the graph (asserted).
  Done 2026-11-25: `junc schema`, `config path|validate|edit`, `apply FILE [--dry-run] [--expect-revision N]`, `describe`, `doctor` (PipeWire ≥ 1.0 and its socket, WirePlumber, pipewire-pulse, pipewire-jack vs jack2, rate and quantum from the `settings` metadata, juncd state and config errors, each with a fix command). Plans are now a structured view shared by D-Bus and the CLI: a plain-language summary from the config difference, the next step's actions with node names and channels, `complete`, statuses. Golden JSON: plans (create, route, change, delete on the snd-aloop graph), every invalid fixture through `config validate`, `describe` on the mock daemon, `schema` equal to the published file. Layer 2: a dry run leaves the JUNC nodes, links and the file unchanged (pw-dump before and after), and doctor reports the isolated session healthy. Found and fixed on the way: the nesting-loop diagnostic depended on hash seeds (`a → b → a` or `b → a → b`).
- [x] **A13** [auto] Cable: one app → another, target app's capture linked.
  Req: F5. Acceptance: layer-2 test with two `pw-cat` clients, bit-exact.
  Done 2026-11-25: spike S11 showed that a JUNC link into the recording app is mixed with WirePlumber's default-mic link, so Cables now set the target app's `target.object` to the Cable and WirePlumber does the linking (new `Action::SetTarget`, stream targets in the graph model from the `default` metadata). The reconciler tracks what it sent, holds a stream the user moves elsewhere (`link_overridden`, `cable/target/<serial>`), and returns only its own streams to the default when the Cable goes. Tests: planner and reconciler on the mock (target set, override held, Restore, cleanup leaves the user's own choice); live: the recording `pw-cat` is fed by the Cable alone, and 70 208 frames of noise arrive bit-exact (the last partial buffer is never played by `pw-cat` itself and arrives as silence).
- [x] **A14** [auto] Nesting with cycle detection. Req: F2. Acceptance: nested
  depth 1–4 works; a cycle is rejected with `E_CYCLE` before any change.
  Done 2026-11-25: nesting and cycle validation already existed; A14 proves them. Planner on the mock: chains of depth 1–4 converge in ≤ 3 rounds with every level linked and active. CLI on the mock daemon: a loop via `input add --junction`, via `apply`, via `apply --dry-run` and via a hand edit is refused with `E_CONFIG_CYCLE` (`a → b → a`), the file and revision unchanged, and the hand edit leaves the last good config in use. Live: seeded noise through five Junctions (four levels) arrives bit-exact (70 208 frames). The code is `E_CONFIG_CYCLE`, as for every config error.
- [x] **A15** [auto] `juncd.service`, D-Bus activation, `junc service`,
  sd_notify. Req: F9. Acceptance: in a systemd-user container, Junctions exist
  after login with no GUI; restore < 1 s for 10 Junctions.
  Done 2026-11-25: `packaging/systemd/juncd.service` (`Type=notify`, after PipeWire, no restart for unfixable exits) and the D-Bus activation file; juncd sends `READY=1` and a status line without libsystemd; `junc service enable|disable|status`. CI boots the Arch image with systemd as PID 1 on both arches: a lingering login with ten Junctions and no GUI has them routed 230–300 ms after login (juncd's own restore 17–40 ms), the unit verifies, D-Bus activation brings juncd back when stopped, `junc service` works, and a second login restores again. Found and fixed: juncd took its bus name before publishing its state, so the call that activated it saw an empty daemon; `doctor` and `service status` started juncd by asking about it.
- [x] **A16** [auto] Layer-2 integration suite in CI using the validation
  `ci-config` (fake devices, no bluez/logind). Req: F1–F5, F9, F10, F16,
  F17. Acceptance: runs on every PR in < 10 min on both arches.
  Done 2026-11-25: the suite (`scripts/ci/live-pipewire.sh`, validation `ci-config`) now also proves F3/F4 channel placement (4-ch Bus, Input on ch 3–4 bit-exact, ch 1–2 silent), recovery from a PipeWire restart (rebuilt in 72–79 ms in CI), F10 with 60 app restarts (p95 6–8 ms in CI) and 10 device re-plugs, and visibility to PulseAudio, JACK and native clients; every test cleans up after itself; architecture §11.2 maps each requirement to its test. In CI the suite takes 23–25 s and the whole check job 3–5 min on both arches; CI fails if the suite reaches 10 min.
- [x] **A17** [auto] Port `bench/` to drive the real `juncd`; daemon perf
  gates with a stored baseline (PSS, wakeups, re-link, recovery, CLI latency,
  binary size, 0 extra quanta, bit-exact). Req: PRD performance. Acceptance:
  gate fails on an injected regression.
  Done 2026-11-25: `crates/junc-bench` drives the real juncd (release, private D-Bus, ten Junctions, isolated PipeWire) and `scripts/ci/bench.sh` gates every PR against `bench/thresholds.toml` and per-arch baselines from the CI runners; CI checks the gate fails when a regression is injected. Found on the way: the Junction adds 0 frames (synchronous JACK clients, as in the validation); timing `pw-cat` streams reports 2048–4096 samples for any plain node and is not a latency measure. PipeWire restart recovery is ~8 ms of juncd's own work but 190–250 ms in CI and ~1.3 s locally from PipeWire's start, mostly WirePlumber; the gate takes juncd's share against the 1 s target and tracks the total. juncd's PSS on the aarch64 runner was 9.8–11.5 MB because glibc arenas of its helper threads were backed by huge pages; one malloc arena brings it to 5.5 MB.
- [x] **A18** [auto] *parallel* Skill draft: `SKILL.md`, `recipes.md`,
  `troubleshooting.md`, `reference.md` generator, `skill-check`, `junc skill
  link/unlink`. Req: F25. Acceptance: skill-check green; a coding agent with
  only the skill creates a Zoom mic from the CLI (dry-run shown first).
  Done 2026-11-25: `skill/junc/` (SKILL.md with the safe change loop and the PRD safety rules, recipes, config format, devices, defaults, troubleshooting) and `reference.md` generated by `cargo xtask skill-ref` from the CLI's own clap definitions and error table (now a library: `junc_cli::args`, `junc_cli::errors`); `cargo xtask skill-check` (frontmatter, links, every `junc …` line parses, every TOML example validates, reference fresh) runs in the test suite; `junc skill link|unlink|status` links into the agent folders that exist and keeps the user's own. Acceptance in an isolated sandbox: two runs of a fresh agent with only the skill built the Zoom mic from the CLI, dry run shown before applying, default mic untouched. Their notes improved the skill and two things in JUNC: a present source no longer shows `waiting` while its Junction is created, and plan summaries name non-default volume and exclusive. Not yet: volume is stored but not applied (B1), presets/templates (B8), linking on juncd's first start and packaging of the skill (A15 packaging follow-up). Repeat on the owner laptop in A19.
- [x] **A19** [owner plugs + sudo, auto runs] Alpha run on the owner's laptop:
  layers 2–3 (snd-dummy/snd-aloop), real USB interface as Input and Listen,
  Asahi default sink as Listen target, re-plug, PipeWire restart,
  suspend/resume. Req: F1–F5, F9, F10, F16, F17. Acceptance: results in
  `docs/hardware-matrix.md`; all pass.
  Done 2026-11-25 (owner plugged, listened, suspended; results in `docs/hardware-matrix.md`): live session with the Komplete Audio 2 as Input and Listen, the Asahi default sink as Listen, snd-aloop `pro-audio` bit-exact through a Junction (layer 3), hand edit, F17 override and Restore, PipeWire restart (all 38 links back 1.47 s after the restart command, mostly the audio stack starting), re-plug (re-linked within 100 ms), suspend/resume (juncd and Junctions fine; the Komplete did not re-enumerate after resume until re-plugged — USB/Asahi, not JUNC); the owner heard both test paths clean. Not all pass: F16 left a Junction as the mic in use when the configured default was unavailable — fixed (restore the device in use) and re-run live; re-link in the live session p95 81/90/101 ms over three runs, at the target's edge — profiling moved to beta (B29). The "graph complete" log now times from the change or reconnection.
- [x] **A20** [auto] Alpha exit review: tick criteria, list known issues,
  tag `v0.1.0-alpha` only after the owner says go. [owner] go/no-go.
  Done 2026-11-25: `docs/alpha-review.md` (criteria with evidence, PRD numbers, known issues). Owner: go; recovery target is JUNC's share. Tagged `v0.1.0-alpha`.

---

## Beta — GUI in the testing channel

Exit criteria (PRD): all v1 features in the GUI; all agent checks green;
owner walkthrough with the real USB interface. Packages in `junc-testing`.

Daemon features first (they gate GUI screens):

- [x] **B1** [auto] Gain nodes: per-Input and per-Listen volume, on/off,
  master volume; Listen routing of all or selected Bus channels. Req: F6,
  F7, F8. Acceptance: layer-2 tests; bit-exact at unity (or S4 fallback).
  Done 2026-11-25: the reconciler sets `channelVolumes` (cubic, 100 % = 1.0)
  on the Junction source, the Input and the Listen gain nodes when it first
  sees a node and when the config changes, never fighting another mixer.
  JUNC nodes carry `state.restore-props = false`: without it WirePlumber
  restored its saved 1.0 over JUNC's value right after the node appeared.
  On/off and Listen links came with the alpha planner. Layer-2 tests:
  Input 50 % → 0.125, × master 50 % → 0.015625, × Listen 70 % → 0.005359
  (within 2 %); Listen `links = [[1, 1]]` plays Bus channel 1 only; an
  Input switched off is silent and back at unity when on. Bit-exact at
  unity is unchanged (the existing null tests run through the gain nodes).
- [x] **B2** [auto] Exclusive via `target.object` metadata. Req: F11.
  Acceptance: layer-2 test with WirePlumber: app leaves speakers while
  captured, returns when turned off.
  Done 2026-11-25: an Exclusive Input sets its app streams' `target.object`
  to the Input's gain node; WirePlumber drops its link to the speakers and
  keeps JUNC's. Cables and Exclusive now share `plan::stream_targets`, so
  Exclusive gets the Cable's F17 behaviour: a move by the user is held and
  reported, turning Exclusive off (or the Input, or the Junction) returns
  only the streams still at JUNC's value. Config validation rejects
  `exclusive` on non-app Inputs. Layer-2 test with WirePlumber: the app
  leaves the speakers 47 ms after juncd starts, comes back 10 ms after a
  hand edit turns Exclusive off, and leaves again when it is turned on.
  A restarted daemon adopts streams already at a wanted target or at a
  JUNC Input gain node, so turning Exclusive off after a restart still
  returns them (unit test).
- [x] **B3** [auto] Pro Audio detection/switching, channel groups, profile
  memory on re-plug. Req: F15. Acceptance: layer-1 fixtures (18-ch) + layer-3
  `snd-aloop` on the owner laptop.
  Done 2026-11-25, daemon and CLI (architecture §3.8, as built): cards bound
  with their profiles, ALSA channel counts for USB cards, `devices` in
  `junc describe` (API 1.1), `junc input add --device … --pro-audio`,
  switching on plug and config change, the user's own choice kept and
  saved. Layer 1: synthetic 18-input USB card (2 of 18, groups 1–8, 9–16,
  17–18), Komplete Audio 2 negative case, snd-aloop "available", binding
  to the Pro Audio node, profile memory over a re-plug, card turned off.
  Layer 3 on the owner laptop (`docs/hardware-matrix.md`): snd-aloop
  switched to Pro Audio on sight, bit-exact through the real ALSA path,
  the user's own switch kept and saved. It found two bugs, both fixed and
  re-run: profile switches went unseen (now re-read on every parameter
  change), and the Asahi built-in UCM cards were offered Pro Audio (UCM
  cards are now never offered or switched).
- [x] **B4** [auto] Bluetooth profile detection and warning data. Req: F18.
  Acceptance: layer-1 fixtures from the owner's headset dump (0.7).
  Done 2026-11-25 (architecture §3.9, as built): `bluetooth` on headset
  mics and cards in `junc describe` (profile, playback/call, call codec and
  quality, `switches_to_call`), the warning on `junc input add`, the skill
  tells agents to ask first. Layer 1 on the AirPods and Pixel Buds 2a
  dumps: A2DP mics warn (mSBC wideband, LC3-SWB super-wideband from the
  call profile's description), HFP mics do not (codec from the nodes).
  Found on the way: candidates hid the device side of every loopback and
  filter chain, so the headset mic, Omarchy's mic and the Asahi DSP sink
  were not offered; fixed and tested on the Asahi dump.
- [x] **B5** [auto] Import of null-sink / pw-loopback setups. Req: F13.
  Acceptance: layer-2 test with `pactl load-module module-null-sink` and
  `pw-loopback` setups.
  Done 2026-11-25 (architecture §5, import as built): `junc import scan` /
  `junc import ID… [--as junction|cable]`, D-Bus `ScanImportable` (API 1.1).
  Layer 2 in the isolated session: a `pactl` null sink with Firefox
  playing into it became a Junction with a Firefox Input; a `pw-loopback`
  from a device to an output became a Junction with that Input and
  Listen; a null sink with one app playing and one recording became a
  Cable; the old modules kept running. Layer 1 on the real dumps: the
  Asahi DSP chains, Omarchy's mic and Bluetooth loopbacks are not offered.
- [x] **B6** [auto] Upgrade re-exec, `junc teardown`, pre_remove hook. Req:
  F20. Acceptance: counter-signal continuity test across re-exec; teardown
  leaves no `junc.managed` object.
  Done 2026-11-25 (architecture §3.13, as built): juncd re-execs itself
  when its executable is replaced and pacman's lock is gone; `junc
  teardown` with `juncd --teardown`; `packaging/arch/junc.install`
  `pre_remove`. Layer 2: a 6 s counter played through a Junction while the
  executable is replaced (lock held, then released) arrives with no gap
  (286 720 consecutive frames; the rest is pw-cat's last partial buffer),
  same PID, no JUNC object recreated; teardown leaves no `junc.managed`
  object and the default input comes back. The hook is tested with
  stand-in `loginctl`/`runuser`.
- [x] **B7** [auto] Capture indicator + `junc watch --json`. Req: F23.
  Acceptance: event stream tests.
  Done 2026-11-25 (architecture §5.2, as built): the daemon's
  `CaptureApps` (with change signals, since the alpha) feeds `junc watch`,
  which starts with the current state and streams captured apps,
  Junctions on/off/created/removed, graph changes, notices and the
  daemon's presence. Tests: on the mock daemon, Junction events for off,
  on, create and delete; live, an app routed into a Junction appears in
  `capture` and leaves it when the Junction is turned off, with graph
  events in between.
- [x] **B8** [auto] Presets/templates: export, import with slots, built-in
  set. Req: F22, F24. Acceptance: round-trip tests; every built-in template
  validates.
  Done 2026-11-25 (architecture §6.3, as built): `junc-core::preset` loads
  presets and templates with the config rules plus slot rules, fills slots
  and exports; the six built-in templates are in `templates/` and compiled
  in; CLI `junc template list|show`, `junc create junction --template`,
  `junc preset export|import`; skill `presets.md` (templates first). Tests:
  every built-in template validates, fills into a valid Junction and
  survives a save and load; export → import round trips every fixture
  Junction unchanged; slot mistakes are reported at their line; the CLI
  test fills two templates, exports and imports a Junction (same Inputs
  and Listens, not a second default input). Installing the templates to
  `/usr/share/junc/templates` comes with the PKGBUILD (B25).
- [x] **B9** [auto] Doctor complete (Asahi DSP, rates, jack2) + bug report
  with path scrubbing. Req: F19. Acceptance: golden outputs; scrub test.
  Done 2026-11-25 (architecture §5.2, as built): doctor reads facts once
  and checks them purely; new `rates` (devices that cannot run at the
  graph's rate, with the `clock.allowed-rates` fix) and `asahi` (DSP sink
  and speakersafetyd, Apple Silicon only) checks; jack2 worded as replace.
  `junc doctor --bug-report`: versions, graph line, checks, Junctions,
  sound cards, config, 200 journal lines, scrubbed of home paths, user and
  host names and hardware addresses; never starts juncd. Golden outputs
  for healthy, broken, rate-mismatch and Asahi sessions and the report;
  a scrub test. On the owner's M1 (read-only): all checks ok, `asahi`
  found `audio_effect.j293-convolver` and speakersafetyd, the report had
  no personal data.

GUI foundation:

- [x] **B10** [auto] Reference renders of all canvas boards in `docs/design/`
  (2026-11-25: 51 PNGs rendered locally from the artifact's files with
  headless Chromium; regeneration steps in `docs/design/README.md`).
- [x] **B11** [auto] `junc-l10n`: language resolution + Omarchy watcher,
  bundled translations, ICU4X numbers, `en_XA`, extraction and catalog
  checks; spike S6. Req: F14. Acceptance: live switch test; CI catalog checks.
  Done 2026-11-25 (architecture §8, as built): Omarchy's registry vendored
  (46 LTR languages in v1, RTL resolves to English); own `.mo` reader and
  plural evaluator (the `gettext` crate's `encoding` dependency fails
  cargo-deny); catalogs loaded per language and switchable in place;
  `en_XA` computed; inotify watcher on Omarchy's language file; ICU4X
  numbers behind a feature juncd does not use (its binary stays 3.1 MB);
  `cargo xtask i18n extract|check|build`, `po/cli/junc.pot`, a
  machine-drafted German catalog; the CLI speaks the chosen language and
  keeps `--json` English. Tests: live switch on Omarchy's file (in-place,
  rename, same code, removal, other file), language switch with German
  and Russian plurals, Arabic/Russian/French plural rules, msgfmt output,
  catalog checks, CLI in German/en_XA/`--json`. CI runs the catalog check.
  S6 passes on Slint 1.19.1 (`bench/spikes/s6/`): live switch with
  accessible labels, plurals with ICU4X-formatted counts; GUI catalogs need
  `msgctxt` = component name.
- [x] **B12** [auto] `junc-gui` skeleton: IPC client, AppState, ThemeService
  with live Omarchy reload and fallback themes, window sizing. Req: F21.
  Acceptance: theme switch test on the owner laptop (`omarchy-theme-set`)
  retints without restart.
  Done 2026-11-25 (architecture §7.7, as built): Spec tokens derived from
  `colors.toml`/`shell.toml`; theme service following `omarchy-theme-set`'s
  swap and edits in place, built-in Tokyo Night / Flexoki Light by the
  portal colour scheme otherwise; Slint window skeleton with the §7.1 sizes;
  IPC client and AppState following juncd live. Owner laptop (owner asked
  for every theme, not two): all 24 Omarchy themes applied in turn under one
  running window, sidebar colour pixel-exact for each, same PID; Osaka Jade
  ships no colours in Omarchy 4.5.0, so JUNC shows its built-in theme there.
  The font comes from fontconfig's monospace alias (the live run showed a
  proportional font before that fix). cargo-deny now flags "unmaintained"
  for direct dependencies only (owner decision below).
- [x] **B13** [auto] Component library per Spec with the a11y fixes and a
  component-gallery scenario. Acceptance: gallery screenshots match the Spec
  board; AT-SPI dump shows labels, switch description, focus rings.
  Done 2026-11-25 (architecture §7.2, §7.9, as built): 22 components in
  `ui/components/` from tokens only; `junc-gui --scenario gallery`;
  `scripts/gui/gallery-check.sh` (private Xvfb + AT-SPI session). Renders in
  Tokyo Night and Flexoki Light match the Spec board's section 2 by eye
  (`docs/design/gallery/`); the AT-SPI tree has every label, the switch's
  "on"/"off" description, slider values and ranges, list and radio items;
  five Tabs focus the first switch with a visible accent ring. The dump found
  a real defect the unit tests could not: sliders exposed no Value interface
  with a "50%" value (fixed: plain number). Keyboard and accessible
  behaviour tested on Slint's testing backend (4 tests). Orca on the owner
  laptop waits for real screens.
- [x] **B14** [auto] Scenario harness (`--scenario`, mock IPC), screenshot
  pipeline with goldens, AT-SPI CI job. Acceptance: one scenario end to end
  in CI on both arches.
  Done 2026-11-25 (architecture §11.3, as built): fixtures with a mock IPC
  that settles config + graph like juncd; offscreen snapshots with a bundled
  font subset; screenshot goldens with a tolerance and a negative test;
  AT-SPI tree goldens from a private Xvfb + AT-SPI session; both in
  `check.sh`. CI on x86_64 and aarch64: the `default` and `gallery`
  scenarios match the goldens made on the owner's M1, and their AT-SPI
  trees match. CI needed fontconfig before and now Xvfb, at-spi2-core,
  python-gobject, a font and winit's X11 libraries. One CI run saw
  `enable_stays_fast` at p95 271 ms on aarch64 (gate 250 ms); it passed on
  the next runs.
- [x] **B15** [auto] Meter pipeline + VisibilityService; spike S7. Req: F7,
  performance. Acceptance: CPU hidden 0 % gate in CI; on the owner laptop
  meters stop within 1 s when minimized or on another workspace.
  Done 2026-11-26 (architecture §7.6, as built): `junc_pw::meter`, one
  passive capture stream with a port per channel, disconnected entirely
  while paused; a gate from closing, Hyprland IPC and frame starvation; the
  sidebar meter per Junction on a ~7 Hz Slint timer that stops while
  hidden. CI gate (§11.3): a stand-in Hyprland moves the window away, the
  meter stream is gone within 1 s and the GUI takes 0 CPU ticks in 3 s.
  S7 on the owner's M1 (window on a temporary headless monitor, isolated
  PipeWire): meters stop 14–29 ms after another workspace or minimizing,
  0 CPU while hidden; frame starvation cannot see hidden windows on
  Hyprland (it draws them at 15 fps), the IPC does.

Screens, in design-canvas order (each: scenario fixture, golden in en/en_XA/
de/ja, AT-SPI dump, keyboard test):

- [x] **B16** [auto] Main, Flexoki, Narrow; source off, offline, link hover.
  Req: F4, F7, F8, F10.
  Done 2026-11-26 (architecture §7.2, §7.3, §8, §11.3, as built): the
  sidebar and the Main screen with per-channel meters, links and their
  hover tooltip; switches and Space toggle, sliders set volumes, a click on
  a link or Del on a port unlinks, through the same config edits as the
  CLI; Show Listen; the Input's optional `name` (schema). The GUI's own
  catalogs (de, ja machine-drafted) and en_XA, switched live with
  Omarchy's language; Slint's own locale pick (`en_US` → `en_XA`) is
  overridden. Boards 1–6 as scenarios with 24 goldens (en, en_XA, de, ja),
  AT-SPI goldens and a keyboard/pointer test. Every column metered: 0.6 %
  of a core visible (release, owner's M1, headless monitor), 0 hidden.
  Left for B18: Ctrl+1…9 and arrow keys between cards.
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
- [ ] **B29** [auto] Re-link latency in a busy desktop session: A19 measured
  p95 81/90/101 ms live (6–9 ms in CI). Split the observer's own batching from
  juncd's bind/sync gating, measure with a lighter observer, and fix what is
  juncd's. Req: F10. Acceptance: live-session p95 < 100 ms with margin over
  five runs on the owner laptop.
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
| GUI CPU with meters: 6.4 % of a core on the M1 with femtovg (target < 2 %), ~20 wakeups per meter tick from full-window repaints and Wayland round trips | Battery drain while JUNC is open; fails the PRD GUI target | B15 done: 0 % while hidden. B16 measured the whole Main screen metered: 0.6 % of a core (release, headless monitor), under the 2 % target; B26 gates it on the owner's own screen. |
| Re-link in the live session: p95 89 ms with the prototype (target < 100 ms); A19 measured 81/90/101 ms with the real juncd | Target missed on busier desktops | Profile in a busy session (B29): how much is the observer's own batching, how much juncd's bind/sync gating; a lighter observer for the measurement. |
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
| 2026-11-25 | The developer merges a PR once its CI is green, by fast-forwarding `main` (no GitHub merge commits, so every commit keeps the owner as author and committer); the owner reviews at will and signs off stages and releases | owner |
| 2026-11-25 | Alpha go: tag `v0.1.0-alpha` | owner |
| 2026-11-25 | The PRD recovery target (< 1 s after a PipeWire restart) is JUNC's share, from PipeWire and the devices being back; the whole stack's return is tracked, not gated | owner |
| 2026-11-25 | Cables deliver through `target.object` and WirePlumber's linking instead of JUNC links into the target app (spike S11) | developer |
| 2026-11-25 | cargo-deny raises "unmaintained" for JUNC's direct dependencies only; vulnerabilities and yanked crates fail anywhere. Slint 1.19.1 brings unmaintained chrono, pin-utils, ttf-parser, zvariant and zbus_names | owner |
| 2026-11-25 | D-Bus carries structured values (Junction view, `Describe`, diagnostics, plans, notices) as JSON strings with the `--json` shapes instead of `a{sv}`; API v1.0 ships the calls the CLI needs, the rest arrive as minor versions | developer |
