// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick

// The JUNC mark (design boards 8, 44, 50): a square with a signal stepping
// down through it, the same shapes as the GUI's JLogo. Plain rectangles, so
// it is sharp at bar size and follows the theme's colour.
Item {
  id: root

  property real size: 16
  property color color: "white"
  // A small square at the top right corner, for "an app is being captured".
  property bool dot: false
  property color dotColor: "red"
  readonly property real line: Math.max(1, Math.round(size / 11))

  implicitWidth: size
  implicitHeight: size

  Rectangle {
    anchors.fill: parent
    color: "transparent"
    border.width: root.line
    border.color: root.color
  }
  Rectangle { x: Math.round(root.size * 0.22); y: Math.round(root.size * 0.25); width: Math.round(root.size * 0.3); height: root.line; color: root.color }
  Rectangle { x: Math.round(root.size * 0.46); y: Math.round(root.size * 0.25); width: root.line; height: Math.round(root.size * 0.48); color: root.color }
  Rectangle { x: Math.round(root.size * 0.46); y: Math.round(root.size * 0.68); width: Math.round(root.size * 0.32); height: root.line; color: root.color }
  Rectangle { x: Math.round(root.size * 0.18); y: Math.round(root.size * 0.2); width: Math.max(2, Math.round(root.size * 0.14)); height: width; color: root.color }
  Rectangle { x: Math.round(root.size * 0.66); y: Math.round(root.size * 0.63); width: Math.max(2, Math.round(root.size * 0.14)); height: width; color: root.color }
  Rectangle {
    visible: root.dot
    width: Math.max(4, Math.round(root.size * 0.4))
    height: width
    x: root.size - width + Math.round(width / 3)
    y: -Math.round(width / 3)
    color: root.dotColor
  }
}
