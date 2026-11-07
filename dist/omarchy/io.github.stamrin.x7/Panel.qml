import QtQuick
import Quickshell
import Quickshell.Io
import Quickshell.Services.SystemTray
import qs.Commons
// A bare `Color` is QtQuick's own value type on Qt 6.12, not the theme.
import qs.Commons as Commons
import qs.Ui
import "Model.js" as Model

// Sound Blaster X7 in the Omarchy bar. The icon shows the active output;
// a left click opens a popup with the controls, read from and written to
// the device through the `x7` CLI (about 35 ms a call, alongside x7-gui).
// Left or right click toggles it, middle click opens x7-gui's full window,
// Left/Right walk the pages, and `omarchy-shell io.github.stamrin.x7
// showPage eq` opens it on a page.
// Shown while x7-gui runs, as the tray item it stands in for; the installer
// hides that item in omarchy.tray so the icon does not show twice.
Panel {
  id: root
  moduleName: "io.github.stamrin.x7"
  ipcTarget: "io.github.stamrin.x7"
  // This panel owns its IPC handler, to add showPage to the usual methods.
  manageIpc: false

  readonly property var theme: Commons.ShellColor || Commons.Color
  readonly property color fg: bar ? bar.foreground : theme.foreground
  readonly property string fontName: bar ? bar.fontFamily : Style.font.family
  // The installer puts the CLI here; the shell's own PATH may not have it.
  readonly property string x7: Quickshell.env("HOME") + "/.local/bin/x7"

  property var device: ({})
  property bool loaded: false
  property bool connected: true
  property string page: "sbx"
  readonly property var pages: ["sbx", "eq", "headphones", "speakers", "voice"]
  property var queue: []
  property var profiles: []

  readonly property var trayItem: {
    var values = SystemTray.items.values
    for (var i = 0; i < values.length; i++) {
      if (String(values[i].id || "") === "x7") return values[i]
    }
    return null
  }
  readonly property string output: {
    var read = value("output", "output")
    if (read) return read
    return trayItem ? Model.outputFromTooltip(trayItem.tooltipTitle) : ""
  }

  function value(group, name) {
    var g = device[group]
    return g && g[name] !== undefined ? g[name] : ""
  }
  function isOn(group, name) { return value(group, name) === "on" }
  function num(group, name, fallback) { return Model.number(value(group, name), fallback) }

  function refresh() {
    if (!readProc.running) readProc.running = true
    if (!profileProc.running) profileProc.running = true
  }

  // Shown at once, written in order, read back when the queue drains.
  function set(group, name, v) {
    var next = Object.assign({}, device)
    next[group] = Object.assign({}, next[group] || {})
    next[group][name] = String(v)
    device = next
    queue = queue.concat([Model.command(x7, group, name, v)])
    pump()
  }
  function toggle(group, name) { set(group, name, isOn(group, name) ? "off" : "on") }
  function pump() {
    if (writeProc.running || queue.length === 0) return
    writeProc.command = queue[0]
    queue = queue.slice(1)
    writeProc.running = true
  }
  function loadProfile(name) {
    if (!name) return
    queue = queue.concat([[x7, "profile", "load", name]])
    pump()
  }

  function showPage(name) {
    if (pages.indexOf(name) !== -1) page = name
    open()
  }
  function stepPage(delta) {
    var i = pages.indexOf(page)
    page = pages[(i + delta + pages.length) % pages.length]
  }

  ShellIpc {
    target: "io.github.stamrin.x7"
    function open(): void { root.open() }
    function close(): void { root.close() }
    function show(): void { root.open() }
    function hide(): void { root.close() }
    function toggle(): void { root.toggle() }
    function showPage(name: string): void { root.showPage(name) }
  }

  onOpenedChanged: if (opened) refresh()

  visible: trayItem !== null
  implicitWidth: visible ? button.implicitWidth : 0
  implicitHeight: visible ? button.implicitHeight : 0

  Process {
    id: readProc
    command: ["sh", "-c", Model.readScript(root.x7)]
    stdout: StdioCollector {
      waitForEnd: true
      onStreamFinished: {
        var parsed = Model.parse(text)
        if (parsed.output && parsed.output.output) root.device = parsed
      }
    }
    onExited: function(code) {
      root.connected = code === 0
      root.loaded = true
    }
  }

  Process {
    id: writeProc
    onExited: function(code) {
      if (code !== 0) root.connected = false
      if (root.queue.length > 0) root.pump()
      else root.refresh()
    }
  }

  Process {
    id: profileProc
    command: ["sh", "-c", "cd \"$HOME/.config/x7/profiles\" 2>/dev/null && for f in *.toml; do [ -f \"$f\" ] && printf '%s\\n' \"${f%.toml}\"; done"]
    stdout: StdioCollector {
      waitForEnd: true
      onStreamFinished: root.profiles = text.split("\n").filter(function(l) { return l !== "" })
    }
  }

  BarIconButton {
    id: button
    anchors.fill: parent
    bar: root.bar
    text: root.output === "speakers" ? String.fromCodePoint(0xf04c3) : String.fromCodePoint(0xf02cb)
    tooltipText: root.opened ? "" : "Sound Blaster X7" + (root.output ? ": " + root.output : "")
    // Middle click opens x7-gui's full window. There is no right-click
    // menu: the shell cannot show a tray item's platform menu, and the
    // popup has everything that menu has.
    onPressed: function(b) {
      if (b === Qt.MiddleButton && root.trayItem) root.trayItem.activate()
      else root.toggle()
    }
  }

  KeyboardPanel {
    id: panel
    anchorItem: button
    owner: root
    bar: root.bar
    open: root.opened
    focusTarget: panel.loadedContent
    contentWidth: panel.fittedContentWidth(Style.space(400))
    contentHeight: panel.fittedContentHeight(panel.loadedContentHeight)

    content: PanelKeyCatcher {
      anchors.fill: parent
      implicitHeight: column.implicitHeight
      onCloseRequested: root.close()
      onMoveRequested: function(dx, dy) { if (dx !== 0) root.stepPage(dx) }
      onTabRequested: function(direction) { root.switchPanel(direction) }

      Column {
        id: column
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        spacing: Style.space(12)

        // ---------- Hero ----------
        Item {
          width: parent.width
          implicitHeight: Math.max(heroIcon.implicitHeight, heroLabels.implicitHeight)

          Text {
            id: heroIcon
            textFormat: Text.PlainText
            text: button.text
            color: root.fg
            font.family: root.fontName
            font.pixelSize: Style.font.display
            anchors.left: parent.left
            anchors.verticalCenter: parent.verticalCenter
          }

          Column {
            id: heroLabels
            anchors.left: heroIcon.right
            anchors.leftMargin: Style.space(14)
            anchors.right: parent.right
            anchors.verticalCenter: parent.verticalCenter
            spacing: Style.space(2)

            Text {
              textFormat: Text.PlainText
              text: "Sound Blaster X7"
              color: root.fg
              font.family: root.fontName
              font.pixelSize: Style.font.title
              font.bold: true
              elide: Text.ElideRight
              width: parent.width
            }
            Text {
              textFormat: Text.PlainText
              text: (!root.connected ? "not connected"
                : !root.loaded ? "reading"
                : root.output + " · sbx " + (root.isOn("sbx", "sbx") ? "on" : "off")
                  + (root.isOn("scout", "scout") ? " · scout" : "")).toUpperCase()
              color: Qt.darker(root.fg, 1.4)
              font.family: root.fontName
              font.pixelSize: Style.font.caption
              font.bold: true
              font.letterSpacing: 1.2
              elide: Text.ElideRight
              width: parent.width
            }
          }
        }

        ButtonGroup {
          width: parent.width
          enabled: root.connected
          options: [{ value: "headphones", label: "Headphones" }, { value: "speakers", label: "Speakers" }]
          value: root.output
          foreground: root.fg
          fontFamily: root.fontName
          fontSize: Style.font.bodySmall
          onChanged: function(v) { root.set("output", "output", v) }
        }

        PanelSeparator { foreground: root.fg }

        ButtonGroup {
          width: parent.width
          options: [
            { value: "sbx", label: "SBX" },
            { value: "eq", label: "EQ" },
            { value: "headphones", label: "Phones" },
            { value: "speakers", label: "Speakers" },
            { value: "voice", label: "Voice" }
          ]
          value: root.page
          foreground: root.fg
          fontFamily: root.fontName
          fontSize: Style.font.caption
          onChanged: function(v) { root.page = v }
        }

        // ---------- SBX ----------
        Column {
          visible: root.page === "sbx"
          width: parent.width
          spacing: Style.space(8)
          enabled: root.connected

          SwitchRow { label: "SBX Pro Studio"; group: "sbx"; name: "sbx"; strong: true }
          Column {
            width: parent.width
            spacing: Style.space(8)
            opacity: root.isOn("sbx", "sbx") ? 1 : 0.45
            EffectRow { label: "Surround"; name: "surround" }
            EffectRow { label: "Dialog Plus"; name: "dialog-plus" }
            EffectRow { label: "Smart Volume"; name: "smart-volume" }
            ChoiceRow {
              label: "Mode"; group: "sbx"; name: "smart-volume-mode"
              options: [{ value: "normal", label: "Normal" }, { value: "loud", label: "Loud" }, { value: "night", label: "Night" }]
            }
            EffectRow { label: "Crystalizer"; name: "crystalizer" }
            EffectRow { label: "Bass"; name: "bass" }
            SliderRow { label: "Crossover"; group: "sbx"; name: "bass-crossover"; minimum: 10; maximum: 500; step: 10; unit: "hz" }
          }
          PanelSeparator { foreground: root.fg }
          SwitchRow { label: "Scout Mode"; group: "scout"; name: "scout" }
        }

        // ---------- Equalizer ----------
        Column {
          visible: root.page === "eq"
          width: parent.width
          spacing: Style.space(10)
          enabled: root.connected

          SwitchRow { label: "Equalizer"; group: "eq"; name: "eq"; strong: true }
          Dropdown {
            width: parent.width
            label: "Preset"
            value: root.value("eq", "preset")
            options: ["acoustic", "classical", "country", "dance", "flat", "jazz", "new-age", "pop", "rock", "vocal"]
            foreground: root.fg
            fontFamily: root.fontName
            onChanged: function(v) {
              root.set("eq", "preset", v)
            }
          }
          Row {
            id: bands
            width: parent.width
            height: Style.space(150)
            opacity: root.isOn("eq", "eq") ? 1 : 0.45
            readonly property var names: ["preamp", "31hz", "62hz", "125hz", "250hz", "500hz", "1khz", "2khz", "4khz", "8khz", "16khz"]
            readonly property var labels: ["Pre", "31", "62", "125", "250", "500", "1k", "2k", "4k", "8k", "16k"]
            Repeater {
              model: bands.names.length
              Band {
                required property int index
                width: bands.width / bands.names.length
                height: bands.height
                name: bands.names[index]
                label: bands.labels[index]
                limit: index === 0 ? 6 : 12
              }
            }
          }
        }

        // ---------- Headphones ----------
        Column {
          visible: root.page === "headphones"
          width: parent.width
          spacing: Style.space(8)
          enabled: root.connected

          ChoiceRow {
            label: "Gain"; group: "headphones"; name: "gain"
            options: [{ value: "normal", label: "Normal, 32-300 Ω" }, { value: "high", label: "High, 600 Ω" }]
          }
          SwitchRow { label: "Direct Mode"; group: "headphones"; name: "direct-mode" }
          SwitchRow { label: "Direct Mode for S/PDIF in"; group: "headphones"; name: "direct-mode-spdif" }
          SwitchRow { label: "High Power Amplification"; group: "headphones"; name: "high-power-amp" }
        }

        // ---------- Speakers ----------
        Column {
          visible: root.page === "speakers"
          width: parent.width
          spacing: Style.space(8)
          enabled: root.connected

          ChoiceRow {
            label: "Layout"; group: "speakers"; name: "layout"
            options: ["2.0", "4.0", "5.0", "5.1"]
          }
          SwitchRow { label: "Front full-range"; group: "speakers"; name: "front-full-range" }
          SwitchRow { label: "Rear full-range"; group: "speakers"; name: "rear-full-range" }
          SwitchRow { label: "Bass redirection"; group: "speakers"; name: "bass-redirection" }
          SliderRow { label: "Crossover"; group: "speakers"; name: "crossover"; minimum: 10; maximum: 500; step: 10; unit: "hz" }
          SwitchRow { label: "Subwoofer gain"; group: "speakers"; name: "subwoofer-gain" }
          ChoiceRow {
            label: "Output"; group: "speakers"; name: "output-target"
            options: [{ value: "line-out", label: "Line" }, { value: "amp-out", label: "Amp" }, { value: "line-and-amp", label: "Both" }]
          }
          SwitchRow { label: "Auto standby"; group: "speakers"; name: "auto-standby" }
        }

        // ---------- CrystalVoice ----------
        Column {
          visible: root.page === "voice"
          width: parent.width
          spacing: Style.space(8)
          enabled: root.connected

          SwitchRow { label: "CrystalVoice"; group: "voice"; name: "voice"; strong: true }
          Column {
            width: parent.width
            spacing: Style.space(8)
            opacity: root.isOn("voice", "voice") ? 1 : 0.45
            SwitchRow { label: "Echo cancellation"; group: "voice"; name: "echo-cancellation" }
            SwitchRow { label: "Noise reduction"; group: "voice"; name: "noise-reduction" }
            SwitchRow { label: "Focus"; group: "voice"; name: "focus" }
            SliderRow { label: "Angle"; group: "voice"; name: "focus-angle"; minimum: 20; maximum: 180; step: 5; unit: "deg" }
            SwitchRow { label: "Smart Volume"; group: "voice"; name: "smart-volume" }
            SliderRow { label: "Level"; group: "voice"; name: "smart-volume-level"; unit: "percent" }
          }
        }

        PanelSeparator { foreground: root.fg }

        // ---------- Footer: profiles and the full window ----------
        Row {
          width: parent.width
          spacing: Style.space(8)

          Dropdown {
            id: profileDrop
            visible: root.profiles.length > 0
            width: parent.width - fullButton.width - parent.spacing
            label: "Profile"
            showLabel: false
            value: ""
            options: root.profiles
            foreground: root.fg
            fontFamily: root.fontName
            onChanged: function(v) { root.loadProfile(v) }
          }
          Item {
            visible: !profileDrop.visible
            width: parent.width - fullButton.width - parent.spacing
            height: 1
          }
          Button {
            id: fullButton
            text: "Full window"
            fontSize: Style.font.bodySmall
            foreground: root.fg
            fontFamily: root.fontName
            bordered: true
            onClicked: {
              root.close()
              if (root.trayItem) root.trayItem.activate()
            }
          }
        }
      }
    }
  }

  // ---------- Rows ----------

  component Label: Text {
    textFormat: Text.PlainText
    color: root.fg
    font.family: root.fontName
    font.pixelSize: Style.font.bodySmall
    elide: Text.ElideRight
    verticalAlignment: Text.AlignVCenter
  }

  component SwitchRow: Item {
    id: sw
    property string label: ""
    property string group: ""
    property string name: ""
    property bool strong: false
    width: parent ? parent.width : 0
    implicitHeight: Math.max(swLabel.implicitHeight, swToggle.implicitHeight)

    Label {
      id: swLabel
      text: sw.label
      font.bold: sw.strong
      anchors.left: parent.left
      anchors.right: swToggle.left
      anchors.rightMargin: Style.space(8)
      anchors.verticalCenter: parent.verticalCenter
    }
    ToggleSwitch {
      id: swToggle
      anchors.right: parent.right
      anchors.verticalCenter: parent.verticalCenter
      checked: root.isOn(sw.group, sw.name)
      foreground: root.fg
      onToggled: root.toggle(sw.group, sw.name)
    }
  }

  component SliderRow: Item {
    id: sr
    property string label: ""
    property string group: ""
    property string name: ""
    property real minimum: 0
    property real maximum: 1
    property real step: 0.01
    property string unit: "percent"
    property bool showLabel: true
    width: parent ? parent.width : 0
    implicitHeight: Math.max(srLabel.implicitHeight, srSlider.implicitHeight)

    Label {
      id: srLabel
      text: sr.label
      opacity: 0.7
      width: sr.showLabel ? Style.space(92) : 0
      visible: sr.showLabel
      anchors.left: parent.left
      anchors.verticalCenter: parent.verticalCenter
    }
    PanelSlider {
      id: srSlider
      bar: root.bar
      anchors.left: srLabel.right
      anchors.right: srValue.left
      anchors.rightMargin: Style.space(8)
      anchors.verticalCenter: parent.verticalCenter
      minimum: sr.minimum
      maximum: sr.maximum
      step: sr.step
      value: root.num(sr.group, sr.name, sr.minimum)
      onReleased: function(v) { root.set(sr.group, sr.name, Model.format(sr.unit, v)) }
    }
    Label {
      id: srValue
      text: Model.readout(sr.unit, srSlider.dragging ? srSlider.liveValue : srSlider.value)
      width: Style.space(52)
      horizontalAlignment: Text.AlignRight
      anchors.right: parent.right
      anchors.verticalCenter: parent.verticalCenter
    }
  }

  // An SBX effect: its switch, and its level under it.
  component EffectRow: Column {
    id: er
    property string label: ""
    property string name: ""
    width: parent ? parent.width : 0
    spacing: Style.space(4)
    SwitchRow { label: er.label; group: "sbx"; name: er.name }
    SliderRow {
      group: "sbx"; name: er.name + "-level"; unit: "percent"; showLabel: false
      opacity: root.isOn("sbx", er.name) ? 1 : 0.45
    }
  }

  component ChoiceRow: Item {
    id: cr
    property string label: ""
    property string group: ""
    property string name: ""
    property var options: []
    width: parent ? parent.width : 0
    implicitHeight: Math.max(crLabel.implicitHeight, crGroup.implicitHeight)

    Label {
      id: crLabel
      text: cr.label
      opacity: 0.7
      width: Style.space(64)
      anchors.left: parent.left
      anchors.verticalCenter: parent.verticalCenter
    }
    ButtonGroup {
      id: crGroup
      anchors.left: crLabel.right
      anchors.right: parent.right
      anchors.verticalCenter: parent.verticalCenter
      options: cr.options
      value: root.value(cr.group, cr.name)
      foreground: root.fg
      fontFamily: root.fontName
      fontSize: Style.font.caption
      onChanged: function(v) { root.set(cr.group, cr.name, v) }
    }
  }

  // One equalizer band, a vertical slider in dB. Dragging shows the value;
  // the device is written on release.
  component Band: Item {
    id: band
    property string name: ""
    property string label: ""
    property real limit: 12
    property bool dragging: false
    property real live: 0
    readonly property real shown: dragging ? live : root.num("eq", name, 0)
    readonly property real track: height - bandValue.height - bandLabel.height - Style.space(12)

    function at(y) {
      var t = Math.max(0, Math.min(1, (y - bandTrack.y) / bandTrack.height))
      return Math.round((limit - t * 2 * limit) * 2) / 2
    }

    Label {
      id: bandValue
      text: Model.readout("db", band.shown)
      font.pixelSize: Style.font.caption
      width: parent.width
      horizontalAlignment: Text.AlignHCenter
      anchors.top: parent.top
    }
    Rectangle {
      id: bandTrack
      anchors.top: bandValue.bottom
      anchors.topMargin: Style.space(6)
      anchors.horizontalCenter: parent.horizontalCenter
      width: Math.max(4, Style.space(4))
      height: band.track
      radius: width / 2
      color: Style.selectedFillFor(root.fg, root.theme.accent)

      // The middle line, 0 dB.
      Rectangle {
        anchors.horizontalCenter: parent.horizontalCenter
        y: parent.height / 2
        width: Style.space(10)
        height: 1
        color: root.fg
        opacity: 0.3
      }
      Rectangle {
        readonly property real zero: parent.height / 2
        readonly property real pos: parent.height * (band.limit - band.shown) / (2 * band.limit)
        width: parent.width
        radius: parent.radius
        y: Math.min(zero, pos)
        height: Math.abs(zero - pos)
        color: root.fg
      }
      Rectangle {
        width: Style.space(12)
        height: width
        radius: width / 2
        color: root.fg
        anchors.horizontalCenter: parent.horizontalCenter
        y: parent.height * (band.limit - band.shown) / (2 * band.limit) - height / 2
      }
    }
    Label {
      id: bandLabel
      text: band.label
      opacity: 0.7
      font.pixelSize: Style.font.caption
      width: parent.width
      horizontalAlignment: Text.AlignHCenter
      anchors.bottom: parent.bottom
    }
    MouseArea {
      anchors.fill: parent
      cursorShape: Qt.PointingHandCursor
      preventStealing: true
      onPressed: function(m) { band.live = band.at(m.y); band.dragging = true }
      onPositionChanged: function(m) { if (band.dragging) band.live = band.at(m.y) }
      onReleased: function(m) {
        band.dragging = false
        root.set("eq", band.name, Model.format("db", band.at(m.y)))
      }
      onWheel: function(w) {
        var next = Math.max(-band.limit, Math.min(band.limit, root.num("eq", band.name, 0) + (w.angleDelta.y > 0 ? 0.5 : -0.5)))
        root.set("eq", band.name, Model.format("db", next))
      }
    }
  }
}
