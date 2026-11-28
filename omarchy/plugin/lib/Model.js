.pragma library
// SPDX-License-Identifier: GPL-3.0-or-later

// What the bar module knows, built from `junc --json watch` lines
// (docs/architecture.md §5.2). Pure functions on plain objects, so the
// tests run them in Node; Service.qml keeps the state and the views read
// it through the functions below.
//
// A state is { running, capture: [app], junctions: [junction] }, with each
// junction { id, name, kind, enabled, status, default, inputs } in the order
// juncd listed them (new ones last).

// Lines longer than this are not JUNC's; they are dropped unread.
var MAX_LINE = 65536
// A name longer than this is cut (juncd allows 64 characters).
var MAX_NAME = 128
// At most this many Junctions or captured apps are kept.
var MAX_ITEMS = 256

function empty() {
  return { running: false, capture: [], junctions: [] }
}

function text(value, max) {
  return typeof value === "string" ? value.slice(0, max) : ""
}

function count(value) {
  return typeof value === "number" && isFinite(value) && value >= 0 ? Math.floor(value) : 0
}

function role(value) {
  return value === "input" || value === "output" ? value : "none"
}

// The state after one line of the stream. A line that is not a JUNC event
// leaves the state as it was.
function apply(state, line) {
  if (typeof line !== "string" || line.length === 0 || line.length > MAX_LINE) return state
  var event
  try {
    event = JSON.parse(line)
  } catch (e) {
    return state
  }
  if (!event || typeof event !== "object" || Array.isArray(event)) return state

  if (event.event === "capture" && Array.isArray(event.apps)) {
    var apps = []
    for (var i = 0; i < event.apps.length && apps.length < MAX_ITEMS; i++) {
      var app = text(event.apps[i], MAX_NAME)
      if (app) apps.push(app)
    }
    return { running: true, capture: apps, junctions: state.junctions }
  }

  if (event.event === "daemon") {
    return { running: event.running === true, capture: event.running === true ? state.capture : [], junctions: state.junctions }
  }

  if (event.event === "junction") {
    var id = text(event.id, MAX_NAME)
    if (!id) return state
    var list = []
    var found = false
    for (var k = 0; k < state.junctions.length; k++) {
      var j = state.junctions[k]
      if (j.id !== id) { list.push(j); continue }
      found = true
      if (event.removed === true) continue
      list.push(junction(id, event))
    }
    if (!found && event.removed !== true && list.length < MAX_ITEMS) list.push(junction(id, event))
    return { running: true, capture: state.capture, junctions: list }
  }

  return state
}

function junction(id, event) {
  return {
    id: id,
    name: text(event.name, MAX_NAME) || id,
    kind: event.kind === "cable" ? "cable" : "junction",
    enabled: event.enabled === true,
    status: text(event.status, 32),
    default: role(event.default),
    inputs: count(event.inputs),
  }
}

// How many Junctions and Cables are on: the number on the bar.
function onCount(state) {
  var n = 0
  for (var i = 0; i < state.junctions.length; i++) if (state.junctions[i].enabled) n++
  return n
}

// Whether any app's audio goes into an enabled Junction now (the dot).
function capturing(state) {
  return state.running && state.capture.length > 0
}

// The junctions as the popup lists them, with the switch already thrown for
// the ones the user just flipped (pending: id -> wanted state).
function rows(state, pending) {
  var out = []
  for (var i = 0; i < state.junctions.length; i++) {
    var j = state.junctions[i]
    var wanted = pending && Object.prototype.hasOwnProperty.call(pending, j.id) ? pending[j.id] : j.enabled
    out.push({ id: j.id, name: j.name, kind: j.kind, enabled: wanted, busy: wanted !== j.enabled, default: j.default, inputs: j.inputs })
  }
  return out
}

// The pending switches still waiting for juncd: a switch is settled once
// the stream reports the state the user asked for.
function settle(state, pending) {
  var out = ({})
  for (var id in pending) {
    for (var i = 0; i < state.junctions.length; i++) {
      var j = state.junctions[i]
      if (j.id === id && j.enabled !== pending[id]) out[id] = pending[id]
    }
  }
  return out
}

// `junc enable ID` or `junc disable ID`, as an argv.
function switchCommand(id, on) {
  return ["junc", on ? "enable" : "disable", "--", id]
}

// Hyprland modifier bits (hyprctl binds -j) in the order Omarchy writes them.
var MODS = [[64, "Super"], [4, "Ctrl"], [8, "Alt"], [1, "Shift"]]

// The shortcut bound to junc-gui, written like Super+Shift+J, or "" when
// none is. `binds` is the parsed output of `hyprctl binds -j`.
function shortcut(binds) {
  if (!Array.isArray(binds)) return ""
  for (var i = 0; i < binds.length; i++) {
    var b = binds[i]
    if (!b || typeof b !== "object") continue
    if (typeof b.arg !== "string" || !/(^|[\s\/])junc-gui(\s|$)/.test(b.arg)) continue
    if (typeof b.key !== "string" || b.key.length === 0 || b.key.length > 32) continue
    var parts = []
    var mask = count(b.modmask)
    for (var m = 0; m < MODS.length; m++) if (mask & MODS[m][0]) parts.push(MODS[m][1])
    parts.push(b.key.length === 1 ? b.key.toUpperCase() : b.key)
    return parts.join("+")
  }
  return ""
}
