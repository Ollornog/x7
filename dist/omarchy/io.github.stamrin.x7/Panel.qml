import QtQuick
import QtQuick.Effects
import Quickshell
import Quickshell.Services.SystemTray
import qs.Commons
// A bare `Color` is QtQuick's own value type on Qt 6.12, not the theme.
import qs.Commons as Commons
import qs.Ui

// The x7-gui tray item (StatusNotifierItem id "x7") drawn on its own in the
// bar, so it can sit beside the clock instead of inside the tray drawer. The
// installer hides the same item in omarchy.tray so it does not show twice.
// Hidden while x7-gui is not running. Left click opens the controls window,
// right click the tray menu, middle click the secondary action.
BarWidget {
  id: root
  moduleName: "io.github.stamrin.x7"

  readonly property var theme: Commons.ShellColor || Commons.Color
  readonly property var trayItem: {
    var values = SystemTray.items.values
    for (var i = 0; i < values.length; i++) {
      if (String(values[i].id || "") === "x7") return values[i]
    }
    return null
  }
  readonly property color foreground: bar ? bar.foreground : theme.foreground
  readonly property string iconSource: trayItem ? String(trayItem.icon || "") : ""
  readonly property bool symbolic: iconSource.split("?")[0].slice(-9) === "-symbolic"

  // The tooltip is the app's own text, but the shared tooltip renders rich
  // text: keep markup out of it.
  function plain(value) {
    return String(value || "").replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;")
  }

  visible: trayItem !== null
  implicitWidth: visible ? Style.bar.iconSlot : 0
  implicitHeight: visible ? (vertical ? Style.bar.iconSlot : barSize) : 0

  Image {
    id: icon
    anchors.centerIn: parent
    width: Style.bar.iconSize
    height: Style.bar.iconSize
    fillMode: Image.PreserveAspectFit
    // Decode at physical pixels, as the tray does, so HiDPI stays sharp.
    sourceSize.width: Math.round(width * Screen.devicePixelRatio)
    sourceSize.height: Math.round(height * Screen.devicePixelRatio)
    source: root.iconSource
    visible: !root.symbolic
    layer.enabled: root.symbolic
  }

  // Symbolic icons take the bar's foreground, as in the tray.
  Rectangle {
    anchors.fill: icon
    color: root.foreground
    visible: root.symbolic
    layer.enabled: root.symbolic
    layer.effect: MultiEffect {
      maskEnabled: true
      maskSource: icon
      maskThresholdMin: 0.5
      maskSpreadAtMin: 1.0
    }
  }

  MouseArea {
    anchors.fill: parent
    acceptedButtons: Qt.LeftButton | Qt.RightButton | Qt.MiddleButton
    hoverEnabled: true
    cursorShape: Qt.PointingHandCursor
    onEntered: if (root.bar && root.trayItem) root.bar.showTooltip(root, root.plain(root.trayItem.tooltipTitle || root.trayItem.title || "Sound Blaster X7"))
    onExited: if (root.bar) root.bar.hideTooltip(root)
    onClicked: function(mouse) {
      var item = root.trayItem
      if (!item) return
      if (mouse.button === Qt.RightButton || item.onlyMenu) {
        var point = root.QsWindow.contentItem.mapFromItem(root, mouse.x, mouse.y)
        item.display(root.QsWindow.window, point.x, point.y)
      } else if (mouse.button === Qt.MiddleButton) {
        item.secondaryActivate()
      } else {
        item.activate()
      }
    }
    onWheel: function(wheel) {
      if (root.trayItem) root.trayItem.scroll(wheel.angleDelta.y, false)
    }
  }
}
