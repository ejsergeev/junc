// SPDX-License-Identifier: GPL-3.0-or-later
// The plugin folder as Omarchy reads it: the manifest's rules (what
// `omarchy plugin validate` checks, for CI where Omarchy is not installed),
// and catalogs that carry every string the QML asks for.

import { test } from "node:test"
import assert from "node:assert/strict"
import { existsSync, lstatSync, readFileSync, readdirSync } from "node:fs"
import { dirname, join, resolve } from "node:path"
import { fileURLToPath } from "node:url"

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..")
const manifest = JSON.parse(readFileSync(join(root, "manifest.json"), "utf8"))

function files(dir) {
  return readdirSync(dir, { withFileTypes: true }).flatMap(e => {
    const path = join(dir, e.name)
    return e.isDirectory() ? files(path) : [path]
  })
}

test("the manifest follows Omarchy's plugin schema", () => {
  assert.equal(manifest.schemaVersion, 1)
  assert.equal(manifest.id, "io.github.ejsergeev.junc")
  for (const field of ["id", "name", "version", "kinds", "entryPoints"]) assert.ok(field in manifest, field)
  assert.deepEqual(manifest.kinds, ["service", "bar-widget"])
  assert.equal(manifest.entryPoints.service, "Service.qml")
  assert.equal(manifest.entryPoints.barWidget, "BarWidget.qml")
  for (const ep of Object.values(manifest.entryPoints)) {
    assert.ok(!ep.startsWith("/") && !ep.includes(".."), ep)
    assert.ok(existsSync(join(root, ep)), ep)
  }
  assert.equal(manifest.barWidget.defaultSection, "right")
})

test("the folder has no symlinks (Omarchy refuses them)", () => {
  for (const f of files(root)) assert.ok(!lstatSync(f).isSymbolicLink(), f)
})

test("every string the QML asks for is in en.json, and every catalog key is used", () => {
  const en = JSON.parse(readFileSync(join(root, "i18n/en.json"), "utf8"))
  const qml = files(root).filter(f => f.endsWith(".qml")).map(f => readFileSync(f, "utf8")).join("\n")
  const used = new Set([...qml.matchAll(/\b(?:t|tn)\("([a-zA-Z.]+)"/g)].map(m => m[1]))
  for (const key of used) assert.ok(key in en, `missing in en.json: ${key}`)
  for (const key of Object.keys(en)) assert.ok(used.has(key), `unused: ${key}`)
})

test("every catalog has only English keys, the same %N holes and the plural forms", () => {
  const en = JSON.parse(readFileSync(join(root, "i18n/en.json"), "utf8"))
  const holes = s => [...String(s).matchAll(/%\d/g)].map(m => m[0]).sort().join()
  for (const name of readdirSync(join(root, "i18n"))) {
    const cat = JSON.parse(readFileSync(join(root, "i18n", name), "utf8"))
    for (const [key, value] of Object.entries(cat)) {
      assert.ok(key in en, `${name}: ${key} is not in en.json`)
      const english = typeof en[key] === "object" ? en[key].other : en[key]
      const forms = typeof value === "object" ? Object.values(value) : [value]
      assert.equal(typeof value, typeof en[key], `${name}: ${key}`)
      if (typeof value === "object") assert.ok("other" in value, `${name}: ${key} has no other`)
      for (const form of forms) assert.equal(holes(form), holes(english), `${name}: ${key}`)
    }
  }
})

test("names from the config and PipeWire are shown as plain text", () => {
  for (const f of files(root).filter(f => f.endsWith(".qml"))) {
    const text = readFileSync(f, "utf8")
    // Plain Text is never used directly: PlainLabel sets Text.PlainText.
    assert.ok(!/^\s*Text\s*\{/m.test(text) || f.endsWith("PlainLabel.qml"), `${f} uses a bare Text`)
  }
  assert.match(readFileSync(join(root, "components/PlainLabel.qml"), "utf8"), /textFormat: Text\.PlainText/)
})
