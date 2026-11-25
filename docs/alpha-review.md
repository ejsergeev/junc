# Alpha exit review

Date: 2026-11-25. Scope: `juncd` + CLI + agent skill (docs/plan.md,
"Alpha"). Main at the end of A19. The tag `v0.1.0-alpha` is made only
after the owner says go.

## Verdict

**Go, with the known issues below.** Every alpha requirement works from
the CLI and is proven by automated tests on both architectures, and the
ones that depend on real hardware were run on the owner's laptop. One
measured target (re-link in a busy live desktop) sits at its edge and is
a beta task; nothing found blocks using the alpha.

## Exit criteria

"F1–F5, F9, F10, F16, F17 work from the CLI; all agent checks green in CI
(x86_64 + aarch64) and on the owner's laptop."

| Req | What | From the CLI | Automated proof (CI, both arches) | Owner laptop (A19, live session) | |
| --- | --- | --- | --- | --- | --- |
| F1 | Virtual devices apps can pick | `junc create junction`, `list`, `status`, `delete` | `live_smoke`; `every_audio_api_sees_a_junction` (PulseAudio, JACK, native) | three Junctions in the live session, used by apps and heard | ✅ |
| F2 | Inputs: apps, devices, monitors, other Junctions | `junc input add --app/--device/--monitor/--junction` | `commands_change_the_config_and_keep_comments`; `nesting_four_deep_is_bit_exact`; loops refused (`nesting_loops_are_refused_before_any_change`) | Komplete Audio 2 and app Inputs | ✅ |
| F3 | Bus channels | `junc bus add/remove` | `custom_links_place_an_input_on_chosen_bus_channels` (4-ch Bus, bit-exact on 3–4, 1–2 silent) | — (2-channel devices) | ✅ |
| F4 | Channel links | `junc link/unlink` | same test; default mapping in `live_statuses_and_graph_changed` | default mapping on the Komplete and snd-aloop (bit-exact through the real ALSA path) | ✅ |
| F5 | Cables, app → app | `junc create cable` | `cable_delivers_one_app_to_another_bit_exact` (recorder hears the Cable alone) | — (no second recording app needed in the live run) | ✅ |
| F9 | Survive login, restart, crash | `junc service enable/status` | systemd user session in CI: login with no GUI → 10 Junctions routed in 230–300 ms; `ten_junctions_restore_in_under_a_second`; restart adopts without recreating | PipeWire restart, suspend/resume, juncd survived both | ✅ |
| F10 | Re-link when apps and devices come and go | — (automatic) | 60 app restarts, p95 6–9 ms; 10 device re-plugs; PipeWire restart recovery | Komplete re-plugged twice, re-linked within 100 ms; app restarts p95 81/90/101 ms | ⚠️ see known issue 1 |
| F16 | Junction as default input/output | `junc default`, `junc switch` | `default_input_set_restored_and_cleared`, `listen_default_follows_the_system_output`, defaults unit tests | set and cleared live; a restore bug found and fixed, re-run live | ✅ (after the fix) |
| F17 | Never fight other tools | — (automatic) | `relinks_on_churn_and_does_not_fight_overrides`; Cable stream targets held when moved | link removed with `pw-link -d`: reported, not recreated, Restore works | ✅ |

Agent checks:

| Check | CI x86_64 | CI aarch64 | Owner laptop |
| --- | --- | --- | --- |
| `cargo xtask skill-check` (commands parse, examples validate, reference fresh) | ✅ in the test suite | ✅ | ✅ |
| Agent with only the skill builds the Zoom mic, dry run first | — | — | ✅ two runs in an isolated sandbox on the laptop |
| JSON contracts (`--json`, golden plans, validation, describe, schema) | ✅ | ✅ | ✅ |
| Performance gate against PRD targets and baselines | ✅ | ✅ | ✅ (bench runs locally) |

## Measured against the PRD (daemon)

| Metric | Target | CI x86_64 | CI aarch64 | Owner laptop |
| --- | --- | --- | --- | --- |
| Added latency | 0 extra quanta | 0 frames | 0 frames | 0 frames |
| Bit-exact at unity | exact | ✅ | ✅ | ✅ (also through snd-aloop) |
| Daemon PSS, 10 Junctions | < 10 MB | 5.9 MB | 5.5 MB | 5.5 MB |
| Idle wakeups | 0/s | 0 | 0 | 0 |
| Restore at login, 10 Junctions | < 1 s | 114 ms | 147 ms | 0.4–0.7 s from process start; 230–300 ms from login in the systemd test (CI) |
| Re-link | < 100 ms | p95 6 ms | p95 9 ms | isolated p95 ~45 ms; live desktop p95 81–101 ms |
| Recovery after PipeWire restart | < 1 s | 8 ms juncd share (190 ms total) | 8 ms (250 ms) | ~50 ms juncd share; 1.3–1.5 s total, mostly WirePlumber and the Asahi DSP chains starting |
| `junc enable` | < 50 ms | p95 3–5 ms | p95 5–36 ms | p95 9–12 ms |
| Daemon binary | < 5 MB | 3.1 MB | 2.9 MB | 2.9 MB |

## Known issues

1. **Re-link in a busy live desktop is at the target's edge** (p95 81, 90,
   101 ms over three runs on the owner's laptop; 6–9 ms in CI). Part of it
   is the measuring observer's own batching. Beta task B29.
2. **Recovery after a PipeWire restart takes 1.3–1.5 s in total** on the
   laptop. juncd's own share is ~50 ms; the rest is PipeWire, WirePlumber
   and the Asahi DSP chains starting. If the PRD's "< 1 s" means the total,
   it cannot be met by JUNC alone. Needs an owner decision.
3. **Volume is stored but not applied** (Junction, Input, Listen). The
   skill tells agents and users so. Beta task B1.
4. **Exclusive is stored but not applied.** Beta task B2.
5. **F16 restore can change the configured default**: when the configured
   device is unavailable and WirePlumber would keep a Junction, JUNC makes
   the device that was in use the configured default (the unplugged one
   stops being configured). Chosen over leaving the user on a Junction.
6. **Leave as is for a Cable's stream is kept in memory only**: after a
   daemon restart JUNC sends the stream to the Cable again.
7. **D-Bus API v1.0 has the CLI's surface only** (per-entry methods,
   import, doctor, bug report come as minor versions).
8. **Komplete Audio 2 does not come back after suspend** until re-plugged
   (kernel/USB on Asahi, not JUNC). Worth reporting upstream separately.
9. **Not in the alpha:** GUI, tray, bar plugin and capture indicator (F23),
   presets and templates, import of existing setups, Pro Audio switching,
   Bluetooth handling, translations (messages are English), the signed
   pacman repository and the PKGBUILD, linking the skill on juncd's first
   start.

## Owner decisions needed

- **Go / no-go** for tagging `v0.1.0-alpha` on main.
- **Known issue 2**: does the 1 s recovery target mean JUNC's share (met:
  ~50 ms) or the whole audio stack coming back (1.3–1.5 s on the laptop)?

## After go

The developer tags main as `v0.1.0-alpha` (annotated, by the owner's
identity, no AI mention) and pushes the tag; beta starts with B1.
