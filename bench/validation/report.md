# JUNC — validation of open questions

Date: 2026-11-24 · Scope: four checks (name, Slint a11y, Slint i18n, performance) · Artefacts: `junc-validation/` (scripts, prototype, raw results, screenshots)

> **Test environment (read first).** No Arch/Omarchy/Hyprland machine was available: the linked computer is a MacBook (macOS arm64). Everything ran in a cloud Linux VM: Ubuntu 24.04, 2 vCPU, 7 GB RAM, **no GPU, no sound card**, PipeWire **1.0.5** + WirePlumber 0.4.17, Slint **1.19.1** (released the same morning), Rust 1.97, Orca 46.1. Display = Xvfb (X11) and headless Weston (Wayland). Audio devices were `support.null-audio-sink` stand-ins. So anything that depends on Hyprland, real speech output, real audio hardware, a GPU, or real power counters is marked **not verified** below, and every number is a VM number, not an Omarchy laptop number. The scripts are written to be rerun on the target box.

## Summary

| # | Check | Verdict | One-line result | Top recommendation |
|---|---|---|---|---|
| 1 | Name availability | **RISK** (low) | `junc` is free on AUR, Arch repos, crates.io and Flathub. It is taken on npm (an unrelated Windows-junction module) and as the GitHub username. No live **JUNC** trademark in classes 9/42 in the US or EU. Nearby marks: JUNC++ION (US, cl. 9), Junco and Junctar (EU, cl. 9/42). GNOME's *Junction* app is in Arch `extra` and Flathub. | Keep `junc` for packages and crates; use a GitHub org such as `junc-audio`. Do a lawyer-run clearance before you spend on branding. |
| 2 | Slint accessibility | **RISK** | Roles, names, values and states reach AT-SPI. Orca reads switches, sliders, buttons and the popover. Tab order, the popover focus trap and Esc all work. Gaps: the switch is announced as "toggle button pressed"; the slider focus ring is almost invisible; the popover is exposed as a "panel" rather than a dialog; the search field has no name; Page Up/Down do nothing. **Not tested on Hyprland.** | Ship your own focus ring, labels and popover semantics. Rerun `orca_test.sh` on Hyprland before you commit. |
| 3 | Slint i18n / RTL / scripts | **FAIL** (RTL mirroring) | Pass: gettext `@tr`, live switch without restart, Arabic joining, Devanagari/Bengali/Sinhala conjuncts, Thai line breaking, font fallback to Noto/CJK, Arabic 6-form and Russian plurals. Fail: Slint 1.19 has **no layout direction**. Mirroring by hand flips the visuals but breaks Tab order. Sliders, switches and Left/Right keys stay LTR. Gap: `{n}` always prints ASCII digits, but Omarchy's own registry wants ۰-based digits for `fa` and ০-based digits for `bn`. Urdu falls back to Naskh, not Nastaliq. | Budget an RTL layer (mirrored layout components, own slider and switch) or push for upstream `layout-direction` support. Format numbers in Rust (ICU4X). |
| 4 | Performance | **RISK** | JUNC adds **0 quanta** of latency. The junction nodes had **0 xruns** in 10 min. The daemon used **1.7 MB PSS with 0 wakeups/s** when it only creates links (4.9 MB and 140 wakeups/s when it hosts loopbacks in-process). PSS over the 1 h churn went 4.9 → 5.0 MB. Re-link over 63 app restarts: p50 3.8 ms, max 14.9 ms. Recovery after a PipeWire restart: 0.37–0.41 s after fixing a reconnect-backoff bug. Fake-app clients logged ERRs that an A/B test traced to VM load, not to JUNC. The Slint prototype used 24 MB PSS and 3.3 % CPU with 7 Hz meters (software renderer), and the same CPU when hidden. | Keep the daemon off the audio path: server-side nodes plus links, no in-process loopbacks. Stop meter updates when hidden. Repeat on real hardware. |

Targets for check 4: 0 extra quanta ✅ · 0 xruns ✅ on junction/device nodes (see the note on client ERRs) · daemon < 10 MB PSS ✅ · ~0 idle wakeups ✅ in links mode, ❌ in loopback mode · re-link < 100 ms ✅ · recovery < 1 s ✅ (in this VM).

---

## 1. Name availability — RISK (low)

**Evidence** (`namecheck/check_names.sh`, output in `namecheck/namecheck-output.txt`, `namecheck/fallback-output.txt`; trademark log in `namecheck/trademarks-2026-11-24.md`; screenshots `screenshots/tm-uspto-junc.jpg`, `screenshots/tm-euipo-junc.jpg`)

