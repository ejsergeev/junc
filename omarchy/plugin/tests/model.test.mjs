// SPDX-License-Identifier: GPL-3.0-or-later
// The bar module's model: `junc --json watch` lines in, what the bar and the
// popup show out. Run with `node --test omarchy/plugin/tests/`.

import { test } from "node:test"
import assert from "node:assert/strict"
import { load } from "./qmljs.mjs"

const M = load("lib/Model.js")

const feed = (lines, state = M.empty()) => lines.reduce((s, l) => M.apply(s, typeof l === "string" ? l : JSON.stringify(l)), state)

// What junc watch prints first (architecture §5.2, B7 + B23 fields).
const START = [
  { event: "capture", apps: ["Spotify", "Soundux", "Firefox"] },
  { event: "junction", id: "zoom", name: "Virtual Mic for Zoom", kind: "junction", enabled: true, status: "active", default: "input", inputs: 3 },
  { event: "junction", id: "obs", name: "Firefox → OBS", kind: "cable", enabled: true, status: "active", default: "none", inputs: 1 },
  { event: "junction", id: "spare", name: "Spare", kind: "junction", enabled: false, status: "off", default: "none", inputs: 0 },
]

test("the start of the stream fills the bar and the popup (board 44)", () => {
  const s = feed(START)
  assert.equal(s.running, true)
  assert.equal(M.onCount(s), 2)
  assert.equal(M.capturing(s), true)
  assert.deepEqual(s.capture, ["Spotify", "Soundux", "Firefox"])
  assert.deepEqual(M.rows(s, {}).map(r => [r.name, r.kind, r.enabled, r.default, r.inputs]), [
    ["Virtual Mic for Zoom", "junction", true, "input", 3],
    ["Firefox → OBS", "cable", true, "none", 1],
    ["Spare", "junction", false, "none", 0],
  ])
})

test("changes replace a Junction in place; a removed one goes", () => {
  let s = feed(START)
  s = feed([{ event: "junction", id: "zoom", name: "Zoom", kind: "junction", enabled: false, status: "off", default: "none", inputs: 2 }], s)
  assert.deepEqual(s.junctions.map(j => j.id), ["zoom", "obs", "spare"])
  assert.equal(s.junctions[0].name, "Zoom")
  assert.equal(M.onCount(s), 1)
  s = feed([{ event: "junction", id: "obs", removed: true }, { event: "junction", id: "new", name: "New", kind: "junction", enabled: true }], s)
  assert.deepEqual(s.junctions.map(j => j.id), ["zoom", "spare", "new"])
  s = feed([{ event: "capture", apps: [] }], s)
  assert.equal(M.capturing(s), false)
})

test("juncd going away hides the dot and keeps the list until it is back", () => {
  let s = feed([...START, { event: "daemon", running: false }])
  assert.equal(s.running, false)
  assert.equal(M.capturing(s), false)
  assert.equal(s.junctions.length, 3)
  s = feed([{ event: "daemon", running: true }, { event: "capture", apps: ["Zoom"] }], s)
  assert.equal(M.capturing(s), true)
})

test("anything that is not a JUNC event is ignored", () => {
  const s = feed(START)
  for (const line of ["", "not json", "[1,2]", "null", '{"event":"graph","generation":7}', '{"event":"junction"}', "x".repeat(70000)]) {
    assert.equal(M.apply(s, line), s, line.slice(0, 20))
  }
  // Odd fields are made safe, never shown raw.
  const odd = feed([{ event: "junction", id: "a", name: 42, kind: "<b>x</b>", enabled: "yes", default: "both", inputs: -3 }])
  assert.deepEqual(odd.junctions[0], { id: "a", name: "a", kind: "junction", enabled: false, status: "", default: "none", inputs: 0 })
  const long = feed([{ event: "capture", apps: ["y".repeat(500), 7, ""] }])
  assert.deepEqual(long.capture.map(a => a.length), [128])
})

test("a flipped switch shows at once and settles when juncd agrees", () => {
  let s = feed(START)
  let pending = { spare: true }
  assert.deepEqual(M.rows(s, pending).map(r => [r.id, r.enabled, r.busy]), [["zoom", true, false], ["obs", true, false], ["spare", true, true]])
  assert.deepEqual(M.settle(s, pending), { spare: true })
  s = feed([{ event: "junction", id: "spare", name: "Spare", kind: "junction", enabled: true }], s)
  pending = M.settle(s, pending)
  assert.deepEqual(pending, {})
  // A pending switch for a Junction that is gone is dropped.
  assert.deepEqual(M.settle(s, { gone: false }), {})
})

test("the switch runs junc enable or disable, the id after --", () => {
  assert.deepEqual(M.switchCommand("zoom", true), ["junc", "enable", "--", "zoom"])
  assert.deepEqual(M.switchCommand("-x", false), ["junc", "disable", "--", "-x"])
})

test("the shortcut hint shows the key the user bound to junc-gui, else nothing", () => {
  const binds = [
    { modmask: 64, key: "Return", arg: "uwsm-app -- alacritty" },
    { modmask: 65, key: "j", arg: "uwsm-app -- junc-gui" },
  ]
  assert.equal(M.shortcut(binds), "Super+Shift+J")
  assert.equal(M.shortcut([{ modmask: 72, key: "F5", arg: "/usr/bin/junc-gui" }]), "Super+Alt+F5")
  assert.equal(M.shortcut([{ modmask: 64, key: "j", arg: "junc-gui-helper" }]), "")
  assert.equal(M.shortcut([{ modmask: 64, key: "j", arg: "notjunc-gui" }]), "")
  assert.equal(M.shortcut(null), "")
  assert.equal(M.shortcut([]), "")
})
