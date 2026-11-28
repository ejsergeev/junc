// SPDX-License-Identifier: GPL-3.0-or-later
pragma ComponentBehavior: Bound
import QtQuick
import qs.Commons
import qs.Commons as Commons
import qs.Ui
import "components"

// The bar module (design board 44, F21/F23): the JUNC mark with the number
// of Junctions on, and a small urgent dot while an app's audio is being
// captured. Left click opens the popup with a switch per Junction; right
// click opens JUNC. Everything it shows lives in Service.qml, shared by the
// bar on every monitor.
BarWidget {
  id: root

  readonly property string pluginId: "io.github.ejsergeev.junc"
  moduleName: pluginId

  // The host types `bar` as a plain QtObject; its API has shell.
  readonly property var service: bar?.shell?.serviceFor(pluginId) ?? null // qmllint disable missing-property
  readonly property bool shown: !!service && service.available
  readonly property int onCount: service ? service.onCount : 0

  // Bar panel contract: `omarchy-shell shell toggle <id>` opens it.
  property bool popupOpen: false
  readonly property bool opened: popupOpen
  function open() { if (shown) { popupOpen = true; service.refreshShortcut() } }
  function close() { popupOpen = false }

  visible: shown
  implicitWidth: shown ? button.implicitWidth : 0
  implicitHeight: barSize

  BarIconButton {
    id: button
    anchors.fill: parent
    bar: root.bar
    iconComponent: Component {
      // The capture dot (F23) rides the mark's top right corner, inside
      // its bounds, so the bar sizes the icon the same with and without it.
      JuncMark {
        size: 15
        color: button.foreground
        dot: root.service ? root.service.capturing : false
        dotColor: Commons.Color.urgent
      }
    }
    labelText: root.onCount > 0 && !root.vertical ? Commons.I18n.formatNumber(root.onCount) : ""
    slotSize: Style.bar.iconSlot * (labelText !== "" ? 1.8 : 1)
    tooltipText: root.service && root.service.capturing
      ? root.service.strings.t("popup.capturing").arg(root.service.state.capture.join(", "))
      : "JUNC"
    onPressed: function(mouseButton) {
      if (!root.service) return
      if (mouseButton === Qt.RightButton) root.service.openApp()
      else if (root.popupOpen) root.close()
      else root.open()
    }
  }

  KeyboardPanel {
    id: panel
    anchorItem: root
    bar: root.bar
    owner: root
    open: root.popupOpen
    focusTarget: (panel.loadedContent as Popup)?.focusTarget ?? null
    contentWidth: panel.fittedContentWidth(Style.space(300))
    contentHeight: panel.fittedContentHeight(panel.loadedContentHeight)

    content: Popup {
      service: root.service
      bar: root.bar
      onCloseRequested: root.close()
    }
  }
}