| Registry | `junc` exact | Notes |
|---|---|---|
| AUR (RPC v5 = what `yay -Ss` queries) | free | names containing "junc": `junction-git`, `junction-zhfix` (GNOME app chooser), `otf-junction`, `ttf-junction`, `perl-syntax-keyword-junction` |
| Arch official repos | free | `extra/junction` 1.13: GNOME "Application chooser", apps.gnome.org/Junction |
| Flathub | free | only `re.sonny.Junction` matches |
| crates.io | free | `junc-daemon`, `junc-gui`, `junc-cli` also free; nearby: `junction`, `juncture*`, `junco`, `junctor` |
| npm | **taken** | `junc` (Windows NTFS junctions, created 2015); `junc-cli`, `@junc-cli/*`, `@junc/*` also exist. Irrelevant to a Rust/Arch app. |
| GitHub | user **`junc` taken** (a personal account, 4 repos) | 8.1k repos contain "junc" (mostly "junction"). No audio project named junc found. `junc-audio`, `juncbox`, `pw-junc` orgs/users are free (HTTP 404). |
| USPTO (tmsearch, wordmark contains "junc") | no live JUNC | 5 hits: JUNC ×2 (dead, class 25 clothing), JUNC HYGX (cl. 21), JUNC STRIKE (cl. 12), **JUNC++ION (live, cl. 9, Codemesh, software-development tools)** |
| EUIPO eSearch plus ("junc", 89 results) | no JUNC | live/pending in 9/42: **Junco** (7, 9), **Junctar** (9, 42, published app.), JUNCTEK (9), JunctionHub (9), JUNCESS, JUNCHUANG, JUNCAUTO, JUNCTION (35/41/42), CLEAR JUNCTION, JUNCTION LINK, … |

`yay` itself was not run (no Arch here). It queries the same AUR RPC that the script calls, so the result should match. Run the script on the Omarchy box to confirm.

**Not covered:** national EU registers (DPMA, INPI, …), WIPO marks that don't designate the EU or US, common-law use, and app-store names. This is not legal advice.

**Conflicts / risks**
1. Inside the product the virtual devices are called "Junctions". GNOME **Junction** ships in Arch `extra` and on Flathub, so a user typing "junction" in a package manager finds a different app. That is a UX collision, not a legal one.
2. JUNC++ION (US cl. 9 software) and Junctar (EU 9/42) are the closest marks. They are low risk for a free audio utility, but a lawyer should look.
3. `github.com/junc` is taken, so you need an org name.

**Fallback package names** (all free on AUR, Arch, crates.io, npm, Flathub and as GitHub owners as of today): **`junc-audio`**, **`juncbox`**, **`pw-junc`**. Recommendation: keep `junc` (binary `junc`, daemon `juncd`, crates `junc-*`) and register the GitHub org `junc-audio` now.

## 2. Slint accessibility prototype — RISK

**Build:** `proto/` (Slint 1.19.1, winit backend, AccessKit → AT-SPI). The main screen mock has a sidebar list, three card columns (Inputs → Bus → Listen), a Switch, Slider and level meter per card, buttons, and an "Add input…" popover with a searchable list (`PopupWindow`, `close-on-click-outside`).

**Method:** Xvfb + session D-Bus + `at-spi-bus-launcher` (`a11y/env-up.sh`); AT-SPI dump with python gi `Atspi` (`a11y/atspi_dump.py`, same data accerciser shows); keyboard-only via xdotool (`a11y/keyboard_walk.sh`, `a11y/interaction_test.sh`); **real Orca 46.1** with `--debug-file`, with what Orca would speak pulled from its log (`a11y/orca_test.sh`). The app was also started on **headless Weston (Wayland)**: the AT-SPI tree was identical to X11 (`a11y/tree-en-wayland.txt`). Keyboard injection and Orca were **X11 only**. **Hyprland: not verified.** No speech was heard (no audio device); the evidence is Orca's own `SPEECH OUTPUT` log lines.

**AT-SPI roles/names/states** (`a11y/tree-en.txt`)

