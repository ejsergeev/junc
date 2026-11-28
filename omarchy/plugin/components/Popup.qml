// SPDX-License-Identifier: GPL-3.0-or-later
pragma ComponentBehavior: Bound
import QtQuick
import qs.Commons
import qs.Commons as Commons
import qs.Ui

// The popup (design board 44): JUNC and how many are on, the apps being
// captured, a switch per Junction and Cable, and Open JUNC. Up and Down move
// the cursor, Space or Enter acts on it, Escape closes.
Item {
  id: root

  required property var service
  required property var bar
  signal closeRequested()

  readonly property Item focusTarget: keys
  readonly property var strings: service ? service.strings : null
  readonly property var rows: service ? service.rows : []
  readonly property bool running: !!service && service.state.running
  // The cursor: a row index, or rows.length for Open JUNC.
  property int cursor: -1

  readonly property color foreground: bar ? bar.foreground : Commons.Color.foreground
  readonly property color muted: Qt.alpha(foreground, 0.6)
  readonly property string fontFamily: bar ? bar.fontFamily : Style.font.family

  function t(key) { return strings ? strings.t(key) : key }

  function subtitle(row) {
    if (row.kind === "cable") return t("row.cable")
    var inputs = strings ? strings.tn("row.junction", row.inputs) : "%1"
    return inputs.arg(Commons.I18n.formatNumber(row.inputs))
  }

  function activate(index) {
    if (!service) return
    if (index >= 0 && index < rows.length) service.setEnabled(rows[index].id, !rows[index].enabled)
    else if (index === rows.length) {
      service.openApp()
      root.closeRequested()
    }
  }

  implicitWidth: column.implicitWidth
  implicitHeight: column.implicitHeight

  PanelKeyCatcher {
    id: keys
    anchors.fill: parent
    focus: true
    onMoveRequested: function(dx, dy) {
      if (dy === 0) return
      var last = root.rows.length
      root.cursor = root.cursor < 0 ? (dy > 0 ? 0 : last) : Math.max(0, Math.min(last, root.cursor + dy))
    }
    onActivateRequested: root.activate(root.cursor)
    onCloseRequested: root.closeRequested()

    Column {
      id: column
      width: parent.width
      spacing: Style.spacing.md

      // JUNC · 2 on
      Row {
        spacing: Style.spacing.lg
        JuncMark {
          size: Style.space(16)
          color: Commons.Color.accent
          anchors.verticalCenter: parent.verticalCenter
        }
        PlainLabel {
          text: "JUNC"
          color: root.foreground
          font.family: root.fontFamily
          font.pixelSize: Style.font.subtitle
          anchors.verticalCenter: parent.verticalCenter
        }
        PlainLabel {
          visible: root.running
          text: root.t("popup.on").arg(Commons.I18n.formatNumber(root.service ? root.service.onCount : 0))
          color: root.muted
          font.family: root.fontFamily
          font.pixelSize: Style.font.caption
          anchors.verticalCenter: parent.verticalCenter
        }
      }

      // Capturing: Spotify, Firefox (F23)
      Row {
        visible: !!root.service && root.service.capturing
        width: parent.width
        spacing: Style.spacing.md
        Rectangle {
          width: Style.space(6)
          height: width
          color: Commons.Color.urgent
          anchors.verticalCenter: parent.verticalCenter
        }
        PlainLabel {
          width: parent.width - Style.space(12)
          text: root.t("popup.capturing").arg(root.service ? root.service.state.capture.join(", ") : "")
          color: root.foreground
          font.family: root.fontFamily
          font.pixelSize: Style.font.caption
          elide: Text.ElideRight
        }
      }

      PlainLabel {
        visible: !root.running || root.rows.length === 0
        width: parent.width
        text: root.running ? root.t("popup.empty") : root.t("popup.notRunning")
        color: root.muted
        font.family: root.fontFamily
        font.pixelSize: Style.font.body
        wrapMode: Text.Wrap
      }

      Repeater {
        model: root.running ? root.rows : []
        delegate: JunctionRow {
          required property var modelData
          required property int index
          width: column.width
          row: modelData
          subtitle: root.subtitle(modelData)
          strings: root.strings
          foreground: root.foreground
          muted: root.muted
          fontFamily: root.fontFamily
          hasCursor: root.cursor === index
          onToggled: root.activate(index)
        }
      }

      PanelSeparator { width: parent.width; foreground: root.foreground }

      // Open JUNC            [Super+Shift+J]
      Item {
        id: openRow
        width: parent.width
        height: Math.max(openLabel.implicitHeight, keyHint.height) + Style.spacing.md * 2
        readonly property bool hot: openMouse.containsMouse || root.cursor === root.rows.length

        Rectangle {
          anchors.fill: parent
          color: openRow.hot ? Style.hoverFill : "transparent"
        }
        PlainLabel {
          id: openLabel
          x: Style.spacing.md
          anchors.verticalCenter: parent.verticalCenter
          text: root.t("popup.open")
          color: root.foreground
          font.family: root.fontFamily
          font.pixelSize: Style.font.body
        }
        Rectangle {
          id: keyHint
          visible: !!root.service && root.service.shortcut !== ""
          anchors.right: parent.right
          anchors.rightMargin: Style.spacing.md
          anchors.verticalCenter: parent.verticalCenter
          width: keyText.implicitWidth + Style.spacing.md * 2
          height: keyText.implicitHeight + Style.spacing.xs * 2
          color: "transparent"
          border.width: 1
          border.color: root.muted
          PlainLabel {
            id: keyText
            anchors.centerIn: parent
            text: root.service ? root.service.shortcut : ""
            color: root.muted
            font.family: root.fontFamily
            font.pixelSize: Style.font.caption
          }
        }
        MouseArea {
          id: openMouse
          anchors.fill: parent
          hoverEnabled: true
          cursorShape: Qt.PointingHandCursor
          onClicked: root.activate(root.rows.length)
        }
      }
    }
  }
}
