// Pure helpers for Panel.qml: reading `x7` CLI output and building the
// commands that set a value. Kept free of QML so the tests can run it.

// The groups read on open, in order. Each is printed by `x7 <group>` as
// "name value" lines; `output` prints only the value.
var groups = ["output", "scout", "sbx", "eq", "headphones", "speakers", "voice"]

// The shell script the panel runs to read everything in one process. It
// stops at the first failure, which is how "not connected" shows.
function readScript(x7) {
  var q = "'" + String(x7).replace(/'/g, "'\\''") + "'"
  var parts = []
  for (var i = 0; i < groups.length; i++) {
    parts.push("echo @" + groups[i] + " && " + q + " " + groups[i])
  }
  parts.push("echo @eq && " + q + " eq preset")
  return parts.join(" && ")
}

// "@group" lines switch the group; "name value" lines fill it. A line with
// one word is the group's own value (`x7 output` prints "headphones").
function parse(text) {
  var state = {}
  var group = ""
  var lines = String(text || "").split("\n")
  for (var i = 0; i < lines.length; i++) {
    var line = lines[i].trim()
    if (line === "") continue
    if (line.charAt(0) === "@") {
      group = line.slice(1)
      if (!state[group]) state[group] = {}
      continue
    }
    if (group === "") continue
    var space = line.indexOf(" ")
    if (space === -1) state[group][group] = line
    else state[group][line.slice(0, space)] = line.slice(space + 1).trim()
  }
  return state
}

// The master switches are set as `x7 <group> on|off`, like `x7 output
// speakers` and `x7 scout on`; everything else as `x7 <group> <name> <value>`.
function command(x7, group, name, value) {
  if (group === name) return [x7, group, String(value)]
  return [x7, group, name, String(value)]
}

// The tray tooltip reads "X7: headphones"; used for the bar icon before the
// panel has read the device.
function outputFromTooltip(text) {
  var m = /:\s*(headphones|speakers)\b/i.exec(String(text || ""))
  return m ? m[1].toLowerCase() : ""
}

function number(value, fallback) {
  var n = parseFloat(value)
  return isNaN(n) ? fallback : n
}

// Readouts in the GUI's units.
function readout(unit, value) {
  var n = number(value, 0)
  if (unit === "percent") return Math.round(n * 100) + " %"
  if (unit === "hz") return Math.round(n) + " Hz"
  if (unit === "deg") return Math.round(n) + "°"
  if (unit === "db") {
    var r = Math.round(n * 10) / 10
    return (r > 0 ? "+" : "") + r
  }
  return String(value)
}

// Slider values go to the device rounded the way the GUI shows them.
function format(unit, value) {
  if (unit === "percent") return (Math.round(value * 100) / 100).toFixed(2)
  if (unit === "db") return (Math.round(value * 2) / 2).toFixed(1)
  return String(Math.round(value))
}

if (typeof module !== "undefined") {
  module.exports = {
    groups: groups,
    readScript: readScript,
    parse: parse,
    command: command,
    outputFromTooltip: outputFromTooltip,
    number: number,
    readout: readout,
    format: format
  }
}
