# JUNC D-Bus API v1

juncd serves this API on the **session bus** as `io.github.ejsergeev.Junc1`.
The CLI, the GUI and scripts all go through it; nothing else changes the
config while the daemon runs, except hand edits of `junctions.toml`, which
the daemon picks up itself.

- Design and rationale: [architecture §4](architecture.md#4-d-bus-api-v1).
- Exact signatures: [`dbus/io.github.ejsergeev.Junc1.xml`](dbus/io.github.ejsergeev.Junc1.xml),
  generated from the running daemon. After changing an interface, run
  `JUNC_BLESS=1 cargo test -p juncd --test dbus introspection`.
- Rust clients: `junc_ipc::DaemonProxy`, `junc_ipc::JunctionProxy`, the JSON
  types in `junc_ipc::view` and `junc_ipc::error_body`.

## Conventions

- **Revisions.** Every accepted config change bumps `Revision`. Methods that
  change the config return the new revision. `Apply` takes the revision your
  change is based on and fails with `Conflict` if someone (GUI, editor,
  another agent) changed the config since; pass `0` to skip the check.
- **JSON.** Structured values are JSON strings with the same shapes
  `junc … --json` prints. Ignore keys you do not know; new keys arrive in
  minor versions.
- **Errors.** Error names are `io.github.ejsergeev.Junc1.Error.<Name>`
  (`NotFound`, `InvalidConfig`, `Conflict`, `PipeWireUnavailable`,
  `NeedsConfirmation`, `Failed`). The message is JSON:

  ```json
  {"code": "E_CONFIG_RANGE", "message": "volume is 900",
   "hint": "Volume is a percentage from 0 to 150.",
   "diagnostics": [{"code": "E_CONFIG_RANGE", "severity": "error",
     "message": "…", "location": {"file": "…", "line": 7, "column": 10},
     "hint": "…"}]}
  ```
- **Single instance.** A second juncd cannot take the name and exits with
  status 4.
- **Latency.** Calls answer in about 1 ms (p95 ≈ 2 ms on the reference
  laptop; the test budget is 50 ms).

## Objects

| Path | Interface |
| --- | --- |
| `/io/github/ejsergeev/Junc1` | `io.github.ejsergeev.Junc1.Daemon` |
| `/io/github/ejsergeev/Junc1/junction/<id>` | `io.github.ejsergeev.Junc1.Junction` |

In paths, characters of the id outside `[A-Za-z0-9]` become `_xx` (hex):
`podcast-mix` → `podcast_2dmix`. Read `Junctions` instead of building paths.

## `Daemon`

| Member | Signature | What it does |
| --- | --- | --- |
| `Describe` | `() → s` | Everything at once: state, revision, config errors, Junctions with statuses and matched nodes, apps being captured, default devices, nodes you could route (`candidates`), sound cards with profiles, input channels and the Pro Audio offer (`devices`, since 1.1). |
| `GetConfig` | `() → (s, t)` | The config text in use and its revision. |
| `Validate` | `(s toml) → s` | JSON list of diagnostics; `[]` means valid. Changes nothing. |
| `Plan` | `(s toml) → s` | Dry run against the live graph: `{"actions": [...], "deferred": [...], "junctions": [...]}`. Needs PipeWire. |
| `Apply` | `(s toml, t expected) → (t, s)` | Validates, writes the text as given, routes it. Returns the revision and the plan that ran. |
| `CreateJunction` | `(s name) → (o, t)` | New empty Junction; the id comes from the name. |
| `ResolveLink` | `(s junction, s ref, s action) → t` | After a `link_overridden` notice: `restore` puts the link back, `leave` keeps it removed (also after restarts). |
| `GraphChanged` | signal `(t)` | Statuses changed; re-read what you show. |
| `Notice` | signal `(s kind, s json)` | See below. |
| `ApiVersion` | property `(uu)` | `(1, 1)`: 1.1 added `devices` to `Describe`, `bluetooth` on headset mics and cards, the `set_device_profile` plan action and the `profile_changed` notice. |
| `DaemonVersion` | property `s` | Package version. |
| `State` | property `s` | `starting`, `connected`, `waiting_for_pipewire`. |
| `Revision` | property `t` | Config revision. |
| `ConfigError` | property `s` | JSON diagnostics when the file on disk is invalid, else `""`. |
| `Junctions` | property `ao` | In config order. |
| `CaptureApps` | property `as` | Apps whose audio goes into an enabled Junction now. |

Notices:

| Kind | Body |
| --- | --- |
| `link_overridden` | `{"junction": "zoom", "ref": "in/mic/1->1"}` — another tool removed a JUNC link; answer with `ResolveLink`. |
| `default_restored` | `{"kind": "input", "to": "alsa_input.…"}` — JUNC put the previous default back. |
| `default_taken_over` | `{"kind": "output", "junction": "zoom"}` — the user chose another default elsewhere; JUNC stops claiming it. |
| `profile_changed` | `{"junction": "studio", "input": "desk", "profile": "output:analog-stereo+input:analog-stereo"}` — the user switched a card JUNC keeps in a profile; JUNC keeps their choice and saved it as the Input's `profile` (`null`: the card was turned off; the config is kept). |
| `config_error` | `{"diagnostics": [...]}` — a hand edit did not validate; the last good config stays in use. |

## `Junction`

| Member | Signature | What it does |
| --- | --- | --- |
| `Rename` | `(s) → t` | New display name; the id stays. |
| `SetEnabled` | `(b) → t` | On or off. |
| `SetVolume` | `(u) → t` | Master volume in percent (0–150). |
| `SetDefault` | `(s) → t` | `none`, `input` or `output`. |
| `Delete` | `(b confirm) → t` | Removes it; `false` fails with `NeedsConfirmation`. |
| `Id`, `Name`, `Enabled` | properties | |
| `View` | property `s` | The Junction as JSON, as in `Describe`. |

Edits through these methods keep the comments and layout of
`junctions.toml`. Everything else (inputs, links, listens, bus channels) is
changed with `GetConfig`, an edit of the text, and `Apply`.

## Examples

```sh
# What is there?
busctl --user --json=short call io.github.ejsergeev.Junc1 /io/github/ejsergeev/Junc1 \
  io.github.ejsergeev.Junc1.Daemon Describe | jq '.data[0] | fromjson'

# Turn a Junction off.
busctl --user call io.github.ejsergeev.Junc1 \
  /io/github/ejsergeev/Junc1/junction/zoom \
  io.github.ejsergeev.Junc1.Junction SetEnabled b false

# Watch notices and status changes.
busctl --user monitor io.github.ejsergeev.Junc1
```

Safe editing loop for agents: `GetConfig` → edit the text → `Validate` →
`Plan` → `Apply(text, revision)`; on `Conflict`, start again from
`GetConfig`.
