// Tests for the Omarchy widget's Model.js. usage: node dist/omarchy/model-test.js
"use strict"
const assert = require("assert")
const path = require("path")
const M = require(path.join(__dirname, "io.github.stamrin.x7", "Model.js"))

// What `sh -c readScript` prints against a device.
const sample = [
  "@output", "headphones",
  "@scout", "scout off",
  "@sbx", "sbx on", "surround on", "surround-level 0.67", "smart-volume-mode normal", "bass-crossover 80.00",
  "@eq", "eq on", "preamp 0.00", "31hz -2.00", "1khz 4.00",
  "@headphones", "gain normal",
  "@speakers", "layout 2.0", "output-target amp-out",
  "@voice", "voice off", "focus-angle 30.00",
  "@eq", "preset vocal",
  ""
].join("\n")

const s = M.parse(sample)
assert.strictEqual(s.output.output, "headphones")
assert.strictEqual(s.scout.scout, "off")
assert.strictEqual(s.sbx.sbx, "on")
assert.strictEqual(s.sbx["surround-level"], "0.67")
assert.strictEqual(s.eq["31hz"], "-2.00")
// The second @eq block adds to the first instead of replacing it.
assert.strictEqual(s.eq.preset, "vocal")
assert.strictEqual(s.eq.eq, "on")
assert.strictEqual(s.speakers.layout, "2.0")
assert.deepStrictEqual(M.parse(""), {})
assert.deepStrictEqual(M.parse("stray line\n"), {})

// Master switches and the output take the group's own form.
assert.deepStrictEqual(M.command("/x7", "output", "output", "speakers"), ["/x7", "output", "speakers"])
assert.deepStrictEqual(M.command("/x7", "sbx", "sbx", "off"), ["/x7", "sbx", "off"])
assert.deepStrictEqual(M.command("/x7", "scout", "scout", "on"), ["/x7", "scout", "on"])
assert.deepStrictEqual(M.command("/x7", "eq", "1khz", "-1.5"), ["/x7", "eq", "1khz", "-1.5"])
assert.deepStrictEqual(M.command("/x7", "eq", "preset", "rock"), ["/x7", "eq", "preset", "rock"])

// The read script quotes the CLI path for sh.
const script = M.readScript("/home/o'neil/.local/bin/x7")
assert.ok(script.includes("'/home/o'\\''neil/.local/bin/x7' sbx"), script)
assert.ok(script.startsWith("echo @output && "), script)
assert.ok(script.endsWith(" eq preset"), script)

assert.strictEqual(M.outputFromTooltip("X7: speakers"), "speakers")
assert.strictEqual(M.outputFromTooltip("X7: not connected"), "")
assert.strictEqual(M.outputFromTooltip(undefined), "")

assert.strictEqual(M.readout("percent", "0.67"), "67 %")
assert.strictEqual(M.readout("hz", "80.00"), "80 Hz")
assert.strictEqual(M.readout("deg", "30"), "30\u00b0")
assert.strictEqual(M.readout("db", "4"), "+4")
assert.strictEqual(M.readout("db", "-1.5"), "-1.5")
assert.strictEqual(M.readout("db", "0"), "0")
assert.strictEqual(M.format("percent", 0.6666), "0.67")
assert.strictEqual(M.format("db", -1.26), "-1.5")
assert.strictEqual(M.format("hz", 79.6), "80")

console.log("all Model.js checks passed")