| Widget | AT-SPI role | Name | State / value | OK? |
|---|---|---|---|---|
| Window | frame | "JUNC" | — | ✅ |
| Sidebar `StandardListView` | list box → list item | "Junctions" / item text | selectable, selected | ✅ |
| `Button` | push button | text | focusable, action click | ✅ |
| `Switch` | **toggle button** (AT-SPI 2 has no switch role) | from `accessible-label` | `pressed` = on (Discord off → no `pressed`). `focusable` appears only when focused. | ⚠️ works, but announced as "toggle button pressed / not pressed" rather than "switch on/off" |
| `Slider` | slider | label | Value iface 0..100, step 1, live | ✅ |
| Level meter (custom `Rectangle`, `progress-indicator`) | progress bar | label | Value iface **only if `accessible-value` is a plain number**. With "40%" it became `indeterminate` and had no Value interface. | ⚠️ easy to get wrong |
| Card | panel (from `groupbox`) | card title | — | ✅ |
| Popover | **panel** "Add input" nested under the "Add input…" button | — | no dialog/window role | ⚠️ |
| Search `LineEdit` | entry | **empty name** | editable. Orca reads the placeholder. | ⚠️ add `accessible-label` |

**Keyboard** (`a11y/tab-order-en.txt`, `a11y/interaction-en.txt`)
- Tab order follows reading order, 17 stops, then wraps: list → New junction → Add input… → (switch, slider) per card, column by column. Shift+Tab reverses it. ✅
- Switch: Space and Return both toggle, and the `pressed` state updates. ✅
- Slider: Left/Right ±1, Home → 0, End → 100 ✅. **Page Up/Page Down do nothing** ⚠️.
- Popover: Return on "Add input…" opens it and focus lands in the search field (needs `init => { self.focus(); }` inside the popup; you cannot focus a popup child from outside, which is a compile error). Typing filters the list. Tab cycles entry → list → Close → entry, so **focus is trapped** ✅. **Esc closes it and focus returns to the opener** ✅.
- Focus visible (`screenshots/a11y/focus-zoom.png`): the switch has a dark 2 px ring ✅. **The slider focus is only a slightly lighter thumb, effectively invisible** ❌ (WCAG 2.4.7). Buttons and list items show a ring or highlight.

**What Orca said** (`a11y/orca-spoken-en.txt`): "JUNC frame." → "New junction push button." → "Add input… push button." → "Firefox panel. Enable Firefox toggle button pressed." → Space: "not pressed" → "Volume of Firefox horizontal slider 80." → Right: "81" → … → Return: "Add input panel. entry Search streams and devices… List with 6 items Firefox." → Esc: "Add input… push button." Gap: the first Tab (onto the sidebar list item) produced no speech.

**Recommendations**
1. Wrap `Switch` in a JUNC component that sets `accessible-label` and `accessible-description` ("on/off"), until AccessKit/AT-SPI get a switch role.
2. Add your own focus indicator via `has-focus` on slider and card. Don't rely on the Fluent default.
3. Give every `LineEdit` an `accessible-label`. Make meters numeric `accessible-value`s, or `accessible-role: none` if they are too chatty.
4. Expose the picker as a dialog: give the popup root `accessible-role` and label, and file an upstream issue for a dialog/window role on `PopupWindow`.
5. Add Page Up/Down (±10) on sliders with a `FocusScope` key handler.
6. **Must do before committing:** rerun `orca_test.sh` and `interaction_test.sh` on Omarchy/Hyprland with a real session (input injection there via `wtype`/`ydotool`), and listen to Orca.

## 3. Slint i18n, RTL and complex scripts — FAIL (RTL mirroring), other items mostly PASS

**Setup:** `@tr()` strings extracted with `slint-tr-extractor` (contexts `Card` / `MainWindow`). Test catalogues for **ar he fa ur hi bn ta si th ja zh_CN ko ru de fi**, plus `en` and pseudo-locale `en_XA` (+40 % length, accented, bracketed) are in `i18n/make_catalogs.py` → `locale/<lang>/LC_MESSAGES/junc-proto.mo`. They are **test strings written for rendering checks, not reviewed by native speakers.** Screenshots: `screenshots/i18n/<lang>.png` (1100 px wide) and `screenshots/i18n-narrow/*-760.png`. Default font: JetBrains Mono. Noto, Noto CJK and Noto Extra are installed.

