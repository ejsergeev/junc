# JUNC — technical architecture

Status: design for v1, before alpha. Source of truth for requirements is the
PRD ("PRD Junc — Audio Routing for Linux (Arch first)", rev. 2026-11-24);
requirement IDs (F1–F25) refer to it. Visual and behavioural spec: the design
canvas "JUNC for Omarchy" (`Main.dc.html` scenarios, `Spec.dc.html`). Evidence:
validation report 2026-11-24 (`junc-validation/report.md`).

Keep this file current. A change that alters an interface described here
(D-Bus, CLI, config schema, PipeWire object layout) updates this file in its
own `docs:` commit next to the code change.

Contents

1. [Principles](#1-principles)
2. [Repository layout and crate boundaries](#2-repository-layout-and-crate-boundaries)
3. [Daemon design](#3-daemon-design)
4. [D-Bus API v1](#4-d-bus-api-v1)
5. [CLI](#5-cli)
6. [Configuration, presets and templates (schema v1)](#6-configuration-presets-and-templates-schema-v1)
7. [GUI architecture (Slint)](#7-gui-architecture-slint)
8. [Localization](#8-localization)
9. [Agent skill](#9-agent-skill)
10. [Packaging, repository and Omarchy integration](#10-packaging-repository-and-omarchy-integration)
11. [Test strategy](#11-test-strategy)
12. [Verification spikes](#12-verification-spikes)

---

## 1. Principles

1. **JUNC never touches audio samples on the audio path.** `juncd` only creates
   and configures native PipeWire objects (server-side adapter nodes, links,
   metadata, device params). PipeWire's real-time thread does all processing.
   `juncd` has no data thread and no `module-loopback` (validation: 0 vs ~140
   wakeups/s, 1.7 vs ~5 MB, leak on reconnect).
2. **Every object JUNC owns is tagged and lingers.** Nodes and links are
   created through the server's factories with `object.linger = true` and a
   `junc.*` property set. JUNC only ever modifies or destroys objects carrying
   its tag (F17). Because they linger, a daemon restart or upgrade does not
   drop audio (F20).
3. **Declarative core.** The desired state is the config (TOML). The daemon
   continuously reconciles the PipeWire graph toward it. GUI, CLI, agents and
   hand edits are all just ways of changing the desired state.
4. **Event-driven, idle = asleep.** No polling timers in steady state. All
   loops block on file descriptors (PipeWire, D-Bus, inotify). Target: 0
   wakeups/s when nothing changes.
5. **Platform-agnostic core.** `junc-core` has no PipeWire, D-Bus or UI
   dependency. PipeWire is one implementation of a `Backend` trait; macOS and
   Windows ports replace the backend and the platform shell only.
6. **One component library.** Every GUI screen is built from JUNC components
   (`JRow`, `JSlider`, `JSwitch`, …) that read semantic tokens only, so theme
   switching, RTL (after v1) and other platforms are component changes.

---

## 2. Repository layout and crate boundaries

```
junc/
├── Cargo.toml                    workspace (resolver 3, edition 2024, MSRV pinned)
├── crates/
│   ├── junc-core/                platform-agnostic: model, config, planner
│   ├── junc-ipc/                 D-Bus contract: zbus proxies, DTOs, error codes
│   ├── junc-pw/                  PipeWire backend (pipewire-rs), GUI meters
│   ├── junc-l10n/                language resolution, catalogs, ICU4X numbers
│   ├── juncd/                    daemon binary
│   ├── junc/                     CLI binary  (crate name junc-cli, bin "junc")
│   └── junc-gui/                 Slint GUI binary + tray
├── ui/                           .slint sources (components/, screens/, app.slint)
├── po/                           gettext catalogs: gui/ cli/ (+ templates .pot)
├── schema/junc-config-v1.json    generated JSON Schema (checked in, CI-verified)
├── templates/                    built-in templates (F24), TOML
├── skill/junc/                   agent skill: SKILL.md + topic guides (F25)
├── omarchy/
│   ├── plugin/                   Quickshell bar widget (QML, F21/F23)
│   └── menu/                     menu snippet + upstream PR text
├── packaging/
│   ├── arch/PKGBUILD             split package junc + junc-gui
│   ├── arch/junc-keyring/        keyring package for the signed repo
│   ├── systemd/juncd.service     user unit
│   ├── dbus/io.github.ejsergeev.Junc1.service   D-Bus activation
│   ├── desktop/junc.desktop, icons/ (from design AppIcons board)
│   └── repo/                     repo layout docs + publish scripts
├── tests/
│   ├── fixtures/graphs/          mocked graphs (layer 1), one per device shape
│   ├── fixtures/scenarios/       GUI scenario fixtures (one per design state)
│   ├── fixtures/configs/         valid/invalid configs + migration fixtures
│   ├── integration/              headless PipeWire tests (layer 2)
│   └── golden/                   screenshot goldens per scenario × language
├── bench/                        perf harness (from junc-validation/bench)
├── scripts/
│   ├── ci/                       CI entry points (also runnable locally)
│   ├── pw-session.sh             start/stop/restart PipeWire (from validation)
│   └── hw/                       layer-3/4 helpers (snd-dummy, snd-aloop, UAC2)
├── xtask/                        cargo xtask: schema, i18n extract/check, goldens
└── docs/                         architecture.md, plan.md, dbus.md (generated),
                                  hardware-matrix.md, compatibility.md
```

### Dependency rules

| Crate | May depend on | Must not depend on |
| --- | --- | --- |
| `junc-core` | serde, toml_edit, schemars, thiserror, slug | pipewire, zbus, slint, any I/O beyond `std::fs` in loaders |
| `junc-ipc` | junc-core, zbus, serde_json | pipewire, slint |
| `junc-pw` | junc-core, pipewire (pipewire-rs 0.10.x), libspa | zbus, slint |
| `junc-l10n` | gettext (pure-Rust `.mo` reader), icu_decimal/icu_locale, inotify | pipewire, slint |
| `juncd` | junc-core, junc-ipc, junc-pw, junc-l10n (notices only), inotify, sd-notify | **slint or any GUI crate** |
| `junc-cli` | junc-core, junc-ipc, junc-l10n, clap | pipewire (talks to the daemon only; `describe` comes from D-Bus) |
| `junc-gui` | junc-core (types), junc-ipc, junc-pw (meter feature only), junc-l10n, slint, ksni | — |

CI enforces the table with `cargo deny` bans plus a `cargo tree` check per
binary. The daemon binary target is < 5 MB stripped (`opt-level = "s"` for
juncd; `lto = "fat"`, `codegen-units = 1`, `strip`, and `panic = "abort"` for
the whole release profile, since Cargo cannot set `panic` per package).

### `junc-core` modules

- `model` — `Junction`, `Input`, `Bus`, `Listen`, `LinkSpec`, `Cable`, ids.
- `config` — TOML load/save with `toml_edit` (comments and order kept),
  schema version, migrations, span-aware diagnostics (file, line, column).
- `schema` — `schemars` types; `junc schema` and `schema/*.json` come from here.
- `preset` — preset/template format, slot filling (F22, F24).
- `graph` — backend-neutral view of the live graph: `Node`, `Port`, `Link`,
  `Device`, `Profile`, with properties as a string map.
- `matcher` — app/device identity matching (F2, F10).
- `planner` — `plan(desired, observed) -> Vec<Action>`: pure, deterministic,
  the heart of reconcile and of `apply --dry-run`.
- `backend` — `trait Backend` (create/destroy node and link, set props/params,
  set metadata, subscribe to graph events) and `MockBackend` (layer 1 tests).
- `diag` — `Diagnostic { code, severity, message, location, hint }` used by
  validate, doctor, errors and banners.

---

## 3. Daemon design

### 3.1 Process model

One process, two event loops, no timers in steady state:

```
┌──────────────────────────── juncd ─────────────────────────────┐
│  PipeWire thread (pw MainLoop)        Control thread (async)    │
│  ├─ registry listener ──events──▶     ├─ zbus object server     │
│  ├─ core proxies (create/destroy)◀──actions── reconciler        │
│  ├─ metadata "default" proxy          ├─ config watcher (inotify)│
│  └─ socket watch (inotify on          └─ self-upgrade watcher   │
│      $XDG_RUNTIME_DIR/pipewire-0)        (inotify on /usr/bin)  │
└────────────────────────────────────────────────────────────────┘
```

- The PipeWire thread owns every pipewire-rs object (they are `!Send`). It
  talks to the control thread over `pipewire::channel` (into the loop) and a
  `async-channel` (out of it).
- The control thread runs zbus on `async-io` and owns `junc-core` state:
  desired config, observed graph, planner, notices.
- `sd_notify(READY=1)` after the first successful reconcile, so
  `systemctl --user start juncd` returns when Junctions exist.

### 3.2 PipeWire object layout per Junction

Validated shape: server-side adapter nodes + links (report §4,
`juncd-links` mode). One Junction with *I* Inputs, a Bus of *C* channels and
*L* Listen outputs creates:

| Object | Factory / class | Count | Purpose |
| --- | --- | --- | --- |
| Junction source (the Bus) `junc.<slug>` | `adapter`, `factory.name = support.null-audio-sink`, `media.class = Audio/Source/Virtual` (spike S1) | 1 | What apps record from: the "virtual mic". `node.description` = Junction name. Bus = its channels. Master volume (F7) = its `channelVolumes`. Inputs are linked into its input ports. |
| Junction sink `junc.<slug>.sink` | same factory, `media.class = Audio/Sink`, same channels | 1 | What apps can play into (F1, "Use as default output" F16). Its monitor ports are linked into the Junction source, so whatever is played into it reaches the Bus like any other Input. Always present, like a Loopback device (owner decision 2026-11-25). |
| Input gain node `junc.<slug>.in.<input-id>` | `adapter`, null-audio-sink, `media.class = Audio/Duplex/Internal` (spike S2), `monitor.channel-volumes = true` | *I* | Per-Input volume and on/off (F7, F8) without touching the app's own volume. Not listed as a device (still visible in JACK patchbays, like filter-chain nodes). |
| Listen gain node `junc.<slug>.listen.<n>` | same as above | *L* | Per-Listen volume and on/off. |
| Links | link factory | as configured | app → input gain → Junction source; Junction sink monitor → Junction source; Junction source outputs → listen gain → output device. |

Properties on every node and link JUNC creates:

```
object.linger      = true
junc.managed       = true
junc.schema        = 1
junc.junction      = <junction id>
junc.role          = junction | input-gain | listen-gain | link
junc.ref           = <stable id: input id, listen id, or link id "in:spotify/1->bus/1">
node.autoconnect   = false          (nodes)
node.dont-reconnect = true          (nodes)
state.restore-props = false         (nodes)
```

`state.restore-props = false` keeps WirePlumber from restoring a volume it
saved for the node name over the one JUNC sets: the config is the only
place JUNC volumes live. JUNC sets a node's `channelVolumes` (cubic, 100 % =
1.0, so unity stays bit-exact) when it first sees the node and whenever the
config changes it; a change made in another mixer is not fought, only
replaced on the next change or when the node is recreated.

Gain nodes are as wide as the Bus: an Input's channels link straight into
the gain-node channels of the Bus channels they feed (a mono mic links its one
channel into channels 1 and 2), and the gain node feeds the Bus one to one.
So an Input's volume is one node's volume whatever its channel count, and the
link layout does not depend on the source. A disabled Junction keeps both its
nodes (apps keep their device and hear silence) and drops Input and Listen
links.

Cost is constant per configured item (no per-buffer work by JUNC). A plain
route at unity gain must stay bit-exact (null test) and add 0 quanta; this is
a CI gate (§11). If spike S4 shows the gain stage is not bit-exact at unity,
the fallback is to link the app straight into the Junction while the Input is
at 100 % and insert the gain node only when volume ≠ 100 % (one re-link,
accepted glitch on the first volume change).

A **Cable** (F5) is a Junction with `kind = "cable"`, one app Input, a 2-ch
Bus, no Listen by default, and a *target app*. JUNC sets the target app's
capture streams' `target.object` (in the `default` metadata) to the Cable's
source node, the way moving a stream in pavucontrol does, and WirePlumber
links it there and drops its default-mic link (spike S11). JUNC makes no
link into the target app itself: next to WirePlumber's routing the app
would hear the mic and the Cable mixed, and WirePlumber removes links to a
stream's old target when it moves the stream. The app's own device picker
shows the Cable as its input. The reconciler remembers which streams it
sent: a value someone else writes later is the user's choice, reported as
`link_overridden` with reference `cable/target/<object.serial>` and held
(Restore re-sends it; Leave as is holds it until the app quits, since a
stream has no place in the config). When the Cable is deleted, turned off
or no longer matches the app, JUNC clears only the values it wrote; a
stream the user sent to a Junction themselves is left alone.

**Nesting** (F2): a Junction may be an Input of another
(`match = { junction = "<id>" }`): the inner Junction's source feeds the
outer one's gain node like any device. Config validation builds the
directed graph of Junctions and rejects a loop (`E_CONFIG_CYCLE`, F2 risk
"feedback loops", reported from the first Junction of the loop in file
order) before anything reaches the store or PipeWire, whether the change
comes from the CLI, `Apply` or a hand edit. As built (A14): depth 1–4
converges in at most three reconcile rounds and is bit-exact through four
levels on the isolated PipeWire.

### 3.3 Graph model and identity matching

`junc-pw` turns registry globals into `junc-core::graph` objects:
`Node { id, name, class, props, ports }`, `Port { id, node, direction,
channel, monitor }`, `Link { id, out_port, in_port, props }`,
`Device { id, name, profiles, active_profile, bluez }`, plus the `default`
metadata (configured/actual default sink and source).

Inputs and Listen targets are stored by *identity*, never by node ID (F2):

| Kind | Match keys, in priority order | Notes |
| --- | --- | --- |
| App stream | `application.process.binary`, `application.name`, `media.role` (optional), `node.name` (optional regex) | All given keys must match. A match can bind several streams (e.g. a browser with two tabs); all are linked. |
| Hardware input/output | `node.name` (stable, e.g. `alsa_input.usb-Focusrite_…`), fallback `device.serial` + `audio.position`, fallback `node.description` | `default` = follow the system default sink/source. |
| Sink monitor | sink `node.name` + `monitor = true` | "System audio" (F2). |
| Junction | Junction `id` | Nesting. |

Ports are matched by `audio.channel` (FL, FR, AUX0…) with `port.id` order as
fallback, so a 2-channel app maps to Bus Ch 1–2 by default (F4).

### 3.4 Reconcile and link ownership

`planner::plan(desired, observed)` returns an ordered action list:
`CreateNode`, `CreateLink`, `DestroyLink` (only `junc.managed`),
`DestroyNode` (only `junc.managed`), `SetMetadata`, `SetDeviceProfile`. The
planner never emits an action on an object without the JUNC tag; a unit test
enforces this over every fixture graph (F17).

Implemented in `junc-core::plan` with `MockBackend` for tests. Desired and
existing JUNC links are compared by their port pair; `junc.ref` names each
link (`in/<input>/<from>-><to>`, `gain/<input>/<ch>`, `sink/<ch>`,
`listen/<id>/…`; a Cable's target stream is held as `cable/target/<serial>`).
Links are planned only once both nodes' ports
exist, so a new Junction converges in two rounds. Links in the *held* set
(removed by another tool, awaiting Restore / Leave as is) are not recreated,
and no JUNC link is added where a foreign link already joins the same ports.
A property test over random configs on six recorded graphs checks that only
`junc.managed` objects are ever destroyed, that planning converges, and that
an empty config restores the original graph.

Reconcile runs on: registry change (debounced to the end of the current
PipeWire batch with a `core.sync` round trip, not a timer), desired-state
change, reconnect. Measured in the prototype: stream appears → link created
p50 3.8 ms, max 14.9 ms (target < 100 ms).

### 3.5 Link state machine (F10, F17)

Each desired link has a state:

```
          endpoint(s) missing                 both endpoints present
 ┌──────────┐ ───────────────▶ ┌──────────┐ ─────────────────▶ ┌──────────┐
 │ Disabled │                  │ Waiting  │                     │ Creating │
 └──────────┘ ◀─ input/junction└──────────┘ ◀── endpoint gone ──└────┬─────┘
       ▲            turned off       ▲                              │ global added
       │                             │ endpoint gone                ▼
       │                        ┌────┴─────┐  removed by someone ┌──────────┐
       │                        │Overridden│ ◀─────────────────── │  Active  │
       │   "Leave as is"        └────┬─────┘   else, endpoints    └──────────┘
       └─────────────────────────────┘          still present         ▲
                                      "Restore" ─────────────────────┘
```

- **Waiting** shows as "Offline · Waiting for device…" (hardware) or
  "Not running · Waiting for Spotify…" (app), links drawn dashed (design
  states `offline`, `appClosed`).
- Implemented in `junc-core::reconcile` (state across snapshots) on top of
  the planner. Objects are tracked by registry id **and** `object.serial`,
  because PipeWire reuses ids. The backend sends a snapshot only after every
  newly bound node and link has reported its full props, so JUNC's own new
  objects never look untagged (which would make the planner duplicate them).
- **Overridden**: a JUNC link disappeared while both endpoints still exist
  and JUNC did not destroy it. Detection: the link is held at once (not
  recreated); if both endpoint nodes are still present in the next snapshot
  it was another tool and a notice is raised, otherwise (an app quitting
  removes its links before its node) the hold is dropped silently. Links
  between JUNC's own nodes are always restored. JUNC does not fight: it raises a `LinkOverridden`
  notice ("EasyEffects removed the Link …", culprit from the client that
  owns the replacing link when known) with **Restore** / **Leave as is**
  (design state `linkOverridden`). "Leave as is" adds the link's channel pair to the `suppressed` list of
  that link in the config, so the choice survives restarts and is editable.
- JUNC never destroys links it did not create, including WirePlumber's
  app→default-sink links. Non-exclusive capture *adds* links (an app plays to
  its device and into the Junction).

### 3.6 Connection and recovery state machine

```
Disconnected ──socket appears / poll──▶ Connecting ──ok──▶ Syncing ──▶ Reconciling ──▶ Steady
     ▲                                     │ fail                                     │
     └──────────── core error / EPIPE ─────┴──────────────────────────────────────────┘
```

- **Disconnected**: watch `$XDG_RUNTIME_DIR/pipewire-0` with inotify
  (`IN_CREATE`) instead of polling; if inotify is unavailable, fall back to the
  validated poll (20 ms for 2 s, then 500 ms) with the backoff **reset after
  every successful connection** (validation bug fix).
- **Syncing**: full registry round trip; *adopt* every `junc.managed` object
  that survived (lingering), index them by `junc.ref`.
- **Reconciling**: diff and apply. Objects with a `junc.ref` that no longer
  exists in the config are destroyed; matching ones are updated in place.
- PipeWire restarts destroy lingering objects; the daemon recreates them
  (measured 368–408 ms after a restart, target < 1 s). Suspend/resume keeps
  PipeWire alive; devices disappear and reappear and go through Waiting.
- D-Bus property `State`: `waiting_for_pipewire` drives the GUI's
  "Can't reach the audio service" pane (design `serviceDown`).

### 3.7 Default device handling (F16)

- "Use as default input/output" writes `default.configured.audio.source` /
  `.sink` in the `default` metadata to the Junction's `node.name` (same as
  `wpctl set-default`; WirePlumber persists it).
- Before the first change, the previous configured default is saved in
  `~/.local/state/junc/state.toml`.
- On disable or delete of the default Junction: if the configured default
  still points at it, restore the saved value (if that node exists; else
  clear the key and let WirePlumber choose). Emit `DefaultRestored` (toast
  with Undo, design `defaultRestored`).
- Implemented in `junc-core::defaults` (driven by the reconciler) and
  persisted by `juncd` in `$XDG_STATE_HOME/junc/state.toml`. The `default`
  metadata belongs to WirePlumber, so snapshots lag behind a change: setting
  and restoring each wait until the snapshot shows the change before deciding
  again (otherwise a stale snapshot looks like a user choice).
- JUNC never fights the user: if the configured default is changed elsewhere
  while a Junction holds it, JUNC reports `default_taken_over`, does not take
  it back while the config still asks for that Junction, and treats the new
  choice as the baseline (no restore later).
- A Listen target `default` follows the system default sink: the daemon
  re-links when `default.audio.sink` changes. On Asahi Macs the default sink
  is a DSP filter-chain (`audio_effect.j293-convolver`) in front of the
  speakers; JUNC must route to that, never to raw speaker nodes. Step 0
  showed the raw speaker and mic-array nodes are not even visible to a
  normal client (only the links to them are), so this holds by construction
  on current Asahi; S8 keeps a test for it. The output picker lists only
  nodes WirePlumber exposes as `Audio/Sink`, skipping `node.hidden`/internal
  filter-chain stream nodes.

As built (A19 fix): JUNC also remembers the default **in use** when it
differs from the configured one (the configured device was unavailable and
WirePlumber had picked another). If, after restoring the configured value,
WirePlumber still uses a Junction (it picks the newest available device in
its history), JUNC makes the device that was in use the default instead.
Found on the owner's laptop: configured headset jack unplugged, built-in
mic in use.

### 3.8 Pro Audio profile switching (F15)

- For each ALSA device the backend reads `EnumProfile`/`Profile` params.
  PipeWire reports channel counts only for the nodes of the **active**
  profile; an inactive `pro-audio` entry lists device classes but no channel
  count (step 0). The full count therefore comes from a read-only look at
  ALSA's stream description for USB cards (`/proc/asound/card<N>/stream*`,
  card number from `api.alsa.card`), never by switching profiles to probe.
  If that count exceeds the active profile's channels, the picker shows
  "2 of 18 channels" and "Use all channels" (design `proAudio`). If the count
  is unknown (non-USB cards), the row says "Pro Audio available" and offers
  the switch without a number. Equal counts (e.g. Komplete Audio 2, 2 = 2)
  show nothing.
- "Use all channels" sets the device `Profile` param to `pro-audio`, waits
  for the new nodes, and binds the Input to the pro-audio capture node with
  all channels grouped by 8 (design `multichannel`: In 1–8, 9–16, 17–18).
- The chosen profile is recorded on the Input's device match
  (`profile = "pro-audio"`) and re-applied when the device re-appears, unless
  the user changed the profile elsewhere while it was plugged in (then the
  config is updated, not overridden).
- Shows device channel count and active profile as a badge.

As built in B3 (daemon and CLI; the picker is the GUI's, B9+):

- The backend binds every card, subscribes to `EnumProfile` and `Profile`,
  and for USB cards reads `/proc/asound/card<N>/stream*` once when the card
  appears (`proaudio::parse_usb_stream`). `junc describe` lists `devices`
  with profile, input channels, groups of 8 and `pro_audio: {now, all}`.
- `junc input add … --device NODE --pro-audio` stores `profile =
  "pro-audio"` with the device match. The match may keep the stereo node
  name: the card comes from the node, or from the name when the node is
  gone (`alsa_input.<card>.<part>` → `alsa_card.<card>`). While the card
  runs another profile the Input waits; then it takes the card's widest
  capture node of that profile.
- The reconciler switches the card (`SetDeviceProfile`, saved by
  WirePlumber like `wpctl set-profile`) once per card appearance (by
  `object.serial`) or config change. If the user switches it elsewhere
  while it stays plugged in, the Input's `profile` becomes their choice
  (saved like any edit, `profile_changed` notice), so a re-plug brings
  that back; a card turned `off` is left off and the config kept. One
  profile per card: the first Input in the config wins.
- Never on UCM cards (profiles like `HiFi` instead of ACP's
  `output:…+input:…`): UCM sets up amplifiers and speaker processing, as
  on the Asahi built-in audio, and Pro Audio would bypass it. Not offered,
  not switched even when a config asks.
- Profiles are re-read on every parameter change of the card: a
  parameter subscription alone does not report a switch.

### 3.9 Bluetooth (F18)

From `device.api = bluez5` and `api.bluez5.profile`: the picker marks a
headset mic that is in A2DP with the HFP warning and offers "Add anyway" /
"Choose another mic" (design `btWarning`); the active profile is a badge on the
device card. JUNC never switches the Bluetooth profile itself; adding the mic
lets WirePlumber switch.

Step 0 detail: with WirePlumber 0.5 a headset in A2DP already exposes a
virtual mic `bluez_input.<address>` (plus `bluez_capture_internal`); linking
it is what triggers the switch to HFP. The warning is attached to exactly
that node. Call quality differs by codec (AirPods mSBC, Pixel Buds 2a
LC3-SWB), so the warning names the codec class from `api.bluez5.codec` of the
HFP profile where known.

As built in B4 (`junc_core::bluetooth`; the picker dialog is the GUI's):
`junc describe` gives headset mics and Bluetooth cards a `bluetooth` object
with the profile, `mode` (`playback`/`call`), the call codec and its class
(CVSD narrowband, mSBC wideband, LC3-SWB super-wideband) and, on a mic in
A2DP, `switches_to_call`. In A2DP the codec comes from the description of
the `headset-head-unit` profile WirePlumber would pick; in a call, from the
nodes. `junc input add` with such a mic adds it and prints the warning.
The headset mic is the device side of a loopback (`node.link-group`), as
are the Asahi DSP sink and mic and Omarchy's mic: candidates hide only the
stream side of loopbacks and filter chains (they hid both before B4).

### 3.10 Exclusive (F11)

Exclusive moves the app's stream to its Input gain node instead of adding a
second link: set the stream's `target.object` in the `default` metadata to the
gain node (the same mechanism as moving a stream in pavucontrol). WirePlumber
then removes its own link to the speakers and leaves JUNC's link in place
(spike S5). Turning Exclusive off clears the
metadata key and WirePlumber restores its default routing. JUNC still never
destroys a foreign link itself.

Cables and Exclusive share one mechanism (`plan::stream_targets`): JUNC sets
the key once the gain node (or the Cable) exists, clears it only on streams
whose value is still the one JUNC wrote, and treats a value someone else
writes as an override (held, `LinkOverridden` notice, reference
`exclusive/target/<object.serial>`). A stream has one target: if two
Junctions capture the same app exclusively, the first in the config wins and
the other still receives it through its own link. Exclusive is valid on app
Inputs only; the config rejects it on device, monitor and Junction Inputs.
A restarted daemon does not remember what it wrote, so it adopts streams
already at a target it wants and streams sent to any JUNC Input gain node
(gain nodes are not listed as devices, so only JUNC sends streams there).

### 3.11 Capture indicator data (F23)

The daemon computes `CaptureApps`: the set of app streams (`Stream/Output/Audio`
nodes from applications, plus `Stream/Input/Audio` for capture-from-app
cables) that have an **Active** JUNC link into an **enabled** Junction. It is a
D-Bus property with change signals, consumed by the bar plugin and the tray
(6 px urgent dot, "Capturing: Spotify, Soundux, Firefox").

### 3.12 Config watching and hand edits (F22)

- `~/.config/junc/` is watched with inotify (`IN_CLOSE_WRITE`, `IN_MOVED_TO`
  on the directory, since editors replace files).
- A changed file is parsed and validated. Valid → becomes desired state,
  reconcile. Invalid → keep the last good config (also persisted as
  `~/.local/state/junc/last-good.toml`), raise `ConfigError` with file, line,
  column and hint (design `configError` banner).
- The daemon writes the config only through `toml_edit`, preserving comments,
  order and formatting of untouched tables, using write-to-temp + `rename`.
  Its own writes are recognised by content hash and do not re-trigger.
- A monotonically increasing `revision` lets GUI, CLI and agents detect
  concurrent edits (`E_CONFIG_CONFLICT`).
- Implemented in `junc-core::store` and `juncd`: writes apply only the
  old→new difference to the `toml_edit` document (arrays of tables matched
  by `id`), defaults the user never wrote stay unwritten, and every write is
  read back and must equal the requested config or nothing is written.
  Comment-only edits do not bump the revision. Removing the file does not
  remove Junctions; starting with an invalid file uses `last-good.toml`.

### 3.13 Upgrade without audio drop (F20)

- All JUNC nodes and links linger, so they survive the daemon process.
- `juncd` watches its own executable path. When pacman replaces it (inode
  change) and the pacman lock `/var/lib/pacman/db.lck` is gone, the daemon
  finishes the current reconcile, flushes state, and `execve`s itself. The new
  process adopts every tagged object (§3.6 Syncing), so no node or link is
  recreated and audio does not drop.
- A CI test plays a continuous counter signal through a Junction, re-execs
  the daemon, and checks the recording for gaps (§11).
- The GUI shows "JUNC updated to 1.1 · audio kept running" when it sees a new
  `DaemonVersion` (design `updated`).
- **Uninstall**: `junc teardown` removes every `junc.managed` object and
  disables the user unit; the package's `pre_remove` runs it for every user
  with an active session (`systemctl --user -M <user>@` + `runuser`), best
  effort. Config in `~/.config/junc` is left for reinstall. Anything missed
  disappears at the next PipeWire restart because nothing re-creates it.

As built in B6: the upgrade watcher is an inotify thread on the
executable's directory (no timers; it polls the pacman lock only while a
transaction runs, `JUNC_PACMAN_LOCK` overrides the path for tests). The
daemon saves its state and `exec`s the same path with the same arguments;
the new process logs that it was re-executed and adopts the graph like any
restart (same PID, so systemd sees no restart). `junc teardown` disables the
unit, refuses while a juncd started outside systemd still runs, then runs
`juncd --teardown`: the daemon with an empty config and no D-Bus, which
removes every JUNC object, returns streams it moved and restores the saved
defaults, then exits (10 s limit). The config file is not touched. The
`pre_remove` hook is `packaging/arch/junc.install`; it runs `junc teardown
--yes` through `runuser` for each user `loginctl` lists with a session bus,
with that user's `HOME` and runtime directory.

### 3.14 Service and activation

- `juncd.service` (user unit, `packaging/systemd/`): `Type=notify`,
  `After=pipewire.service wireplumber.service`, `Wants=pipewire.service`,
  `Restart=on-failure` except for exits a restart cannot fix
  (`RestartPreventExitStatus=2 3 4`: bad arguments, no usable config,
  another juncd owns the bus name), `WantedBy=default.target`, enabled by
  the "Start at login" setting (F9; `junc service enable`; on by default
  after onboarding).
- Readiness: juncd sends `READY=1` once it serves D-Bus and has loaded
  the config (callers can talk to it from then on; PipeWire may come
  later), and a one-line `STATUS=` for `systemctl status`. One datagram to
  `$NOTIFY_SOCKET`, no libsystemd.
- D-Bus activation file (`packaging/dbus/`, `SystemdService=juncd.service`)
  so any CLI/GUI call starts the daemon if needed. juncd serves its
  objects with their first state before taking the bus name, so the call
  that activated it never sees an empty daemon. Commands that only report
  on the daemon (`doctor`, `service status`) ask the bus whether the name
  has an owner instead of calling it, so they do not start it.
- Logs go to the journal (`journalctl --user -u juncd`). English only.
- Runs as the user; no root, no kernel modules (PRD security).
- Tested in CI with systemd as PID 1 (`scripts/ci/systemd-user.sh`; the
  image gets its packages before it boots, since a booted Arch Linux ARM
  image replaces Docker's DNS): a lingering login with no GUI, D-Bus
  activation, `junc service`, logging out and in. From login to ten
  Junctions routed takes 230–300 ms including the user manager and
  PipeWire; juncd's own restore is 17–40 ms. On the isolated PipeWire, a
  restart adopts lingering objects without recreating any.

---

## 4. D-Bus API v1

Session bus only. Well-known name `io.github.ejsergeev.Junc1`; the trailing
`1` is the major API version. Reference with examples: `docs/dbus.md`;
introspection XML generated from the running daemon:
`docs/dbus/io.github.ejsergeev.Junc1.xml` (a test fails when it is stale).

**Decision (A10): structured values are JSON strings, not `a{sv}`.** The
Junction view, `Describe`, diagnostics, plans and notice bodies are JSON with
the same shapes `junc … --json` prints (`junc-ipc::view`). One schema serves
agents, the CLI and the GUI; `a{sv}` nesting for inputs with matchers and
statuses would be a second, harder-to-read encoding of the same data. Simple
values (`Revision`, `Enabled`, `Junctions`) stay native D-Bus types so
`busctl` and property bindings work without parsing.

### 4.1 Objects

| Path | Interfaces |
| --- | --- |
| `/io/github/ejsergeev/Junc1` | `io.github.ejsergeev.Junc1.Daemon`, `org.freedesktop.DBus.Properties` |
| `/io/github/ejsergeev/Junc1/junction/<id>` | `io.github.ejsergeev.Junc1.Junction`, `org.freedesktop.DBus.Properties` |

`<id>` is the Junction id with characters outside `[A-Za-z0-9]` encoded as
`_xx` (hex): `podcast-mix` → `podcast_2dmix`. Inputs, Listen outputs and
Links are not separate objects; they are part of the Junction's `View`
(keeps the object count and signal traffic small).

### 4.2 `Daemon` interface (v1.0)

Properties (all emit `PropertiesChanged`):

| Property | Type | Meaning |
| --- | --- | --- |
| `ApiVersion` | `(uu)` | (major, minor). Minor grows with additive changes. |
| `DaemonVersion` | `s` | Package version. |
| `State` | `s` | `starting`, `connected`, `waiting_for_pipewire` |
| `Revision` | `t` | Current config revision. |
| `ConfigError` | `s` | JSON diagnostics of an invalid file on disk; `""` when valid (the last good config stays in use). |
| `Junctions` | `ao` | Ordered as in the config. |
| `CaptureApps` | `as` | §3.11 |

Methods:

| Method | Signature | Notes |
| --- | --- | --- |
| `Describe` | `() → s` | JSON snapshot: Junctions with live status, candidates, defaults (same JSON as `junc describe --json`). |
| `GetConfig` | `() → (s toml, t revision)` | The last good file text. |
| `Validate` | `(s toml) → s` | JSON diagnostics; never changes anything. |
| `Plan` | `(s toml) → s` | Dry run: graph changes and resulting statuses as JSON. |
| `Apply` | `(s toml, t expected_revision) → (t revision, s plan)` | Validates, writes the text as given (comments kept), reconciles. `expected_revision = 0` skips the check. |
| `CreateJunction` | `(s name) → (o, t revision)` | Id from the name (`slugify`, made unique). Cables need their apps up front: use `Apply`. |
| `ResolveLink` | `(s junction, s ref, s action) → t` | Answer to `link_overridden`: `restore` or `leave` (writes `suppressed`). |

Signals:

| Signal | Args | Meaning |
| --- | --- | --- |
| `GraphChanged` | `(t generation)` | Live status changed (cheap; clients re-read what they show). |
| `Notice` | `(s kind, s json)` | `link_overridden`, `default_restored`, `default_taken_over`, `config_error`. |

Planned additive methods (minor versions, with the features that need them):
`ListCandidates`, `ImportPreset`, `ScanImportable`/`Import` (F13), `Doctor`
and `BugReport` (F19), `Defaults` and `PipeWire` properties, ObjectManager.

### 4.3 `Junction` interface (v1.0)

Properties: `Id` s, `Name` s, `Enabled` b, `View` s (JSON: kind, volume,
default, channels, node name, status, inputs with matchers, statuses and
matched nodes, listens).

Methods, each returning the new config revision `t`: `Rename(s)`,
`SetEnabled(b)`, `SetVolume(u percent)`, `SetDefault(s none|input|output)`,
`Delete(b confirm)`. Edits are merged into the file so the user's comments
and layout stay. Anything else (inputs, links, listens, bus channels) goes
through `GetConfig` + `Apply` in v1.0; per-entry methods (`AddInput`,
`Link`, `AddListen`, `Duplicate`, `ExportPreset`, …) arrive as minor
versions when the GUI needs them.

### 4.4 Errors

All errors are `io.github.ejsergeev.Junc1.Error.<Name>`; the message is JSON
`{"code","message","hint"?,"diagnostics"?}` (`junc_ipc::ErrorBody`) so CLI
and GUI can show actionable text and agents can parse it. The D-Bus name is
the coarse class; `code` is the stable detail (`E_CONFIG_CYCLE`,
`E_CONFIG_RANGE`, …):

| Name | When |
| --- | --- |
| `NotFound` | Unknown Junction or overridden link (`E_NOT_FOUND`). |
| `InvalidConfig` | Validation failed (`E_CONFIG_*`, incl. cycles and ranges); `diagnostics` carry file/line/column. |
| `Conflict` | `expected_revision` is stale (`E_CONFIG_CONFLICT`). |
| `PipeWireUnavailable` | The call needs the graph and the daemon is waiting for PipeWire. |
| `NeedsConfirmation` | `Delete(false)`. |
| `Failed` | Anything else (`E_WRITE_FAILED`, `E_INTERNAL`). |

### 4.5 Versioning and access

- Additive changes (new methods, properties, enum values) bump
  `ApiVersion.minor`. Clients ignore unknown JSON keys.
- Breaking changes ship as `Junc2` side by side for at least one minor
  release. Changes are announced in release notes (PRD).
- Session bus only; no system bus name, no polkit. The default session bus
  policy restricts access to the user's own processes.

---

## 5. CLI

Binary `junc`. Talks only to `juncd` over D-Bus (auto-activated). Global
flags: `--json`, `--quiet`, `--no-color`, `--lang <code>`. Messages for humans
are localized (F14); `--json` output and error codes are always English.

### 5.1 Commands

```
junc list                                  # Junctions and Cables with status
junc status [NAME]                         # one Junction in detail
junc describe                              # full snapshot for agents (F25)
junc create junction NAME [--template T --slot mic=…]
junc create cable FROM_APP TO_APP [--name NAME]
junc rename NAME NEW_NAME
junc duplicate NAME [--name NEW]
junc delete NAME --yes                     # refuses without --yes when not a TTY
junc enable|disable|toggle NAME
junc switch NAME [--as input|output]       # enable + make default (F12, story 7)
junc default NAME --as input|output | --clear
junc volume NAME [INPUT|--listen ID] PERCENT
junc input add NAME (--app BIN | --device NODE | --monitor SINK | --junction J) [--exclusive]
junc input remove|enable|disable NAME INPUT
junc bus add NAME [--mono] [--count N] | junc bus remove NAME CH
junc link NAME FROM TO | junc unlink NAME LINK_ID
junc listen add NAME (OUTPUT|default) | junc listen remove NAME ID
junc preset export NAME [-o FILE] | junc preset import FILE [--slot k=v]
junc template list | junc template show T
junc import scan | junc import ID… [--as junction|cable]          # F13
junc config path | junc config validate [FILE] | junc config edit
junc schema [--version 1]                  # JSON Schema (F25)
junc apply FILE [--dry-run] [--expect-revision N]
junc doctor [--bug-report]                 # F19
junc watch                                 # stream of JSON events (bar plugin)
junc service enable|disable|status        # Start at login (F9)
junc skill link|unlink|status              # F25
junc omarchy install|uninstall|status      # bar plugin + menu entry (F21)
junc teardown --yes                        # remove all JUNC objects (F20)
junc completions bash|zsh|fish
```

### 5.2 `--json` output shape

Every command prints exactly one JSON document (except `watch`, which prints
one per line):

```json
{ "junc": 1, "ok": true, "revision": 42, "data": { … } }
```

```json
{ "junc": 1, "ok": false,
  "error": { "code": "E_CONFIG_UNKNOWN_KEY", "message": "unknown key 'volum'",
             "location": { "file": "~/.config/junc/junctions.toml", "line": 14, "column": 1 },
             "hint": "Did you mean 'volume'? Allowed keys: id, name, enabled, volume, …" } }
```

`junc list --json` → `data.junctions[]`, the Junction view of
`junc_ipc::view` (the same JSON as the D-Bus `View` property):

```json
{ "id": "zoom-mic", "name": "Zoom Mic", "kind": "junction", "enabled": true,
  "volume": 100, "default": "input", "channels": 2, "node": "junc.zoom_mic",
  "path": "/io/github/ejsergeev/Junc1/junction/zoom_2dmic", "status": "active",
  "inputs": [ { "id": "spotify", "match": { "binary": "spotify" }, "enabled": true,
                "volume": 70, "exclusive": true, "status": "active",
                "matched": ["spotify"], "links": [[1, 1], [2, 2]],
                "custom_links": false, "suppressed": [] } ],
  "listens": [ { "id": "hp", "target": "default", "enabled": true, "volume": 80,
                 "status": "active", "links": [[1, 1], [2, 2]],
                 "custom_links": false, "suppressed": [] } ] }
```

`status` is `active`, `waiting` (app or device not there), `disabled` or
`unknown` (the daemon has no graph yet). `links` are the pairs in use: the
config's own list, or the default mapping for what is connected now. Other
commands return small objects (`{"id": …, "enabled": false}`); `revision` is
the config revision after the change.

As built in A11: a Junction is named by id or by name (any case), with "did
you mean" on a miss; link ends are `INPUT:CH` / `LISTEN:CH` or a bare Bus
channel. Single-field changes call the Junction methods; everything else
reads the config, edits it with `junc_core::store` (comments kept) and sends
it with `Apply` and the revision it read, redoing the edit if another change
raced it.

Import (F13), as built in B5: `junc_core::import::scan` finds null sinks
(`support.null-audio-sink` not made by JUNC; origin `pactl load-module` when
`pulse.module.id` is set, else `config`) and loopbacks whose two sides are
streams (`pw-loopback`, or a loopback module set up that way). Loopbacks
that present a virtual device (Omarchy's mic), Bluetooth's own and filter
chains are system parts and not offered. What feeds a setup (links, or the
loopback's capture target) becomes Inputs, the outputs it plays to become
Listen targets, and apps recording from it are listed; one app into one
recording app is suggested as a Cable. juncd answers `ScanImportable`; the
CLI builds the Junctions or Cables and applies them like any edit. The old
setup is never removed.

`junc describe --json` → `data`: `{ pipewire, defaults, junctions[], candidates:
{ apps[], hardware_inputs[], outputs[], monitors[] }, devices[] (with profiles,
channel counts, bluez profile) }` — everything an agent needs in one call.

`junc apply FILE --dry-run --json` → `data.plan` (`junc_ipc::plan::PlanView`,
the same JSON the D-Bus `Plan` and `Apply` return):

```json
{ "summary": ["Create Junction “Zoom Mic” (2 channels)",
              "Add USB Microphone to “Zoom Mic”",
              "Add spotify to “Zoom Mic” (routed when it appears)"],
  "actions": [ { "op": "create_node", "node": "junc.zoom_mic", "media_class": "Audio/Source/Virtual",
                 "channels": 2, "junction": "zoom-mic", "role": "junction" },
               { "op": "create_link", "from": "alsa_input.usb-mic:1", "to": "junc.zoom_mic.in.mic:1",
                 "junction": "zoom-mic", "ref": "in/mic/1->1" }, … ],
  "complete": false, "deferred": [], "junctions": [ … ] }
```

The summary comes from the difference between the config in use and the
proposed one; `actions` are the next reconcile step with node names and
1-based channels. A new Junction's first step only creates nodes
(`complete: false`); its links follow once the nodes exist. Golden plans:
`tests/golden/plan/`.

`junc doctor --json` → `data.checks[]`: `{ "id": "pipewire|wireplumber|pipewire-pulse|jack|clock|rates|asahi|juncd",
"status": "ok|warn|missing|down", "detail": "…", "fix": "sudo pacman -S pipewire-pulse" }`.
`doctor` and `schema`, `config validate FILE` and `config path` work without
juncd.

As built (B9): `Facts::gather` reads the system once (processes,
`pacman -Q`, `pw-metadata -n settings 0`, `pw-dump`, the device tree,
juncd's `Describe` only if it already runs); every check is a pure
function of those facts, with golden outputs in `tests/golden/doctor/`.
`rates` reads each ALSA input and output's `EnumFormat` rates (one value,
a range or a list) and warns when one cannot run at the graph's rate and
its own rate is not in `clock.allowed-rates`; the fix is the
`pw-metadata … clock.allowed-rates "[ … ]"` command (Bluetooth is left
out: its call profiles always resample). `asahi` appears only on Apple
Silicon (`apple,` in `/proc/device-tree/compatible`) when the session sees
the built-in card (ALSA path `hw:Apple…`; an isolated PipeWire does not): the `audio_effect.*`
DSP sink must exist and `speakersafetyd` (a system service) must run.
`jack` says to replace jack2, not to install beside it.

`junc doctor --bug-report` prints one Markdown text for an issue
(`data.report` with `--json`): versions (junc, juncd, PipeWire,
WirePlumber, kernel, arch), a graph line (Junctions, Cables, JUNC links,
rate, quantum), the checks, each Junction with its statuses, the sound
cards, the config file and the last 200 lines of `journalctl --user -u
juncd`. Never audio. The whole text is scrubbed: `$HOME` → `~`, other
`/home/NAME` → `/home/<user>`, the user and host names (whole words) →
`<user>`/`<host>`, hardware addresses (`AA:BB:…`, `AA_BB_…`) → `XX:…`.
It asks juncd for the config only if juncd runs, so a report never starts
it, and it sends nothing. Tests: `report::tests` (scrub, golden
`tests/golden/doctor/bug-report.md`). The GUI's "Copy bug report" (board
41) uses the same text; the D-Bus `Doctor`/`BugReport` methods come with
the GUI if it cannot run `junc`.

`junc watch --json` lines: `{"event":"graph","generation":7}`,
`{"event":"capture","apps":["Spotify"]}`, `{"event":"notice", …}`,
`{"event":"junction","id":"…","enabled":false}`.

As built in B7: the stream starts with the current state (one `capture`
line, one `junction` line per Junction with `name`, `kind`, `enabled`,
`status`), then prints changes only. It follows `GraphChanged`, `Notice`
(`{"event":"notice","kind":…,"body":{…}}`) and changes of `CaptureApps`,
`Revision` and `Junctions`, re-reading `Describe` and printing what differs
(a deleted Junction gives `{"event":"junction","id":…,"removed":true}`). It
also reports juncd going away and coming back (`{"event":"daemon",
"running":false}`). One line per event, flushed at once, so a bar can read
it as it comes. Without `--json` the lines are for people.

### 5.3 Exit codes

`0` ok · `1` error · `2` usage · `3` invalid config · `4` not found ·
`5` conflict (stale revision) · `6` daemon unavailable · `7` PipeWire
unavailable · `8` needs confirmation (`--yes` missing).

### 5.4 Error messages

Every error says what is wrong, where (file, line, column when it comes from a
file), and how to fix it, with "did you mean" for unknown keys and names. The
codes are stable and documented in the skill's reference guide.

---

## 6. Configuration, presets and templates (schema v1)

### 6.1 Files

| Path | Owner | Content |
| --- | --- | --- |
| `~/.config/junc/junctions.toml` | user + daemon | Junctions and Cables (desired state). |
| `~/.config/junc/settings.toml` | GUI + daemon | Start at login, language, theme, sample rate default. |
| `~/.config/junc/presets/*.toml` | user | Exported presets (F22). |
| `/usr/share/junc/templates/*.toml` | package | Built-in templates (F24). |
| `~/.local/state/junc/state.toml` | daemon | Saved previous defaults, notices, last seen versions. Not user intent. |
| `~/.local/state/junc/last-good.toml` | daemon | Last valid `junctions.toml`. |

### 6.2 `junctions.toml`

```toml
# JUNC configuration. Edit by hand or with `junc`; the daemon picks up changes.
schema = 1

[[junction]]
id = "virtual-mic-for-zoom"        # stable; node.name = "junc.virtual_mic_for_zoom"
name = "Virtual Mic for Zoom"
kind = "junction"                  # "junction" | "cable"
enabled = true
volume = 100                       # percent, 0–150, cubic like PulseAudio
default = "input"                  # "none" | "input" | "output"
rate = "follow"                    # "follow" or 44100…192000
bus = { channels = 4 }             # 1–64; layout FL FR for 2, AUX0… otherwise

[[junction.input]]
id = "spotify"
match = { app = "Spotify", binary = "spotify" }
enabled = true
volume = 70
exclusive = true                   # F11
links = [[1, 1], [2, 2]]           # [input channel, bus channel]; omitted = default mapping

[[junction.input]]
id = "soundux"
match = { binary = "soundux" }
links = [[1, 3], [2, 4]]

[[junction.input]]
id = "usb-mic"
match = { device = "alsa_input.usb-Generic_USB_Microphone-00.mono-fallback" }
name = "USB Microphone"            # optional; shown while the device is unplugged
links = [[1, 1], [1, 2]]           # mono mic to both sides
suppressed = [[1, 2]]              # removed by another tool, left as is (F17)

[[junction.listen]]
id = "builtin"
target = "default"                 # "default" or an output node.name
volume = 80
links = [[1, 1], [2, 2]]           # [bus channel, output channel]

[[junction]]
id = "firefox-to-obs"
name = "Firefox → OBS"
kind = "cable"
[[junction.input]]
id = "firefox"
match = { binary = "firefox" }
[junction.cable]
to = { binary = "obs" }            # the app that hears the Cable as its input
```

Implemented in `junc-core` (`model.rs`, `config.rs`, `migrate.rs`); the
model types are the single source of truth and `schema/junc-config-v1.json`
is generated from them (`cargo xtask schema`, freshness tested).

Rules (enforced by the validator, all with file, line and column):

- `schema` is a whole number; a missing key means v0 and is migrated; a
  newer schema is refused. Unknown keys are errors with "did you mean".
- ids are `[a-z0-9-]+` (no leading/trailing `-`, ≤ 64), unique per scope
  (Junctions; Inputs and Listen outputs within a Junction); names are free
  text in any script but not empty.
- `links` pairs are 1-based `[from, to]` channels within range (input side
  1–64 since device channels are only known live; Bus side ≤ `bus.channels`);
  a missing `links` uses the default mapping (F4). `suppressed` lists pairs
  another tool removed and the user left as is (F17).
- Bus channels 1–64 (F3); volumes 0–150 %; fixed rates 44100–192000 Hz.
  More than 32 Inputs on one Junction is a warning.
- A `match` uses exactly one kind of key: app keys (`app`, `binary`,
  `role`, `node`), `device`, `monitor`, `junction`, or `slot` (presets only);
  `profile` may accompany a device match (F15).
- At most one Junction per role has `default = "input"` / `"output"`.
- Nesting must name an existing Junction and be acyclic (self-nesting
  included).
- `kind = "cable"` needs `[junction.cable] to = { … }` and exactly one app
  Input; a `cable` table on a Junction is an error.

Diagnostic codes (stable; also listed in the skill reference):

| Code | Meaning |
| --- | --- |
| `E_CONFIG_SYNTAX` | Not valid TOML |
| `E_CONFIG_SCHEMA_NEWER` / `E_CONFIG_SCHEMA_INVALID` | Unknown or too new `schema` |
| `E_CONFIG_UNKNOWN_KEY` / `E_CONFIG_MISSING_KEY` / `E_CONFIG_INVALID_VALUE` | Shape errors |
| `E_CONFIG_ID_FORMAT` / `E_CONFIG_ID_DUPLICATE` | Bad or repeated id |
| `E_CONFIG_RANGE` | Number out of range |
| `E_CONFIG_LINK_CHANNEL` | Link to a channel that does not exist |
| `E_CONFIG_DEFAULT_DUPLICATE` | Two defaults for one role |
| `E_CONFIG_MATCH_KIND` | Empty or mixed match |
| `E_CONFIG_JUNCTION_UNKNOWN` / `E_CONFIG_CYCLE` | Bad nesting |
| `E_CONFIG_CABLE` | Cable shaped wrong |
| `E_CONFIG_SLOT_OUTSIDE_PRESET` | Template slot in a config |
| `E_PRESET_SLOT` | Preset slot undeclared, declared twice, unused, or of a kind that does not fit |
| `E_PRESET_FILL` | Template slot values missing or naming slots it lacks |
| `W_CONFIG_MANY_INPUTS` | Warning: > 32 Inputs |

### 6.3 Presets and templates (F22, F24)

One format for both. A preset is a single Junction table plus a `[preset]`
header; a template is a preset whose matches may reference **slots**.

```toml
schema = 1

[preset]
name = "Mic + App"
description = "Your mic and one app mixed into one input."
example = "e.g. USB Microphone + Spotify"
icon = "mic"
builtin = true                     # only in /usr/share/junc/templates

[[preset.slot]]
id = "mic"
kind = "hardware-input"            # hardware-input | app | output | monitor
prompt = "Pick a mic"

[[preset.slot]]
id = "app"
kind = "app"
prompt = "Pick an app"

[junction]
name = "{app} + {mic}"
kind = "junction"
bus = { channels = 2 }

[[junction.input]]
id = "mic"
match = { slot = "mic" }
[[junction.input]]
id = "app"
match = { slot = "app" }
```

- Choosing a template asks for each slot (design `templateSlots`), then
  creates an ordinary Junction or Cable; nothing in the result refers back to
  the template.
- Export (`exportPreset`) writes the same format; device-specific matches are
  flagged `device-specific` in the dialog and match by name on import, showing
  offline until the device is present.
- Built-in set (F24): `cable`, `mic-plus-app`, `combine-inputs`,
  `separate-tracks`, `stream-mix`, `start-empty`. They double as agent recipes.

As built (B8), in `junc-core::preset`:

- Slots are used as `match = { slot = "…" }` on Inputs (hardware-input,
  app or monitor) and on a Cable's `to` and Input (app only), as
  `target = "{…}"` on Listen outputs (output slots), and as `{…}` in the
  Junction's name. Filling turns a slot into `device`, `binary` or
  `monitor`, or the Listen target;
  the name gets the value's label (device description or app name, from
  the graph) or the value itself.
- Loading reuses the config loader (syntax, schema migration, every config
  rule, with locations mapped into the preset file) and adds the slot
  rules: declared once, used, of a kind that fits (`E_PRESET_SLOT`).
  Shape errors inside `[junction]` point at its header. `id` may be left
  out: it is made from the name, and an import makes it unique anyway.
- Without slots, braces in a name are plain text, so any Junction exports.
- Exports keep the whole Junction (volumes, links, `suppressed`, default
  role) and write `match`, `bus` and `to` inline as in the examples above;
  the CLI adds a comment naming the device-specific Inputs and Listen
  outputs. An import whose default role another Junction holds is added
  without it, with a note.
- The built-in templates live in `templates/`, are compiled into
  `junc-core` (so `junc template list` works without the daemon and always
  matches the binary) and are installed to `/usr/share/junc/templates` by
  the package (B25). `builtin = true` is informational.
- CLI: `junc template list|show`, `junc create junction NAME --template T
  --slot ID=VALUE`, `junc preset export NAME [-o FILE]`, `junc preset
  import FILE [--slot ID=VALUE] [--name NAME]`. All edits go through
  `GetConfig` + `Apply`; the D-Bus API is unchanged (the GUI's
  `ImportPreset`/`ExportPreset` come with the GUI if it needs them).
- Tests: `junc-core/tests/preset.rs` (every built-in template loads, fills
  into a valid Junction and survives a save; export → import round trips
  every fixture Junction; slot mistakes reported at their line);
  `junc-cli/tests/cli.rs` `templates_and_presets_make_junctions` (mock
  daemon). `cargo xtask skill-check` loads `[preset]` examples as presets.

### 6.4 Schema versioning and migrations

- `schema = N` in every file. The JSON Schema is generated from the Rust types
  (`schemars`) and published by `junc schema`; the checked-in
  `schema/junc-config-v1.json` must match (CI).
- Loading an older schema runs pure migration functions `vN → vN+1` (unit
  tested with fixture pairs), backs up the original as
  `junctions.toml.v<N>.bak`, and writes the migrated file. A newer schema than
  the daemon knows is refused with a clear error and the last good config is
  kept.

---

## 7. GUI architecture (Slint)

### 7.1 Process and data flow

```
junc-gui
├─ IPC client (junc-ipc proxies) ── D-Bus ──▶ juncd
├─ AppState (Rust) ── models ──▶ Slint UI (ui/app.slint)
├─ ThemeService  (Omarchy colors.toml + shell.toml → Tokens global)
├─ L10nService   (Omarchy language file → select_bundled_translation, ICU4X numbers)
├─ MeterService  (junc-pw meter stream, only while visible)
├─ VisibilityService (window shown/minimized/occluded)
└─ Tray (ksni StatusNotifierItem + DBusMenu)
```

- The GUI never edits config files; every action is a D-Bus call. On
  `GraphChanged`/`PropertiesChanged` it updates Slint models (`VecModel`s per
  column), so external changes (CLI, agent, hand edit) appear live.
- Slint renderer: femtovg (GPU) on Omarchy; software renderer fallback. The
  validation numbers (24 MB PSS, 3.3 % CPU with meters, software renderer) are
  the upper bound to beat. Step 0 on the M1 (femtovg): 88 ms cold start,
  62 MB PSS, but 6.4 % CPU with 7 Hz meters because every tick repaints the
  whole window (see `docs/hardware-matrix.md`); §7.6 must limit repaints to
  the meter regions.
- The window has no title bar (tiled by Hyprland); sizes follow the design:
  sidebar 300 px (240 px when the tile is < 1100 px), main pane three columns
  ≥ 780 px, Listen stacked below 780 px (design `Narrow`).

### 7.2 Tokens and component library

Layer 1 — `ui/tokens.slint`, a `global Tokens` with every semantic token from
the Spec sheet. Values are set from Rust at start and on theme change;
components never contain hex values.

| Token group | Tokens (Spec §1) | Omarchy source |
| --- | --- | --- |
| Surfaces | `surface-desktop`, `surface-window`, `surface-sidebar`, `surface-card`, `surface-card-header` | `darker_background`, `background`, `dark_background`, `lighter_background`, mix(card, fg, 4 %) |
| Text | `text-default`, `text-strong`, `text-subtle`, `text-status-*` | `foreground`, `bright_foreground`, mix(fg, bg, 20 %), status inks darkened to 4.5:1 |
| Marks | `mark-dim`, `line-divider`, `accent`, `selection`, `meter-safe/hot/clip` | `dark_foreground`, `muted`, `accent`, `selection`, `green`/`yellow`/`red` |
| Control states | normal / hover-cursor / focus / selected / pressed / text-selection fills and borders | `shell.toml [controls]` alphas (4/8/8/18/22/35 %) |
| Spacing, type, shape, motion | `space-*`, `size-*`, `type-*` (base 12), radius 0, border 1, popover border 2, durations | `shell.toml [spacing]`, `[font]`; reduced motion → all durations 0 |

Derived values (mixes, alphas, contrast-corrected status inks) are computed in
one Rust module (`junc-gui/src/theme.rs`), the same formulas as the Spec sheet.

Layer 2 — components in `ui/components/`, one file each, mapped 1:1 to the
Spec sheet:

| Spec component | Slint component | Notes (a11y fixes from the validation report in **bold**) |
| --- | --- | --- |
| Card | `JCard` (+ `JCardHeader`, `JCardFooter`) | Focus/off/offline/not-running/exclusive states. **Own focus ring via `has-focus`.** |
| Port | `JPort` | Button per channel; free/linked/hover/offline/warn/valid/snap states. |
| Link | `JLink` (`Path`, cubic, halo on hover) | Hit area 10 px; dashed offline; yellow dashed overridden. |
| Meter | `JMeter` | Width-only updates; **`accessible-role: none`**, level announced on the channel label. |
| Toggle | `JSwitch` | **Wrapper with `accessible-label` + description "on/off"** (AT-SPI has no switch role). |
| Slider | `JSlider` | ←→ 1 %, Shift 10 %, Home/End, **PageUp/PageDown ±10**, **visible focus ring**. |
| Button | `JButton`, `JIconButton` | Five control states; tooltip with shortcut. |
| Picker | `JPicker` (`PopupWindow`) | **Root exposed as dialog with label** (best available role; upstream issue filed), **search field labelled**, ↑↓ Enter Esc, focus trapped, focus returns to opener. |
| Empty state | `JEmptyState` | First Tab stop in its column. |
| Checklist row | `JCheckRow` | ok/waiting/missing/mismatch; code field + Copy. |
| Confirm dialog | `JConfirm` | Focus on Cancel; destructive needs Ctrl+Enter; Esc cancels. |
| Inline text field | `JInlineField` | **`accessible-label`**; Enter save, Esc restore, blur save. |
| Language row | `JLanguageRow` | Follow Omarchy switch + disabled select. |
| Pro Audio badge, Badge | `JBadge` | Never interactive. |
| Channel group | `JChannelGroup` | Tree rows, groups of 8, mini meters. |
| Toast | `JToast` | 8 s, stays while hovered/focused, Ctrl+Z Undo. |
| Banner | `JBanner` | neutral/warning/error; `role=status`. |
| Tooltip | `JTooltip` | 500 ms hover, instant on focus. |
| New menu, context menu | `JMenu` | ↑↓ Enter Esc. |
| Radio group | `JRadioGroup` | Add Bus channels popover. |
| Keyboard chip | `JKbd` | Footer hint bar. |
| Layout | `JRow`, `JColumnLane` | **Direction-aware from day one**: take `rtl` from a global and order children/keys accordingly; v1 always passes `false`. |

As built (B13): `ui/components/` holds one file per component above
(`JCard` includes its header and footer; `JIconButton` is a square
`JButton`; tray menu and bar module come with the tray and the bar plugin),
plus `state.slint` (the five control states), `direction.slint`
(`Direction.rtl`, `JRow`, `JColumnLane`; arrow keys follow it), `dashed.slint`
(dashed borders, drawn piece by piece: Slint has no dash pattern; dashed
links likewise use short pieces of the curve). Components with an inner
`FocusScope` forward focus. `JButton.needs-ctrl` makes destructive
confirmations run only on Ctrl+Enter or a click. `Tokens` gained the
motion durations (0 with `reduced-motion`) and the tooltip delay.
`junc-gui --scenario gallery [--theme light|dark]` shows every component in
its states.

Layer 3 — platform shell (Spec §3): no window chrome, Nerd Font glyphs through
the fontconfig `monospace` alias, `[popups]` styling, StatusNotifierItem tray,
the Omarchy bar plugin (separate QML, §10.4).

Screens in `ui/screens/`: `Main` (sidebar + header + routing canvas + footer
hints), `CableView`, `Onboarding` (check, templates, slots), `Checks`
(doctor/service down), `Settings`, `EmptyNone`; overlays: pickers, menus,
dialogs (delete, import, export, bug report), banner, toast.

As built (B16): `ui/app.slint` is the window (sidebar rows: name and switch,
what feeds the Junction, a 4 px meter, master volume) and
`ui/screens/main.slint` the Main screen (header, column headings, the
canvas in a `Flickable`, bottom bar). One global, `View` (`ui/view.slint`),
carries everything Rust gives the window and every callback back. In Rust:
`main_view.rs` turns `Describe` into a language-free model (cards, Bus
groups, links with their `owner` and config pair; titles from the Input's
new optional `name`, else the matched or matching node, else the match;
a waiting Input keeps dashed links on the default stereo mapping; an Input
that is off draws none); `layout.rs` places it (below); `ui.rs` makes the
Slint models; `app.rs` is the controller (selection, layout, hovered link,
callbacks → `actions::Command`); `edits.rs` holds the pure config edits
(on/off, volume, unlink as "pairs in use less these") that `actions::Dbus`
applies with `GetConfig` → `Apply(revision)`, retried on a conflict, on its
own thread; Junction on/off, master volume and delete use their own
methods. Volumes are sent once a slider rests 250 ms. Meters are numbered
slots (a node's channel, or its loudest channel for a sidebar row) in
level models the window owns, so an unfed meter reads silence. Show
Listen hides the Listen column (the docs left it open; the design shows
it pressed everywhere). Delete asks with `JConfirm`; Rename, Default, the
add buttons, New and Settings are drawn and come alive with their boards
(B17–B22). Ctrl+1…9, the arrow keys between cards and Del on a hovered
link came with B18 (below); Tab follows the tree: sidebar, header, cards
column by column, ports, bottom bar.

As built (B17): `ui/screens/onboarding.slint` (boards 8, 9, 28) and
`ui/screens/empty.slint` (board 10) take the window in place of the Main
screen; the Main screen gained the empty columns of board 11 and the Add
Input picker of board 7. Onboarding (`onboarding.rs`) shows on a first
launch while juncd has no Junctions; it is remembered as done in the
GUI's own `$XDG_STATE_HOME/junc/gui.toml` (never the config), and ends
by itself when Junctions appear meanwhile (CLI, agent). Step 1 runs
`junc doctor --json` (the `junc` next to `junc-gui`, else on `PATH`) on a
thread and shows four of its checks (`checks.rs`: PipeWire, WirePlumber,
pipewire-pulse, the JACK layer); a missing or stopped one holds Continue,
a warning does not; R re-checks, Copy puts the fix on the clipboard with
`wl-copy`. Step 2 lists the six built-in templates, worded in the GUI's
catalogs (the template files stay the CLI's and agents' text); "Start
empty" is a New Junction (`Daemon.CreateJunction` with the localized
name), selected when juncd shows it; templates with slots wait for the
slots step (board 49, B22). Board 10's New Junction and Ctrl+Shift+J do
the same; New Cable and Import come with B19, the Listen empty state's
button with B18. The picker (`picker.rs`) groups `Describe`'s candidates
as running apps (one row per binary), hardware inputs, system audio
(sink monitors) and the other Junctions, leaving out those that would
make a loop; rows the Junction takes already say "added" and cannot be
picked again; the search filters by name; Enter, a click, Ctrl+N and the
"+" on Inputs work as the board says. Picking adds an Input through
`GetConfig` → `Apply` (`Edit::AddInput`, id from the name as `junc input
add` makes it). Words Rust needs in the UI language (group captions,
"Monitor of …", the New Junction name) come from the `Words` global's
functions, so Rust stays without catalogs. Not built: the picker's
"playing" detail, since the graph keeps no node state. Components gained
what the boards use: `JButton.show-key` (the shortcut chip inside the
button) and `flat`, `JPicker` icons, search icon and key hints, a
`down` state in `JCheckRow`, `JLogo`.

As built (B18, Junction off): a Junction that is off (board 18) keeps its
cards, dimmed to 45 % with their own switches as they were, and draws no
links, so every port is free; its meters stay dark. A neutral banner
under the header ("Junction is off — apps see silence") shifts the
columns down and offers Turn on with its key, Space: Space that nothing
focused took turns the selected Junction back on (`Junction.SetEnabled`),
as does its sidebar switch. The sidebar row says "Off · apps see
silence" in place of its Inputs. `JBanner` gained `action-key` and
`action-primary`.

As built (B18, Add Listen output): board 17's picker is the Add Input
picker in a second mode, over the Listen column's right edge; Ctrl+M,
the "+" on Listen and the empty Listen column's button open it.
`picker::outputs` lists the default output first ("Default output
(follows system)", target `default`), then the output devices from
`Describe`'s sink candidates by name, a screen glyph for HDMI and
DisplayPort. A row says "added" when a Listen output goes there already,
by name or, for one that follows the default, to the device it follows.
Picking adds a Listen output through `GetConfig` → `Apply`
(`Edit::AddListen`: id from the device's name, default links and
volume). `JPicker` gained a `note` under its rows.

As built (B18, Add Bus channels): Ctrl+Shift+N and the "+" on Bus open
board 16's popover, right-aligned to the Bus column: a stereo pair or one
mono channel after the last (`JRadioGroup` gained a detail per option,
"Ch 5–6", and marks the chosen row while focused), how many channels apps
will see, Cancel (Esc) and Add (Enter). Add raises `bus.channels`
through `GetConfig` → `Apply` (`Edit::AddBus`), never past 64. The Bus
keeps no grouping of its own: its cards pair channels from the first, so
a mono channel followed by a pair reads as pairs from the start.

As built (B18, Input Options): the Options button of an Input card opens
its panel (boards 15, 30), under the card, or beside it over the Bus on
a narrow tile; one at a time, its place from `layout.rs` like the card's.
It has Exclusive for apps (`Edit::Exclusive`), "Feeds Bus" with a
`JSelect` per channel showing the Bus channel it feeds (the first when
several; `main_view::feeds`) and setting that channel to feed only the
one chosen, or none (`Edit::Feed`, the other channels' pairs kept), and
Remove input (Del, `Edit::RemoveInput`, no confirmation: the Input comes
back from the picker). Esc closes it; the window also takes Esc and Del
while it is open, in case an update moved the focus. Its models are
updated in place (`app::update`), so the selects keep their elements and
the focus. `JSelect` (new in the library and the gallery) opens its list
under the field, pushing what follows down; its owner sets the choice.
`JCard` lets its owner say whether the Options panel is open, through
`JButton.controlled` (a toggle whose state its owner sets; it stays a
toggle button to assistive technology).

As built (B18, Cable selected): a Cable has its own screen
(`ui/screens/cable.slint`, board 12) in place of the Main screen. From
(the app it takes from) and To (the app that hears it as its input), the
Main model's Input card and Bus channels with their meters, linked
straight across, dashed in mark.dim while either app is away; under them
the Cable's volume (`Junction.SetVolume`), Listen (a switch and the
outputs of `picker::outputs` in a `JSelect`: on makes a Listen output on
the output shown, `Edit::ListenTarget` moves it) and Exclusive. "Change
app…" opens the picker of playing apps (From, `Edit::CableFrom`) or
recording apps (To, `Edit::CableTo`), the one it has marked added
(`picker::cable_apps`). `main_view::cable_to` names the To app from the
recording candidates. Not built: the sample rate in the header (Describe
has none).

As built (B18, links): a link is made from any port (boards 13, 14).
Pressing a port and dragging draws the link to the pointer; the ports it
may end at (`linking::targets`: across the Bus, Input ↔ Bus in and Bus
out ↔ Listen, none it is linked to, no card that is off) get an accent
border, the one within 16 px of the pointer snaps (`linking::snap`), a tip
names the link ("Soundux 1 (L) → Ch 1", Esc) and releasing there links;
anywhere else nothing happens. Enter on a focused port starts one with
the keys: the first target is chosen, ↑ ↓ step through them top to
bottom, the line is dashed, Enter connects, Esc cancels. The bottom bar
shows only the keys that apply meanwhile. A link adds its pair through
`GetConfig` → `Apply` (`Edit::Link`; the pairs in use become the config's
own list, and the pair is no longer one another tool removed). The
canvas updates its card, port and link models in place (`app::update`),
so a port keeps the pointer while the canvas changes under a drag; the
canvas no longer scrolls by dragging (the wheel does), and a port's
FocusScope leaves the press to its TouchArea (`focus-on-click: false`,
Slint's default would take it).

As built (B18, keys from §7.4): Ctrl+1…9 select the Junctions in the
sidebar's order. On a focused card ↑ ↓ go to the card above or below in
its column and ← → to the card of the next column whose middle is
nearest, skipping an empty column (`ui::arrow_target`; the card is asked
to take the focus through `JCard.wants-focus`). Del with the pointer on a
link unlinks it, as a click does. ← → on channel groups wait for them
(the Main screen draws none yet).

As built (B19, Rename): F2 or the header's pencil puts the name in place
(board 26, `JInlineField`, the title's type, its text selected) on the
Main and the Cable screen. Enter or leaving the field saves through
`Junction.Rename`; the id, and so the node name, stays. Esc, a blank name
or the same one changes nothing.

As built (B19, Delete confirmation): Ctrl+Delete or Delete in the bottom
bar asks first (board 22, `JConfirm` with a lead, a "Removed with it"
list and a note). The list counts the Inputs and links, names the Bus
groups and counts the Listen outputs (`main_view::removal`); the note
names the Inputs that keep playing. Focus starts on Cancel; Ctrl+Enter
deletes from anywhere in the dialog (a plain button lets Ctrl+Enter
pass). A Cable's dialog says which app goes back to its own input. Not
built: naming the apps that record from the Junction, as the board does
("Zoom uses this Junction…"); `Describe` does not say who records from
it, so the lead says it of all such apps.

As built (B19, Context menu): a right-click on a sidebar row, or the
Menu key (Shift+F10) on the selected one, opens its menu beside the row
(board 21, `JMenu` with dividers and a destructive item): Rename (F2),
Duplicate (Ctrl+D), Turn off/on (Space), Export preset… and Import
preset… (greyed until B22), Delete… (Ctrl+Delete, the board 22 dialog).
Duplicate writes a copy after the original through `GetConfig` →
`Apply` (`Edit::Duplicate`): named "<name> copy", its id from that name,
and never a system default, since only one Junction can be.

As built (B19, App closed): an app Input whose app quit (board 23) keeps
its card with "Not running" (a subtle marker), "Waiting for <app>…" in
place of Options and volume, and dashed links; Exclusive is not shown
while it waits. The Inputs heading and the sidebar row add "· N waiting".
A closed app is named from its binary with a capital ("spotify" →
"Spotify") when nothing else names it.

### 7.3 Routing canvas

Columns Inputs → Bus → Listen with a 120 px (min 72) link gutter. Card and
port positions are computed in Rust (`layout.rs`) from the model and window
width, then passed to Slint as flat models (cards, ports, links as SVG path
commands). Rationale: link curves need port coordinates from three columns,
which is simpler and faster to compute once in Rust than with Slint bindings,
and the same function mirrors geometry for RTL later (`x′ = width − x`).
Meters and hover never change layout (Spec "never").

As built (B16): `layout.rs` uses the Spec sizes (card header 33, rows 28 +
4, padding 10, footer 28, 16 between cards, gutters 120 down to 72, cards
at least 200); below 780 px Listen stacks under the Bus without ports or
links and its rows read "L ← Ch 1" (design Narrow); the window hides the
footer hints in a tile under 1100 px. The link under the pointer is the
nearest within 5 px of the curve (Slint paths take no pointer events);
its tooltip names it ("USB Microphone 1 (L) → Ch 1").

Interaction: drag from a port draws a live link and snaps to valid targets
(design `linkDrag`); keyboard: Enter on a port, ↑↓ moves a target ring over
valid Bus ports, Enter connects, Esc cancels (`linkKeys`); click a link or Del
to unlink (`wireHover`). Valid targets and cycle checks come from
`junc-core::planner` so GUI and daemon agree.

### 7.4 Keyboard map (Spec "Keyboard")

| Keys | Action |
| --- | --- |
| Tab / Shift+Tab | Junctions → Inputs → Bus → Listen → bottom bar |
| ↑ ↓ / ← → | within a column / between columns; → ← expand/collapse channel groups |
| Space | toggle focused Junction, Input or Listen output |
| Enter on port | start link; ↑↓ target; Enter connect; Esc cancel |
| Del | unlink, or remove focused Input |
| Ctrl+N / Ctrl+M / Ctrl+Shift+N | add Input / Listen output / Bus channels |
| Ctrl+Shift+J / Ctrl+Shift+K | new Junction / new Cable |
| Ctrl+, · F2 · Ctrl+1…9 | Settings · rename · switch Junction |
| Ctrl+D · Ctrl+E · Ctrl+I · Ctrl+Delete | duplicate · export preset · import preset · delete |
| Ctrl+Shift+E | expand all channel groups |
| Ctrl+Z | Undo on toast |
| Shift+Enter (picker) | Use all channels / Add anyway |

### 7.5 How each design state is reached

Every state has a scenario fixture (`tests/fixtures/scenarios/<scenario>.toml`:
config + mocked graph + UI action script). `junc-gui --scenario <name>`
(compiled only with the `scenarios` feature) runs the GUI against a mock IPC
server fed by the fixture; screenshot and AT-SPI tests use it (§11).

| # | Design board | Scenario | Reached in the real app by |
| --- | --- | --- | --- |
| 1 | Main · Tokyo Night | `default` | Select a Junction with 3 Inputs, 2 Bus pairs, 2 Listen outputs |
| 2 | Main · Flexoki Light | `default` + theme flexoki | Omarchy theme Flexoki Light (or fallback light theme) |
| 3 | Narrow tile 900×600 | `default` + narrow | Tile narrower than 1100 px; main < 780 px stacks Listen |
| 4 | Soundux toggled off | `sourceOff` | Space on an Input card |
| 5 | USB Microphone offline | `offline` | Unplug a hardware Input (link → Waiting) |
| 6 | Link hover | `wireHover` | Pointer over a link |
| 7 | Add Input picker | `picker` | "+" on Inputs or Ctrl+N |
| 8 | Onboarding · system check | `onboardCheck` | First launch with a missing dependency |
| 9 | Onboarding · templates | `onboardTemplates` | Checks pass → Continue |
| 10 | Empty · no Junctions | `emptyNone` | Onboarding done, config empty |
| 11 | Empty · new Junction | `emptyJunction` | New → Junction / template "Start empty" |
| 12 | Cable selected | `cable` | Select a Cable |
| 13 | Link · dragging | `linkDrag` | Drag from an Input port |
| 14 | Link · keyboard | `linkKeys` | Enter on a port |
| 15 | Input Options expanded | `inputOptions` | Options disclosure on a card |
| 16 | Add Bus channels | `addBus` | Ctrl+Shift+N |
| 17 | Add Listen output | `addListen` | Ctrl+M |
| 18 | Junction off | `junctionOff` | Space on a Junction row |
| 19 | Tray menu | `tray` | SNI menu; tested as a DBusMenu tree (drawn by the Omarchy shell) |
| 20 | Audio service down | `serviceDown` | Daemon `State = waiting_for_pipewire` |
| 21 | Context menu · Duplicate | `contextMenu` | Right-click / Menu key on a Junction row |
| 22 | Delete confirmation | `deleteConfirm` | Ctrl+Delete or menu Delete… |
| 23 | App closed · waiting | `appClosed` | Captured app quits |
| 24 | Import existing setups | `import` | Settings › Existing setups › Import… (F13) |
| 25 | Settings | `settings` | Ctrl+, |
| 26 | Rename | `rename` | F2 or pencil |
| 27 | New Cable | `newCable` | Ctrl+Shift+K |
| 28 | Onboarding · Flexoki | `onboardCheck` + flexoki | as 8, light theme |
| 29 | Empty Junction · Flexoki | `emptyJunction` + flexoki | as 11, light theme |
| 30 | Input Options · Narrow | `inputOptions` + narrow | as 15, narrow tile |
| 31 | Settings · Language picker | `langPicker` | Turn off Follow Omarchy, open Language |
| 32 | Arabic (RTL reference) | `default` + ar | **Not in v1**: v1 asserts the UI shows English for `ar` (PRD) |
| 33 | Main · German | `default` + de | Omarchy language `de` |
| 34 | Main · Japanese | `default` + ja | Omarchy language `ja` |
| 35 | Pro Audio prompt | `proAudio` | Picker cursor on a device with more channels in Pro Audio |
| 36 | Multichannel Input | `multichannel` | After "Use all channels" |
| 37 | Use as default | `useDefault` | Default menu in the header |
| 38 | Default restored | `defaultRestored` | Delete/disable the default Junction → toast |
| 39 | Link overridden | `linkOverridden` | Another tool removed a JUNC link → banner |
| 40 | Bluetooth warning | `btWarning` | Picker cursor on an A2DP headset mic |
| 41 | Bug report | `bugReport` | Settings › Diagnostics › Copy bug report… |
| 42 | Doctor issue | `doctorIssue` | Settings › Run checks finds an issue |
| 43 | Updated | `updated` | Daemon version changed since last GUI start |
| 44 | Bar module | `barModule` | Omarchy bar plugin popup (QML; tested via `omarchy agent look plugin`) |
| 45 | Omarchy menu | `omarchyMenu` | Omarchy menu › Setup › "Audio routing · JUNC" (menu JSON test) |
| 46 | Export preset | `exportPreset` | Ctrl+E |
| 47 | Config error | `configError` | Invalid hand edit → error banner |
| 48 | New menu | `newMenu` | "+ New" › From template… |
| 49 | Template slots | `templateSlots` | Choose a template with slots |
| 50 | App icon | — | Assets exported to `packaging/icons` (SVG + hicolor PNGs, symbolic, Omarchy template) |

The Spec sheet board is the component reference, not a state.

### 7.6 Meter pipeline (pauses when hidden)

PipeWire nodes do not publish peak levels as properties, so meters need
samples. To keep the daemon at 0 wakeups and the audio path untouched:

- Meters live **in the GUI process only**, in `junc-pw`'s `meter` feature: one
  `pw_filter` named `junc-gui.meter` with one input port per visible meter
  channel, linked (non-lingering, tagged `junc.role = meter`) from the monitor
  ports of the input gain nodes, the Junction and the listen gain nodes.
  One filter = one RT callback per graph cycle regardless of how many meters.
- The callback computes peak per port into atomics. A Slint `Timer` at ~7 Hz
  (Spec: "~7 times a second") reads the atomics and updates `JMeter` widths.
- **Visible only**: `VisibilityService` reports hidden when the window is
  closed, minimized, or not presented (Wayland frame-callback starvation:
  a requested redraw not presented within 500 ms; on Hyprland also the
  workspace of the window via Hyprland IPC events). Hidden → stop the 7 Hz
  timer, destroy the meter links and deactivate the filter, so the GUI takes
  0 % CPU and PipeWire runs no extra node (PRD "CPU when hidden 0 %").
  Shown → re-create links. Verified by spike S7 and a CI gate (meter callback
  count = 0 while hidden).
- Reduced motion does not stop meters (they are information), but the peak
  tick does not animate.

As built (B15): pipewire-rs 0.10 has no `pw_filter` binding, so
`junc_pw::meter` uses one capture **stream** `junc-gui.meter` instead, with
one AUX input port per metered channel (≤ 64): still one RT callback per
cycle, no unsafe code. Its thread links the output ports of each watched
node (`Meter::watch(node names)`; a sink's outputs are its monitor ports)
with passive, non-lingering links tagged `junc.role = meter`; the stream is
`node.passive`, not autoconnected, and WirePlumber links nothing to it. The
callback keeps per-channel peaks in atomics (`fetch_max` on the `f32`
bits); `Meter::read()` takes them per node. Watching nothing disconnects
from PipeWire entirely: stream, links and callback are gone and the thread
blocks on a condition variable (live test: gone ~30 ms after the pause, 0
callbacks and 0 thread CPU while paused). In the GUI (`visibility.rs`,
`meter_service.rs`, `meters.rs`) a `Gate` combines three signals, any of
which hides: closed; Hyprland (events on `.socket2.sock`, then `j/clients`
and `j/monitors`: the window's workspace must be a monitor's active or
shown special workspace and no other window fullscreen on it — Hyprland's
per-client `visible` stays true on other workspaces); starvation (a frame
asked for and not presented within 500 ms, from Slint's rendering notifier,
femtovg only). The gate pauses the meter itself from whichever thread
notices. The ~7 Hz `Timer` lives in `app.slint` and runs only while the
gate says running; bars jump up and fall at 24 dB/s, the peak tick holds
1.5 s, values round to 0.5 dB so that changes a bar cannot show cause no
repaint. On a pause the bars drop to silence and one frame stays asked
for, so a false starvation resumes at once. Spike S7 (§12.1): on Hyprland
the IPC is what works; Hyprland draws hidden windows at
`misc:render_unfocused_fps` (15), so starvation never fires there and is
the fallback for other compositors only. So far the sidebar shows one
meter per Junction (its source); the column meters come with B16.

### 7.7 Live theme reload

1. Read palette roles from `colors.toml` and `[controls]`, `[spacing]`,
   `[font]`, `[popups]`, `[bar]` from `shell.toml` in
   `~/.local/state/omarchy/current/theme/`.
2. Derive mixes/alphas/inks in `theme.rs`.
3. Watch `~/.local/state/omarchy/current` with inotify (the theme directory is
   swapped by `omarchy-theme-set`), debounce to the end of the move.
4. Assign all `Tokens` properties in one pass; components re-render in place.

Settings › "Follow Omarchy theme" off, or not on Omarchy: built-in Tokyo Night
(dark) and Flexoki Light (light), chosen by the portal color scheme
(`org.freedesktop.appearance color-scheme`).

As built (B12), in `junc-gui` (a library plus a thin `main.rs`, so the logic
is tested without a display):

- `theme.rs` reads `colors.toml` roles and `shell.toml` (`[controls]`
  alphas, `[spacing] scale`, `[font] base-size`) and derives every Spec
  token with the Spec board's formulas (`mix` per channel, foreground
  alphas, status inks moved toward the foreground in 10 % steps until
  4.5:1 on a card). Missing roles take the nearest one. The font family is
  what `fc-match monospace` names (Slint does not resolve fontconfig
  aliases; JetBrains Mono on Omarchy).
- `theme_service.rs` watches `…/omarchy/current` for the rename
  `omarchy-theme-set` makes (`rm -rf theme; mv next-theme theme`) and the
  theme directory for edits in place (re-watched after each swap), waits
  150 ms of quiet, then loads once. A theme without readable colours falls
  back to the built-in theme: Omarchy 4.5.0 lists Osaka Jade but ships
  only its wallpapers. `$OMARCHY_THEME_DIR` overrides the location.
- `ui/tokens.slint` (`Tokens` global, assigned in one pass by
  `ui::apply`) and `ui/app.slint` (sidebar 300/240 px, Inputs · Bus ·
  Listen, Listen stacked under 780 px; the window's size never comes from
  its content). Slint 1.19.1 without Qt: winit, femtovg, software renderer,
  accessibility. Its generated code needs `unsafe`, so the library denies
  `unsafe_code` and allows it only in the generated module.
- `daemon.rs` + `state.rs`: the IPC client re-reads `Describe` on
  `GraphChanged`, `Notice`, `Revision`/`Junctions`/`CaptureApps` changes
  and juncd's name owner coming or going; AppState turns it into the
  sidebar rows and juncd's state (`down` while away).
- Tests: Spec values for both built-in themes; a live swap done as
  `omarchy-theme-set` does it; one window retinted, resized and labelled
  on Slint's testing backend; juncd followed on a private bus. On the
  owner's M1 every one of Omarchy's 24 themes was applied in turn under
  one running window: the sidebar matched each theme's `dark_background`
  to the pixel (Osaka Jade: the built-in Tokyo Night), with no restart.
- CI installs `fontconfig` (Slint's text stack links it).

### 7.8 Tray

`ksni` StatusNotifierItem with DBusMenu: one check item per Junction/Cable,
"Capturing: …" line, Open JUNC (default), Quit (closes the GUI, the daemon
keeps running). The GUI process can run window-less (`junc-gui --background`)
for the tray; the window is created on demand and destroyed on close to free
renderer memory. On Omarchy the bar plugin is primary; the tray is offered when
an SNI host is present.

### 7.9 Accessibility checklist (from the validation report)

- `JSwitch`: label + on/off description.
- Own focus ring on sliders, cards, ports (WCAG 2.4.7).
- Picker root: dialog-like role and label; search field labelled.
- Every text field labelled; meters `accessible-role: none`.
- PageUp/PageDown ±10 on sliders.
- Tab order = reading order, explicit where layouts are computed.
- Re-run Orca and the interaction tests on Hyprland (owner laptop) per stage.
- Upstream Slint issues: dialog role for `PopupWindow`, layout direction.

As built (B13), checked on the gallery (`scripts/gui/gallery-check.sh`,
headless; `docs/design/gallery/`) and in `junc-gui/tests/components.rs`:

- Done: switch label + "on"/"off" (AT-SPI `toggle button name='Zoom Mic'
  desc='on'`); own focus rings on switches, sliders, cards, ports,
  channel groups (accent, outside the control); picker root a labelled
  group (Slint 1.19 has no dialog role) with a labelled search field and
  Tab kept inside; every text field labelled; meters `none`; PageUp/
  PageDown ±10; Tab order follows the layout and skips disabled controls.
- Found by the AT-SPI dump: sliders need a plain numeric
  `accessible-value` for AT-SPI's Value interface (`value=50 [0..100]
  step=1`), as the validation had it.
- Banners and toasts are polite live regions (`accessible-live-region`).
- Still open: Orca on Hyprland on the owner laptop (with the first screens,
  B16+), the upstream issues.

---

## 8. Localization

- **Languages**: 46 left-to-right languages of Omarchy's registry in v1;
  `ar`, `he`, `fa`, `ur` show English until RTL lands (PRD). The Settings
  language picker lists only the 46 (its note reads "46 languages", not the
  design's "50"); the RTL four are added with RTL. Registry =
  Omarchy's `default/i18n/locales.json` (vendored copy, updated by a CI job
  that diffs upstream).
- **Resolution order**: Settings override → `$OMARCHY_LANGUAGE_FILE` or
  `${XDG_CONFIG_HOME:-~/.config}/omarchy/language` (first line, whitespace
  stripped, registry code; unknown → English) → `LC_ALL`/`LC_MESSAGES`/`LANG`.
  Watch the directory for `IN_CLOSE_WRITE`/`IN_MOVED_TO` (Omarchy rewrites in
  place). Normalize codes to gettext form (`zh-CN` → `zh_CN`, `pt-BR` →
  `pt_BR`, `es-MX` → `es_MX`, `no` → `nb`).
- **GUI**: `@tr()` strings, `.po` catalogs under `po/gui/`, compiled into the
  binary with Slint bundled translations (`slint-build` `with_bundled_translations`)
  and switched live with `slint::select_bundled_translation`. This avoids the
  validation pitfall that gettext ignores `LANGUAGE` under a `C` locale.
  Spike S6 confirms it on Slint ≥ 1.19.
- **CLI / notices / tray**: `po/cli/*.po` compiled to `.mo`, loaded with the
  pure-Rust `gettext` crate from `/usr/share/locale/<lang>/LC_MESSAGES/junc.mo`.
  The daemon localizes only user-visible notice text it sends; logs are English.
- **Plurals**: CLDR via gettext plural forms; whole sentences with
  placeholders, never glued fragments.
- **Numbers**: formatted in Rust with ICU4X (`icu_decimal`, numbering system
  from the registry's `zeroDigit`), passed as `{0}`; `{n}` only selects the
  plural form (validated in Slint 1.19.1).
- **Fonts**: JetBrains Mono via the `monospace` alias; Noto/Noto CJK fallback;
  per-language `default-font-family` for `ja`/`zh-*`/`ko` Han variants.
- **Workflow**: English `.pot` extracted by `cargo xtask i18n extract`
  (`slint-tr-extractor` + `xgettext` for Rust); translations machine-drafted
  and marked `#, fuzzy`-free with a `X-Junc-Machine-Drafted: yes` header and a
  per-entry `#. machine-drafted` comment; Weblate later. CI fails on malformed
  catalogs, missing keys versus the template, or mismatched placeholders —
  never on pending translations. A pseudo-locale `en_XA` (+40 %, accented,
  bracketed) runs in screenshot tests.
- **Bar plugin**: its strings come from the same `.po` files, converted at
  build time to the JSON catalogs Omarchy plugins read through
  `Commons.Translations`.

As built (B11), in `junc-l10n`:

- `registry`: Omarchy's `locales.json` vendored (`data/`, Omarchy 4.5.0,
  MIT), read in file order; `v1()` is its 46 LTR languages. Codes are found
  in any form (`pt_BR`, `pt-BR`, `de_DE` → `de`, `nb` → `no`) and map to
  gettext names (`pt_BR`, `nb`).
- `resolve` answers a registry code: `--lang`/setting → Omarchy's file (an
  unknown code there is English, not a fall-through) → `LC_ALL`/
  `LC_MESSAGES`/`LANG` → `en`; RTL languages resolve to `en` until RTL.
- `mo` + `plural`: our own `.mo` reader and `Plural-Forms` evaluator. The
  `gettext` crate was dropped: it pulls the unmaintained `encoding` crate
  (RUSTSEC-2021-0153), which cargo-deny refuses. `set_language` loads
  `junc.mo` from `$JUNC_LOCALEDIR` or `/usr/share/locale` (own, else base
  language); `tr`/`trf`/`trn` look up there, missing messages stay
  English; the language is process-wide and switchable any time.
- `en_XA` is computed (no catalog): accented outside `{placeholders}`, +40 %,
  in `⟦…⟧`, as in the validation.
- `watch::spawn` follows the Omarchy language file with inotify on its
  directory (in-place writes and renames), reports only real changes, and
  never creates Omarchy's directory.
- `numbers` (feature `numbers`, on for the CLI and GUI, off for juncd):
  ICU4X `icu_decimal` with the registry's format locale and the numbering
  system its zero digit names (`arabext`, `beng`, …). juncd's release
  binary does not link ICU data (3.1 MB).
- Workflow: `cargo xtask i18n extract` (xgettext ≥ 0.24, `--add-location=file`
  so line moves do not change the template) → `po/cli/junc.pot`;
  `cargo xtask i18n check` (in `scripts/ci/check.sh`) fails on a stale
  template, malformed catalogs, missing messages, placeholder or plural-count
  mismatches, never on untranslated ones; `cargo xtask i18n build [DIR]`
  writes `.mo` files with junc-l10n's writer (packaging, B25, installs
  them to `/usr/share/locale`). First catalog: `po/cli/de.po`,
  machine-drafted and marked; the other 44 are V1.
- The CLI sets the language before every command except with `--json`,
  whose text stays English; doctor's ms and kHz use ICU4X.

As built (B16), the GUI: `cargo xtask i18n extract` also writes
`po/gui/junc-gui.pot` from the `@tr` of `crates/junc-gui/ui/` (gallery
excepted) with its own Slint literal reader, no `slint-tr-extractor`; the
messages have no context (the UI is compiled with
`DefaultTranslationContext::None`, so "Delete" is translated once);
`check` checks `po/gui/<lang>.po` like the CLI's. `build.rs` bundles them
plus an `en_XA` catalog made from the template with `junc_l10n::pseudo`.
`ui::set_language` selects the junc-l10n language (Omarchy's file,
followed by `watch::spawn`) right after a window is created: at that
moment Slint otherwise picks a catalog from the system locale by language
alone, and `en_US` matched `en_XA`. Lists of names use "、" in Japanese
and Chinese. First GUI catalogs: `de`, `ja`, machine-drafted with the
design's boards 33 and 34 as the term reference. Snapshots render
Japanese with a subset of Noto Sans CJK JP (OFL) registered as the
fallback for Japanese scripts.

---

## 9. Agent skill

### 9.1 Layout

```
/usr/share/junc/agent/skills/junc/
├── SKILL.md            frontmatter (name: junc, description with triggers), vocabulary,
│                       safety rules, the edit → validate → dry-run → confirm → apply loop
├── recipes.md          Zoom mic, stream mix, separate tracks, app→app cable, combine USB
│                       interfaces; each is a template + exact commands
├── junctions.md        inputs, bus, links, listen, volumes, on/off, nesting
├── devices.md          matching apps/devices, Pro Audio, Bluetooth caveat, Asahi DSP note
├── defaults.md         default input/output and restore behaviour
├── presets.md          preset/template format, slots, import/export
├── troubleshooting.md  junc doctor output → fixes; service, PipeWire, conflicts
└── reference.md        generated: CLI commands, --json shapes, error codes, schema summary
```

`reference.md` is generated by `cargo xtask skill-ref` from clap and the error
table; CI fails if it is stale (generated-file commit rule applies).

As built (A18): the skill lives in `skill/junc/` (installed to
`/usr/share/junc/agent/skills/junc/`); `presets.md` came with presets
(B8). `junc skill link` links into the folders above that exist and
never creates one; the automatic link on juncd's first start comes with
packaging.

### 9.2 Contents outline (`SKILL.md`)

1. When to use: any request about virtual mics, routing app audio, combining
   inputs, sending one app to another, listening to a mix, JUNC itself.
2. Vocabulary: Junction, Cable, Input, Bus, Ch 1–2, Link, Listen, Exclusive.
3. First step always: `junc describe --json`.
4. The safe change loop: write a config **copy** (`junc config path`, copy to
   a temp file) → `junc config validate FILE --json` → `junc apply FILE
   --dry-run --json` → show the user the `summary` lines in plain words → on
   confirmation `junc apply FILE --expect-revision N`. Never edit
   `junctions.toml` in place (the daemon applies saved edits immediately).
5. Small changes may use direct commands (`junc input add …`) after stating
   what will happen.
6. Templates first: `junc template list`, fill slots.
7. Reading errors: `code`, `location`, `hint`; fix and retry.

### 9.3 Safety rules (verbatim from the PRD, enforced in examples)

- Never delete a Junction, change the default device, or capture an app the
  user didn't name without asking first.
- Always validate and dry-run before applying; show the planned change in
  plain words.
- Capturing an app's audio is visible through the capture indicator (F23);
  say so.
- Edit only `~/.config/junc/`; never touch PipeWire or WirePlumber configs.
- Plus: destructive CLI calls need `--yes`; the agent passes it only after the
  user confirmed in the conversation.

### 9.4 Linking

- Same agent folders as `omarchy-agent-skills-link`: `~/.agents/skills`,
  `~/.claude/skills`, `~/.codex/skills`, `~/.pi/agent/skills`,
  `~/.gemini/config/skills`, `~/.hermes/skills`, `~/.hermes/profiles/*/skills`.
- pacman runs as root and cannot write user homes, so linking happens per
  user: `junc skill link` (idempotent), run automatically by `juncd` on first
  start of each package version (opt out in `settings.toml`), and offered in
  onboarding. A folder or link of the user's own named `junc` wins.
- Omarchy's linker only prunes links into its own tree
  (`*/default/agents/skills/*`), so it leaves JUNC's link alone.
- Upstream: PR to omacom/omarchy adds one line to the `omarchy` skill's audio
  section (`system.md`, "Network, Bluetooth, and Audio") pointing at the `junc`
  skill when installed.
- `junc skill unlink` and `junc teardown` remove the links.

### 9.5 Checks

`cargo xtask skill-check`: frontmatter valid, every command in a code block
parses with clap, every TOML example validates against schema v1, reference is
fresh. Acceptance (owner laptop, run by the developer): a coding agent with
only the skill turns "make a Zoom mic with my USB mic and Spotify" into a
working Junction, validated and dry-run first.

---

## 10. Packaging, repository and Omarchy integration

### 10.1 PKGBUILD (`packaging/arch/PKGBUILD`)

Split package, `arch=(x86_64 aarch64)`:

| Package | Contents | Depends |
| --- | --- | --- |
| `junc` | `juncd`, `junc`, user unit, D-Bus activation file, templates, skill, CLI `.mo` files, completions, man pages | `pipewire>=1.0`, `wireplumber`, `libpipewire` |
| `junc-gui` | `junc-gui`, `.desktop`, icons (hicolor + symbolic), Omarchy bar plugin, menu snippet | `junc`, `fontconfig`, `libxkbcommon`, `wayland`, `libgl`/mesa; optdepends `libx11` (X11) |

`pipewire-pulse` and `pipewire-jack` are optdepends (doctor/onboarding report
them). Source: `git+https://github.com/ejsergeev/junc.git#tag=v$pkgver`
(pinned, as omarchy-pkgs requires). `check()` runs `cargo test --workspace
--locked` (layer 1). `junc.install` handles `pre_remove` teardown (§3.13).
Channel builds: `junc-git` in the testing repo (pkgver from `git describe`).

### 10.2 Signed pacman repository

- Hosted on GitHub Pages of `ejsergeev/junc-pkgs`:
  `https://ejsergeev.github.io/junc-pkgs/$repo/$arch/` with repos `junc` (stable)
  and `junc-testing`. Each has `junc*.pkg.tar.zst`, `.sig`, and the signed db
  (`$repo.db.tar.zst` + `.sig`). Last 3 versions kept.
- Signing: the owner creates the OpenPGP key (primary key offline) and a
  signing subkey stored as a GitHub Actions secret in an environment that only
  release workflows can use. The public key ships in `junc-keyring`
  (pacman-key populate) and on the README with its fingerprint.
- User setup:

  ```ini
  [junc]
  Server = https://ejsergeev.github.io/junc-pkgs/$repo/$arch
  ```
  after `pacman-key --add` + `--lsign-key <fingerprint>` (bootstrap), then
  `pacman -S junc-keyring junc-gui`.

### 10.3 CI publishing

- Builds in clean Arch containers: x86_64 on `ubuntu-24.04` with
  `archlinux:base-devel`; aarch64 on `ubuntu-24.04-arm` with an Arch Linux ARM
  rootfs image built the way omarchy-pkgs does (pacstrap into a Docker image).
- `main` merge → `junc-git` → `junc-testing`.
- Tag `vX.Y.Z` → `junc`, `junc-gui` → `junc-testing`; promotion to `junc`
  (stable) is a separate workflow gated by a GitHub Environment that requires
  the owner's approval (release sign-off).
- Steps: build → `namcap` → install into a fresh container → smoke test
  (`juncd` + headless PipeWire, `junc doctor --json`) → sign → `repo-add` →
  upload packages, then signatures, then the database last (omarchy-pkgs
  order, so mirrors never see a db pointing at missing files).

### 10.4 Omarchy integration (F21, F23)

| Piece | v1 (JUNC ships) | Upstream later (owner opens PRs) |
| --- | --- | --- |
| Launcher entry | `junc.desktop` (Omarchy's app launcher reads XDG data dirs) | — |
| Menu entry "Audio routing · JUNC" under Setup, next to Audio | `junc omarchy install` adds a marked entry to `~/.config/omarchy/extensions/omarchy-menu.jsonc` (only with consent, idempotent, removable) | PR to omacom/omarchy: entry in `default/omarchy/omarchy-menu.jsonc` with `when: command -v junc-gui` + `menu.json` catalog key |
| Bar module (count of Junctions on, capture dot, popup with switches) | Quickshell plugin `io.github.ejsergeev.junc` (manifest kinds `bar-widget`, `service`), shipped in `/usr/share/junc/omarchy/plugin/`, symlinked into `~/.config/omarchy/plugins/` by `junc omarchy install` (Omarchy loads plugins only from there); data from `junc watch --json`, actions via `junc enable/disable`; validated with `omarchy plugin validate` | Optional: propose as a built-in plugin |
| Keybinding | Suggest Super+Shift+J (free in Omarchy's defaults) in onboarding and skill; never written automatically | — |
| Theme + language live | §7.7, §8 | — |
| Skill pointer | `junc skill link` | PR adds a pointer in the `omarchy` skill |
| Package | own repo | PR to omacom/omarchy-pkgs: `pkgbuilds/junc/` + `.omarchy/package.json` with a GitHub release watch |

---

## 11. Test strategy

### 11.1 Layers

| Layer | What | Where | Gate |
| --- | --- | --- | --- |
| Unit | config parse/validate/migrate, matcher, planner (never touches untagged objects), preset slots, i18n resolution, number formatting, theme derivation | CI, every PR | must pass |
| 1 · Mocked graph | `MockBackend` + fixtures for 1–64 ch devices, odd channel maps, appear/disappear, profile changes, Bluetooth props, Asahi DSP topology, foreign links; full daemon (D-Bus + reconcile) against the mock | CI, every PR | must pass |
| 2 · Virtual PipeWire devices | Headless PipeWire + WirePlumber (validation `ci-config`: null-audio-sink fake devices, no bluez/logind) in a container; real `juncd`; `pw-cat` test signals; record and compare bit-exact; app churn; PipeWire restart; re-exec upgrade continuity; 18-ch null device | CI, every PR (short), nightly (long soak) | must pass |
| 3 · Virtual kernel cards | `snd-dummy`, `snd-aloop` (up to 32 ch) through the real ALSA path, ACP profiles incl. pro-audio (F15); hot-plug simulated by switching the card profile `off`/on (unloading the module needs WirePlumber stopped) | Owner laptop (sudo once per session); CI attempt if the runner kernel has the modules | per stage |
| 4 · Emulated USB audio | UAC2 gadget via configfs on a virtual UDC | **Skipped** (owner agreed 2026-11-25): no `dummy_hcd` and no UDC on the owner's laptop. USB hot-plug is covered by `snd-aloop` reload and the real interface | — |
| 5 · Real devices | Native Instruments Komplete Audio 2 (USB class-compliant, 2 in / 2 out), MacBook built-in audio behind the Asahi DSP, Google Pixel Buds 2a and Apple AirPods (Bluetooth, F18). No HDMI audio on this laptop. The Komplete Audio 2 has only 2 channels, so F15 (more channels in Pro Audio) is proven on `snd-aloop` only | Owner | per stage |

### 11.2 Daemon integration scenarios (layer 2)

`scripts/ci/live-pipewire.sh` starts a headless PipeWire + WirePlumber +
pipewire-pulse with the validation `ci-config` (null-audio fake devices,
no bluez/logind), runs the live tests of `junc-pw`, `juncd` and `junc-cli`
against it one at a time, and stops it. Every live test removes what it
created, so tests that time things run on a clean graph. CI runs it on
every PR on x86_64 and aarch64 and fails if it takes 10 minutes or more
(about 20–40 s today). Systemd itself is covered by
`scripts/ci/systemd-user.sh` (§3.14).

As built (A16), requirement → test (`crates/<crate>/tests/<file>`):

| Req | Scenario | Test |
| --- | --- | --- |
| F1 | Create, route, disable, delete through the CLI | `junc-cli/cli.rs` `live_smoke` |
| F1 | A Junction is a device for PulseAudio (`pactl`), JACK (`pw-jack jack_lsp`) and native (`pw-cli`) apps | `juncd/live.rs` `every_audio_api_sees_a_junction` |
| F1/F9 | Build, adopt after restart, clean up | `junc-pw/live.rs` `builds_adopts_and_cleans_up_a_junction` |
| F2 | Four levels of nesting, noise bit-exact end to end | `juncd/live.rs` `nesting_four_deep_is_bit_exact` |
| F3/F4 | 4-ch Bus, Input linked to ch 3–4 only: bit-exact there, ch 1–2 silent | `juncd/live.rs` `custom_links_place_an_input_on_chosen_bus_channels` |
| F4 | Mono device → default mapping 1→1, 1→2 | `juncd/dbus.rs` `live_statuses_and_graph_changed`, `junc-cli/cli.rs` `live_smoke` |
| F5 | Cable between two apps, recorder fed by the Cable alone, bit-exact | `juncd/live.rs` `cable_delivers_one_app_to_another_bit_exact` |
| F9 | Ten Junctions restored < 1 s; restart adopts without recreating | `juncd/live.rs` `ten_junctions_restore_in_under_a_second` |
| F10 | 60 app restarts re-linked, p95 < 100 ms | `juncd/live.rs` `relinks_on_churn_and_does_not_fight_overrides` |
| F10 | A device unplugged and plugged back 10 times, re-linked every time | `juncd/live.rs` `a_device_that_comes_and_goes_is_relinked_every_time` |
| F16 | Default input set, restored, cleared; Listen follows the default output | `juncd/live.rs` `default_input_set_restored_and_cleared`, `listen_default_follows_the_system_output` |
| F17 | A JUNC link removed by another tool is reported, not recreated; foreign links untouched | `juncd/live.rs` `relinks_on_churn_and_does_not_fight_overrides` |
| F22 | Hand edits applied live; a bad edit keeps the last good config | `juncd/live.rs` `hand_edits_apply_live_and_bad_edits_keep_the_last_good_config` |
| F25 | A dry run changes no node, link or file; doctor sees a healthy session | `junc-cli/cli.rs` `live_dry_run_changes_nothing_and_doctor_is_happy` |
| §3.6 | PipeWire restarts: Junctions rebuilt (≈ 0.3 s after PipeWire is back) | `juncd/live.rs` `junctions_come_back_after_pipewire_restarts` |

| F20 | pacman replaces juncd mid-playback: it waits for the lock, re-execs in place, adopts every object; a counter signal arrives unbroken | `juncd/live.rs` `an_upgrade_reexecs_without_an_audio_gap` |
| F20 | `junc teardown` leaves no `junc.managed` object and gives the default input back; `pre_remove` runs it per logged-in user | `junc-cli/cli.rs` `teardown_leaves_no_junc_object`, `pre_remove_tears_down_every_logged_in_session` |

Still to come: the long nightly soak.

### 11.3 GUI tests

- **Screenshots**: `junc-gui --scenario X --lang L --theme T --size WxH
  --snapshot out.png` (software renderer, deterministic fonts installed in the
  CI image). Every PR: all 49 scenarios × {en, en_XA, de, ja} × tokyo, plus
  the flexoki and narrow variants the canvas has. Nightly: all scenarios × 46
  languages. Compared to goldens in `tests/golden/` with a perceptual
  tolerance; goldens update in their own commit. Overflow check: any text
  element reporting elision outside allowed elements fails.
- **Design comparison**: the developer compares goldens side by side with the
  reference renders in `docs/design/` (same numbering as §7.5) at each stage.
- **AT-SPI**: from the validation harness (`env-up.sh`, `atspi_dump.py`,
  `keyboard_walk.sh`, `interaction_test.sh`): role/name/state tree per
  scenario compared to an expected dump; Tab order; switch/slider/picker
  keyboard behaviour. CI on Xvfb + headless Weston.
As built (B14): `junc-gui --scenario X [--theme light|dark] [--size WxH]
[--snapshot OUT.png]` (feature `scenarios`). A fixture
(`tests/fixtures/scenarios/<name>.toml`) gives a config, a graph from
`tests/fixtures/graphs/`, juncd's state, the board's theme and size; the
mock IPC runs `Reconciler` on `MockBackend` until it settles and builds the
`Describe` juncd would send (`scenario.rs`). `--snapshot` renders offscreen
(an own Slint platform, software renderer, scale 1) with a font collection
holding only `crates/junc-gui/fonts/` (an 80 KB subset of JetBrains Mono
Nerd Font, OFL; `scripts/gui/subset-font.sh`), so pixels do not depend on
the machine: goldens made on the owner's M1 pass in CI's x86_64 and
aarch64 containers. `tests/snapshots.rs`: goldens in `tests/golden/gui/`
(≤ 0.2 % of pixels may differ by ≤ 24 per channel), a real change must
fail, renders are deterministic. `scripts/gui/atspi-check.sh` runs a
scenario in `scripts/gui/session.sh`'s private session (Xvfb, own bus,
AT-SPI bus and registry; X11 window via winit) and diffs the tree with
`tests/golden/atspi/`. `scripts/ci/check.sh` runs both on every PR (CI
installs Xvfb, at-spi2-core, python-gobject, a font and the X11 libraries
winit loads). Scenarios so far: `default` (board 1) and `gallery`; the
other boards, languages and Weston come with the screens that show them.

- **Main screen** (B16): boards 1–6 are scenarios `default`, `flexoki`,
  `narrow`, `sourceOff`, `offline`, `wireHover`. Fixtures describe their
  graph inline (`[[node]]`), may start from another (`base`) and turn
  Inputs `off`, `unplug` nodes or `hover` a link; the mock IPC applies the
  window's requests to the fixture's config and settles it again, so a
  scenario can be driven. Snapshots settle first (frames and timers), so
  the canvas has its real width. `tests/snapshots.rs` compares all six in
  en, en_XA, de and ja (24 goldens); `tests/main_screen.rs` drives the
  default scenario from the keyboard, pointer and switches (Space, Del on a
  port, a click on a link, Show Listen) and switches the language live;
  `check.sh` compares the AT-SPI trees of all six.

- **Onboarding, empty states, picker** (B17): boards 7–11, 28 and 29 are
  scenarios `picker`, `onboardCheck`, `onboardTemplates`, `emptyNone`,
  `emptyJunction`, `onboardFlexoki`, `emptyJunctionFlexoki`. Fixtures
  gained `onboarding` (the step), `[[check]]` (what doctor answers),
  `picker` (open it) and `select`; bases nest and add their nodes. 52
  goldens in four languages; `tests/onboarding.rs` drives the check (R,
  Copy, Enter), the templates (arrows, Esc, Enter on "Start empty"), New
  Junction (Ctrl+Shift+J) and the picker (Ctrl+N, "+", search, Esc,
  Enter, added rows refused); `check.sh` compares the AT-SPI trees of the
  five new screens.

- **Main screen states** (B18): board 18 is scenario `junctionOff`
  (fixture `off` takes a bare Junction id too), board 17 `addListen`
  (fixture `listen_picker`), board 16 `addBus` (fixture `add_bus`).
  `tests/states.rs` turns a Junction on with Space and with the banner's
  button and off again from its switch, adds Listen outputs from Ctrl+M,
  the picker and the empty Listen column, and adds Bus channels (Enter,
  ↓ for mono, Esc), and drives Input Options (Exclusive, a select by
  pointer and keys, Esc, Del); boards 15 and 30 are `inputOptions` and
  `inputOptionsNarrow` (fixture `options`, the card's index), and a
  Cable (`cable`, board 12: Exclusive, Listen on and to another output,
  both Change app… pickers), and links (`linkDrag`, `linkKeys`, fixture
  `link` with `link_keys`: a drag from Soundux onto Bus 1, Enter ↓ Enter,
  Enter Esc), and the keys of §7.4 (Ctrl+2, Ctrl+1, ↓ and → → with
  Space, Del on a hovered link); `check.sh` compares their AT-SPI trees.

- **Hidden-window gate** (B15, §7.6): `scripts/gui/hidden-check.sh` under
  `with-pipewire.sh` runs `--scenario default --meters` in the private
  session with a node playing under the Junction's name and a stand-in
  Hyprland (`scripts/gui/fake_hyprland.py`) that moves the window to
  another workspace: the meter stream and its links must be gone within
  1 s, the GUI process must take 0 clock ticks in 3 s, and the meters must
  return within 1 s. CI runs it after the layer-2 suite on both arches.

- **Orca**: `orca_test.sh` speech log on the owner laptop under Hyprland,
  keyboard via `wtype` with a focus guard (`interaction_test_wayland.sh`,
  `orca_test_wayland.sh`); at least 3 s between key presses so phrases are
  not cut off; owner listens once per stage.

### 11.4 Performance gates (PRD targets)

`bench/` emits `results.json`; CI compares with the last release's baseline
and fails on regression beyond noise (thresholds in `bench/thresholds.toml`):

| Metric | Target | Where |
| --- | --- | --- |
| Added latency | 0 extra quanta (plain), ≤ 1 with mixing | CI (impulse method), owner laptop (`jack_iodelay` with loopback cable if available) |
| Xruns on JUNC/device nodes | 0 over run (10 min CI, 24 h soak nightly/owner) | CI + owner |
| Bit-exact at unity | exact | CI |
| Scaling | linear at 1/8/16/64 ch, 1–20 Inputs, nesting 1–4 | CI nightly |
| Daemon PSS | < 10 MB with 10 Junctions; flat over 24 h | CI + owner |
| Daemon idle wakeups | 0/s (thread context switches over 60 s) | CI; `powertop` on owner laptop |
| Restore at login | < 1 s for 10 Junctions | CI + owner |
| Re-link | < 100 ms | CI |
| Recovery | < 1 s, 100 % links, JUNC's share: from PipeWire and the devices being back to all routed (owner, 2026-11-25); the total is tracked | CI |
| CLI/D-Bus | `time junc enable X` < 50 ms | CI |
| Daemon binary | < 5 MB stripped | CI |
| GUI cold start | < 300 ms to first frame with live state | owner (GPU); CI tracked, not gated |
| GUI PSS | < 80 MB | owner; CI tracked |
| GUI CPU with meters | < 2 % of one core at ~7 Hz | owner laptop (M1); the Pi 4 target is best-effort, no Pi available |
| GUI CPU hidden | 0 %; meter callbacks = 0 | CI gate (`hidden-check.sh`, §11.3) + owner (S7) |
| Frame time while dragging links | p95 < 16 ms | owner |

As built (A17): `crates/junc-bench` drives the real juncd (release build,
private D-Bus, ten Junctions) on the isolated PipeWire
(`scripts/ci/with-pipewire.sh`); `scripts/ci/bench.sh` runs it after the
layer-2 suite on every PR, compares with `bench/thresholds.toml` (the
targets above as `max`/`min`, plus `regress`/`slack` against
`bench/baseline/<arch>.json` recorded on the same CI runners) and checks
that the gate fails on an injected regression. Results are uploaded per
architecture. How each daemon metric is taken:

- Restore: process start → every Junction routed, seen by an observer.
- PSS: `/proc/<pid>/smaps_rollup` one second after routing, with the
  anon/file/shmem split and huge-page backing recorded alongside. juncd
  runs with one glibc malloc arena (unit `Environment=`): on Arch Linux
  ARM each extra arena of its helper threads was backed by a 2 MB
  transparent huge page, 9.8–11.5 MB PSS instead of ~5.5 MB.
  junc-bench starts juncd with transparent huge pages disabled
  (`PR_SET_THP_DISABLE`, inherited across exec): the same binary's heap
  got 2 or 4 MB of huge pages depending on the aarch64 runner, 5.8 or
  7.3–7.5 MB PSS, and failed the gate on runs that changed nothing in
  juncd. The shipped unit is unchanged.
- Idle wakeups: context switches of all juncd threads over 10 s idle.
- CLI: `junc enable|disable`, process start to exit, p95 of 20.
- Re-link: 30 app restarts, appearance → links back, p95.
- Recovery: three PipeWire restarts. Gated: from the devices and the
  default output being back to all routed (juncd's share, ~50 ms);
  tracked: from PipeWire's start (~1.3 s, mostly WirePlumber starting).
- Added latency: synchronous JACK clients on the graph clock hear a burst
  directly and through a Junction (the validation's method); 0 frames.
  Timing two `pw-cat` streams instead shows 2048–4096 samples even for a
  single plain node, depending on how the graph orders those
  asynchronous clients, so that number is not the Junction's latency.
- Bit-exact: seeded noise through a Junction against the direct copy.

First baselines (CI runners, 2026-11-25): PSS 5.9 MB (x86_64) / 5.5 MB
(aarch64), 0 idle wakeups, restore 114 / 147 ms, re-link p95 6 / 9 ms,
recovery 8 ms after the devices return (190–250 ms from PipeWire's
start), `junc enable` p95 3–36 ms, 0 added frames, bit-exact, juncd
3.1 / 2.9 MB.

### 11.5 CI vs owner laptop

| CI (every PR / nightly) | Owner's Omarchy laptop (developer runs, owner assists) |
| --- | --- |
| build x86_64 + aarch64, clippy, fmt, deny | layer 3 (sudo once), layer 5 with real devices |
| unit + layer 1 + layer 2 | Hyprland-specific: theme/language live reload, visibility, bar plugin, menu |
| screenshots, pseudo-locale, overflow | Orca speech under Hyprland; owner listening test |
| AT-SPI dumps (Xvfb/Weston) | GPU renderer perf, `powertop`, 24 h soak |
| i18n catalog checks, schema freshness, skill checks | Agent acceptance run with the skill |
| perf gates vs baseline | release walkthrough per stage |
| package build, namcap, install smoke test | install from the signed testing repo |

---

## 12. Verification spikes

Short, throwaway experiments at the start of alpha, each ending in a note in
this file (and a changed design if it fails):

| ID | Question | Pass criterion | Fallback |
| --- | --- | --- | --- |
| S1 | Which `media.class` makes a server-side null-audio-sink appear as both sink and source to pulse, JACK and native apps? | `Audio/Duplex` listed by `pactl list short sinks` **and** `sources`, `jack_lsp`, `wpctl status` | Two nodes per Junction: `Audio/Sink` + `Audio/Source/Virtual`, linked |
| S2 | Class/props that hide gain nodes from device lists | Not listed in pulse/JACK/`wpctl` Sinks/Sources, still linkable | `node.virtual`/`node.hidden` props; or `Stream/*` class adapter |
| S3 | Custom `junc.*` props on lingering links are visible in the registry and survive the creating client's exit | `pw-dump` shows props after `juncd` exits | Store ownership in a JUNC-owned metadata object (`junc.links`) |
| S4 | Gain node at 100 % is bit-exact; `monitor.channel-volumes` semantics | Null test passes | Direct link at 100 %, gain node only when ≠ 100 % |
| S5 | Exclusive via `target.object` metadata with WirePlumber 0.5 | Stream moves; WP link to speakers gone; restored on clear | Document Exclusive as P2 limitation |
| S6 | Slint bundled translations + live switch + ICU4X `{0}` | Language switch in place, AT-SPI names update | gettext runtime with `setlocale` to a UTF-8 locale |
| S7 | Hidden-window detection on Hyprland (minimize, other workspace, closed) | Meters stop within 1 s in each case | Hyprland IPC only on Omarchy; frame starvation elsewhere |
| S8 | Asahi DSP chain: linking Listen to the default filter-chain sink, never raw speakers | No direct link to speaker ALSA nodes in any test | Exclude `alsa_output.*` with `api.alsa.path` of protected speakers |

### 12.1 Spike results (2026-11-25, PipeWire 1.6.9, WirePlumber 0.5.18)

Scripts: `bench/spikes/`. All ran in an isolated instance on the owner's M1.

| ID | Result | Consequence |
| --- | --- | --- |
| S1 | **Fallback taken.** `Audio/Duplex` is listed by PulseAudio as both sink and source and by JACK, but WirePlumber does not link recording apps to it: `parecord -d`, `pw-record --target` and the default source all fall back to another source. `Audio/Source/Virtual` is linked in all three cases. Playback into Duplex/Internal nodes is linked. | A Junction is two nodes: `junc.<slug>` (`Audio/Source/Virtual`, what apps record; Inputs are linked into its input ports) and `junc.<slug>.sink` (`Audio/Sink`, what apps can play into, F1/F16 output) whose monitor feeds the source. Owner chose Loopback-like behaviour: both nodes always (2026-11-25). |
| S2 | `Audio/Duplex/Internal` is hidden from PulseAudio and `wpctl` device lists but linkable; `Audio/Sink/Internal` still shows in `wpctl`; `node.hidden`/`node.virtual` hide nothing. JACK lists every node's ports. | Gain nodes use `Audio/Duplex/Internal`. They are visible in JACK patchbays, like PipeWire's own filter-chain nodes. |
| S3 | **Pass.** A link created through the link factory with `object.linger=true` and `junc.*` props stays after its creator exits, with all props in the registry; without linger it disappears. Prop values containing `:` or `>` must be quoted in SPA-JSON. | Ownership tags on links as designed (§3.2). |
| S4 | **Pass.** pw-cat → gain node → Junction → recorder is bit-exact at 100 % and exactly ×0.5 at 50 % (144 k frames, max error 0). | No direct-link fallback needed. |
| S5 | **Pass.** With a JUNC link stream→gain in place, setting `target.object` on the stream to its gain node makes WirePlumber drop its link to the default sink and keep JUNC's; clearing the key restores the default-sink link. | Exclusive targets the Input's gain node (not the Junction). |
| S6 | **Pass** (B11, Slint 1.19.1, testing backend). `.po` catalogs bundled with `with_bundled_translations`; `select_bundled_translation` switches en → de → ru → bn → en in one process and every `@tr` and accessible label follows; an unbundled language is refused and the UI keeps its own. `@tr("{0} input connected" \| "{0} inputs connected" % inputs, inputs-text)` picks the form from the int and shows ICU4X's text (`1.234`, `২১`). `bench/spikes/s6/`. | GUI as designed. Its catalogs carry `msgctxt` = component name (Slint looks messages up that way); element lookups in GUI tests need Slint debug info. |
| S7 | **Pass on Hyprland through its IPC** (B15, 2026-11-26, Hyprland 0.56.2, owner's M1; `bench/spikes/s7-visibility.sh`, window on a temporary headless monitor, isolated PipeWire). Meters stop 14–29 ms after the window goes to another workspace or a hidden special workspace (how Omarchy minimizes) and 10–16 ms after it is closed; 0 CPU ticks in 3 s while hidden; back 13–19 ms after it returns. Visible with two sidebar meters: ~1 % of a core (debug build, default renderer); B16, every column metered (19 meters, seven playing nodes), release build: 0.6 % of a core on a 1920×1080 headless monitor. With the IPC out of reach nothing stops the meters: Hyprland keeps drawing hidden windows at `misc:render_unfocused_fps` (15), so frames never starve. | Hyprland IPC is the signal on Omarchy; frame starvation stays as the fallback for other compositors, checked by unit tests only so far. Hyprland 0.56 dispatchers are Lua (`hl.dsp.*`). |
| S8 | **Pass by construction.** In the live session the speaker (node 62) and mic-array (node 56) ALSA nodes are inaccessible (`no global`), only their links are visible. | The planner only ever targets nodes it can see; test kept on the Asahi fixture. |

| S9 | Registry globals carry only part of the properties. Nodes: no `junc.*` tags, no `application.process.binary`, no `audio.channels`. Links: no `junc.*` tags. Ports: everything needed (`port.id`, `audio.channel`, direction, `node.id`). | `junc-pw` binds every audio node and every link to read their full info props (cheap client-side proxies, no objects created); ports are read from globals. |

| S11 | A recording app linked by WirePlumber to the default source (`fake.hwin`), plus a JUNC link from a Cable's source: the app hears both. Setting the stream's `target.object` to the Cable's source makes WirePlumber drop the default-source link and link the Cable itself (both channels into a mono stream); clearing it restores the default source and also removes the links from the Cable, JUNC's included. | Cables use `target.object` only and leave the linking to WirePlumber; JUNC links into the target app are gone. Bit-exact through a Cable confirmed by the A13 live test (s16 → s16, 1.46 s of noise, 3 runs). |
| S10 | `node.description` cannot be changed after creation (`set-param Props` ignores it). When a node is destroyed, WirePlumber moves its recording apps (PulseAudio and native) to the default source and back once a node with the same `node.name` reappears. | Renaming recreates the node, but only while no foreign link touches it; otherwise the rename is *deferred* (the GUI shows the new name at once, PipeWire gets it when the Junction is idle), so a recording app never hears the real mic for a moment. Gain nodes carry id-based descriptions and are never renamed. A Bus resize still recreates at once; the GUI warns when the Junction is in use. |

Other findings: ports must be looked up by direction and `audio.channel`, never
by name (`Audio/Source/Virtual` names its inputs `input_FL`, sinks use
`playback_FL`), and links can only be created once a new node's ports exist
(wait for the port globals, not just the node).
