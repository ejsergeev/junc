// SPDX-License-Identifier: GPL-3.0-or-later
pragma ComponentBehavior: Bound
import QtQuick
import qs.Commons
import qs.Commons as Commons
import qs.Ui

// One Junction or Cable in the popup: its name, a "Default input" badge
// when it is one, what it is (Junction · 3 Inputs, Cable) and its switch.
// The whole row flips the switch.
Item {
  id: root

  required property var row
  required property string subtitle
  required property var strings
  property color foreground: Commons.Color.foreground
  property color muted: Qt.alpha(foreground, 0.6)
  property string fontFamily: Style.font.family
  property bool hasCursor: false
  signal toggled()

  readonly property bool hot: hasCursor || mouse.containsMouse
  readonly property string badge: row.default === "input" ? t("row.defaultInput")
    : row.default === "output" ? t("row.defaultOutput") : ""

  function t(key) { return strings ? strings.t(key) : key }

  implicitHeight: texts.implicitHeight + Style.spacing.md * 2

  Rectangle {
    anchors.fill: parent
    color: root.hot ? Style.hoverFill : "transparent"
  }

  Column {
    id: texts
    x: Style.spacing.md
    width: parent.width - toggle.width - Style.spacing.md * 3
    anchors.verticalCenter: parent.verticalCenter
    spacing: Style.spacing.xs

    PlainLabel {
      width: parent.width
      text: root.row.name
      color: root.foreground
      font.family: root.fontFamily
      font.pixelSize: Style.font.body
    }
    Row {
      spacing: Style.spacing.md
      Rectangle {
        id: badgeBox
        visible: root.badge !== ""
        width: badgeText.implicitWidth + Style.spacing.md * 2
        height: badgeText.implicitHeight + Style.spacing.xxs * 2
        color: "transparent"
        border.width: 1
        border.color: Commons.Color.accent
        PlainLabel {
          id: badgeText
          anchors.centerIn: parent
          text: root.badge
          color: Commons.Color.accent
          font.family: root.fontFamily
          font.pixelSize: Style.font.caption
        }
      }
      PlainLabel {
        // Long words (German) are cut before they reach the switch.
        width: Math.min(implicitWidth, texts.width - (badgeBox.visible ? badgeBox.width + Style.spacing.md : 0))
        anchors.verticalCenter: parent.verticalCenter
        text: root.subtitle
        color: root.muted
        font.family: root.fontFamily
        font.pixelSize: Style.font.caption
      }
    }
  }

  ToggleSwitch {
    id: toggle
    anchors.right: parent.right
    anchors.rightMargin: Style.spacing.md
    anchors.verticalCenter: parent.verticalCenter
    checked: root.row.enabled
    busy: root.row.busy
    interactive: false
    foreground: root.foreground
  }

  MouseArea {
    id: mouse
    anchors.fill: parent
    hoverEnabled: true
    cursorShape: Qt.PointingHandCursor
    onClicked: root.toggled()
  }
}