| Item | Result | Evidence |
|---|---|---|
| gettext + `@tr` | ✅ `slint::init_translations!` + `LANGUAGE` + `slint::update_all_translations()`. The process locale must not be `C` or gettext ignores `LANGUAGE`. | `src/main.rs` |
| Live switch without restart, watching `~/.config/omarchy/language` | ✅ inotify (no polling). en→ar→ru→he→en in the same PID; visible text **and AT-SPI names** update within 0.6 s; RTL flag flips live. Assumption: the file holds a bare code (`ar`, `zh-CN`); the Omarchy file format was not verified. | `i18n/live-switch.txt`, `screenshots/live-switch/` |
| Full RTL mirroring | ❌ **Slint 1.19 has no layout-direction / RTL mirroring** (checked: no such property in the compiler's builtins; FlexboxLayout has only row/column (+reverse)). I mirrored by hand (duplicated `if rtl` branches, swapped column order, right alignment). The visuals look right (`screenshots/i18n/ar.png`), but: **Tab order no longer matches reading order** (RTL Tab goes Add input → Listen → Bus → Inputs → sidebar, i.e. left→right, `a11y/tab-order-ar.txt`); **Slider fills from the left and Right arrow still increases** (75→76) ❌; **Switch knob is not mirrored** ❌; every `HorizontalLayout` needs a hand-written mirror. | screenshots, `a11y/tab-order-ar.txt` |
| Bidi inside text | ✅ Arabic/Hebrew/Persian/Urdu lines render RTL with correct punctuation, numbers and ellipsis placement. | `screenshots/i18n/{ar,he,fa,ur}.png` |
| Arabic joining, Persian ZWNJ | ✅ | ar/fa screenshots |
| Devanagari conjuncts (क्ष द्वि स्त्री श्रृं), Bengali (ক্ষ স্ত্রী ক্ত), Sinhala ශ්‍රී (ZWJ), Tamil | ✅ correct shapes | `screenshots/i18n/{hi,bn,si,ta}.png` |
| Thai line breaking | ✅ at 760 px the Thai sentence wraps at a word boundary (…ทั้งหมดที่ / เลือกไว้…), tone marks fine | `screenshots/i18n-narrow/th-760.png` |
| Font fallback JetBrains Mono → Noto / Noto CJK | ✅ every script rendered, no tofu. ⚠️ **Urdu falls back to a Naskh-style face, not Nastaliq**, even with Noto Nastaliq Urdu installed (fallback is chosen per script, not per language). JA vs SC Han glyph-variant selection **not verified**. | screenshots |
| Plurals | ✅ Arabic: all six gettext forms selected correctly for 0,1,2,3–10,11–99,100–102. Russian: one/few/many correct (1, 21, 101 / 2–4, 22, 102 / 0, 5–20, 25, 111). CLDR `other` (fractions) can't be expressed with integer gettext `n`, which doesn't matter for counts. | `i18n/plural-dump.txt` |
| Persian digits | ❌ `{n}` is always ASCII ("3 ورودی متصل"). Omarchy's `locales.json` sets `zeroDigit` ۰ for `fa` (decimal ٫) and ০ for `bn`. Digits a translator types in the string render fine (۱۲۳). | `i18n/plural-dump.txt`, `i18n/omarchy-locales.json` |
| Pseudo-locale +40 % | ✅ no clipping at 1100 px or 760 px. Buttons grow, the sample text wraps, card titles have `overflow: elide`. Not tested below 760 px or with long device names. | `screenshots/i18n/en_XA.png`, `screenshots/i18n-narrow/en_XA-760.png` |

Omarchy registry check: 50 locales; RTL = ar, ur, fa, he (matches the brief). Codes use hyphens (`zh-CN`, `pt-BR`, `es-MX`), so normalise them to gettext's `zh_CN` (the prototype does).

**Recommendations**
1. **Decide RTL now.** Options: (a) build a small set of direction-aware layout components (`JRow`, `JSlider`, `JSwitch`) that mirror visuals **and** key handling, and set focus order explicitly; (b) contribute or wait for upstream layout-direction support in Slint. Budget is roughly 1–2 weeks for (a). Without either, ar/he/fa/ur users get mirrored chrome with LTR controls.
2. Format numbers in Rust (`icu_decimal` with the locale's numbering system) and pass them as `{0}`. Keep `{n}` only for choosing the plural form. Test that Slint accepts `@tr("{0} input" | "{0} inputs" % n, formatted)` (**not tested here**).
3. For Urdu, set `default-font-family` per language (e.g. "Noto Nastaliq Urdu") when the language is `ur`. Do the same for `ja` vs `zh-*` vs `ko` Han fonts.
4. Keep `en_XA` in CI and screenshot every PR at the minimum window width.

## 4. Performance baseline — RISK

**Graph (same for both builds):** `junc.bus` (null-audio-sink, 4 ch FL FR RL RR) ← 3 app streams (`pw-cat`, endless silence, latency 1024) + `fake.hwin` (mono "hardware" input → FL+FR); bus monitor → 2 listen outputs → `fake.hp`, `fake.obs`. Quantum 1024 @ 48 kHz, driver = `fake.hwin`.
- **manual** (`bench/graph-manual.sh`): `pw-cli create-node adapter` (linger), `pw-link`, 2× `pw-loopback`; apps carry `target.object=junc.bus` and WirePlumber links them.
- **juncd** (`bench/juncd`, pipewire-rs 0.10.1): creates the bus server-side via the `adapter` factory, links apps and hw input itself (`link-factory`), and reconnects when PipeWire restarts. Two listen variants:
  - `juncd`: loads `libpipewire-module-loopback` **inside the daemon** (same as pw-loopback)
  - `juncd-links`: links bus monitor ports straight to the outputs, so the daemon has no audio thread
- Independent observer `pwobs` (also pipewire-rs) timestamps node and link globals, so re-link and recovery times are measured the same way for every mode.
- Harness `bench/run-bench.sh <mode>`, raw output in `bench/results/<mode>/`.

**Results** (VM, 2 vCPU, PipeWire 1.0.5 in a container, null-sink devices)

| Metric | Target | manual | juncd (in-process loopback) | juncd-links |
|---|---|---|---|---|
| Extra latency vs direct device path | 0 quanta | **0** frames | **0** frames | **0** frames |
| pw-top, 600 × 1 s: ERR delta on bus, loopback and device nodes | 0 | **0** | **0** | **0** |
| pw-top ERR delta on the 3 fake-app clients | — | 0 | 17 (5–6 each) | 12 (4 each, at the same 2 moments) |
| Helper PSS, idle (manual = 2 pw-loopback processes) | < 10 MB | 6.0 MB | 4.9 MB | **1.7 MB** |
| Daemon PSS after 1 h with an app restart every minute | < 10 MB | — (5 min churn: 6.0 → 6.0 MB) | **4.9 → 5.0 MB** (60 restarts, +84 kB); 6.7 MB after 5 more PipeWire restarts (loopback modules re-loaded per reconnect, likely not freed) | — |
| Wakeups/s, graph running, no topology changes | ~0 | 190 (pw-loopback data threads) | 140 (data loop thread) | **0.00** |
| App appears → link created (pwobs) | < 100 ms | n=7 p50 8.4, max 19.4 ms (WirePlumber) | **n=63 p50 3.8, p95 8.6, max 14.9 ms** | n=3 p50 5.1, max 7.4 ms |
| Build full graph from nothing | — | 141 ms | 77 ms | 39 ms |
| Recovery: restart PipeWire → bus + hw input + both outputs linked again | < 1 s | **doesn't recover by itself**. Re-running the script: NA (first try lost the hw-input links to a pw-link race), then 427–472 ms | 427–571 ms | 392–545 ms; **368–408 ms** after the reconnect fix (n=10) |

Notes on method and caveats:
- **Latency.** `jack_iodelay` (via `pw-jack`) gives 1024 frames on the direct device path (one quantum, because it is a feedback loop). Through the junction it never locked ("Signal below threshold"), even though audio provably flows (a `pw-record` capture of `fake.hp` shows the bursts). Cause: a single JACK client that both feeds the bus and reads the far end forms a graph cycle through `pw-loopback`, which PipeWire schedules differently. `bench/impulse_latency.py` uses separate TX and RX JACK clients on the graph clock and measures **0 frames for both the direct and the junction path**, so 0 extra quanta. The absolute device latency is the same with or without JUNC. **Hardware round-trip latency was not measured (no sound card).**
- **xruns.** Junction and device nodes stayed at 0 in every mode. In the `juncd-links` run the three `pw-cat` app clients each logged +4 ERR at the same two moments (~6 and ~7.3 min). The `juncd` run had 17 (5–6 per app); the manual run had 0. To check whether juncd causes them, `bench/ab-applink.sh` ran both linkings **at the same time** on two separate PipeWire instances for 10 min: apps linked by juncd = **48** ERR, apps linked by WirePlumber (`target.object`) = **46** ERR, bus 0 in both (`bench/results/ab-applink/`). So the client ERRs do not depend on who creates the links. They are the non-RT `pw-cat` clients missing deadlines on this busy 2-vCPU VM; the manual run's 0 was a quieter window. Rerun on the target with rtkit.
- **Wakeups.** `powertop` is meaningless in a VM, so I counted voluntary + involuntary context switches of all daemon threads instead. In `juncd-links` the main thread logged 88 switches total and the data loop 1, all during setup. Hosting loopbacks in-process adds a data thread that wakes about twice per cycle per loopback (47 cycles/s at 1024/48 kHz): 140/s for juncd, 190/s for the two pw-loopback processes.
- **Recovery** runs from the start of the restart command (which includes a 0.3 s `sleep` in `pw-session.sh` and PipeWire start-up) to the observer seeing the core graph complete. In juncd's own log, connect → graph complete takes 85–150 ms. Bug found and fixed: juncd never reset its reconnect backoff, so every reconnect after the first waited up to 500 ms. A reset plus exponential backoff was worse (630–675 ms, because the steps add up past PipeWire's ~0.3 s start-up). The final version polls every 20 ms for 2 s, then every 500 ms. Re-measured in `bench/results-reconnect-fix/`: **368–408 ms over 10 restarts** (≈ 300 ms of it is the restart itself, ≈ 100 ms juncd reconnect → links done), 0 wakeups/s idle. An inotify watch on `$XDG_RUNTIME_DIR/pipewire-0` would remove the polling entirely.
- The 1 h churn ran 60 restarts and 63 re-links. A harmless `apps.sh` syntax error in `churn.log` after the last restart comes from editing that script on disk while the run was using it. The committed script is fine.
- No systemd user session in the container, so "restart" means kill + respawn `pipewire`, `wireplumber` and `pipewire-pulse` (`pw-session.sh`). On the box: `PW_SESSION=systemd bench/run-bench.sh …` uses `systemctl --user restart pipewire wireplumber`.

**Slint prototype** (`proto/perf.sh`, `proto/perf-*.txt`, `proto/cold-start.txt`)

| Metric | software renderer | femtovg (GL on Mesa llvmpipe, i.e. CPU-rasterised GL) |
|---|---|---|
| Cold start (exec → first frame) | 51 ms to the first event-loop turn (the software renderer has no rendering notifier) | **219 ms** median (181–247) to the first presented frame |
| PSS, meters on | **24 MB** | 142 MB (llvmpipe + GL stack; not representative of a GPU) |
| CPU, 4+1+2 meters at 7 Hz, visible | **3.3 %** of one core | 21 % (software GL) |
| CPU, same, window hidden (unmapped) | **3.2 %**: no saving | 21.7 %: no saving |
| CPU idle, no meters | 0.00 % | 0.02 % |

The meter timer keeps updating properties while the window is unmapped. JUNC must pause meter subscriptions (and the PipeWire peak streams behind them) when hidden. Slint gives no visibility event on X11; on Wayland use the frame callback or occlusion. GPU numbers on real hardware: **not verified**.

**Recommendations**
1. **Architecture:** keep `juncd` out of the RT path. Create the bus (and per-output volume nodes if needed) **server-side** (`adapter` factory with `object.linger`, or a `pipewire.conf.d` drop-in written by juncd), and connect them with links. Don't load `module-loopback` in the daemon: in this VM that is the difference between 0 and ~130 wakeups/s and between 1.7 and ~5 MB.
2. Keep the observer-based re-link path (registry global → link-factory). Re-link was 1.4–19 ms, well under 100 ms. Look into the PSS growth on reconnect in loopback mode (5.0 → 6.7 MB after 5 restarts); it is likely the loopback modules of old contexts not being unloaded.
3. The manual graph does not survive a PipeWire restart, and the script raced once. juncd recovers on its own. Keep backoff short and reset it after each successful connection (fixed).
4. GUI: pause meters when hidden or minimised. Measure GPU-backed femtovg or skia on the Omarchy box; the 24 MB / 3 % software-renderer numbers are the upper bound to beat.
5. Rerun `bench/run-bench.sh` on the Omarchy box with real hardware, `PW_SESSION=systemd`, quanta 256 and 1024, plus `powertop --csv` for true wakeups. CI can run the same harness headless (it brings its own PipeWire config in `bench/ci-config/`).

---

### Reproduce
See `README.md`. Summary: build `proto` and `bench/juncd` with cargo, run `python3 proto/i18n/make_catalogs.py`, then the scripts in `proto/a11y`, `proto/i18n` and `proto/perf.sh`, then `bench/run-bench.sh {manual,juncd,juncd-links}` with the `STABLE_SEC`, `CHURN_SEC` and `RECOVERY_N` env knobs.
