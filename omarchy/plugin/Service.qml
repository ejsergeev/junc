// SPDX-License-Identifier: GPL-3.0-or-later
pragma ComponentBehavior: Bound
import QtQuick
import Quickshell
import Quickshell.Io
import qs.Commons
import "lib/Model.js" as Model

// The plugin's state, one per shell and shared by the bar on every monitor:
// it follows `junc --json watch` (the Junctions, which are on, which apps are
// being captured) and runs `junc enable|disable` for the popup's switches.
// The watch restarts after a pause when it ends (juncd or junc updated, junc
// not installed yet), so the module never needs a shell restart.
Item {
  id: root

  property var shell: null   // set by the host

  readonly property Translations strings: Translations { directory: Qt.resolvedUrl("i18n") }

  property var state: Model.empty()
  // id -> the state a switch was flipped to, until the stream agrees.
  property var pending: ({})
  // junc answered at least once: without it the bar shows nothing.
  property bool available: false
  // The shortcut bound to junc-gui, if the user bound one.
  property string shortcut: ""

  readonly property int onCount: Model.onCount(state)
  readonly property bool capturing: Model.capturing(state)
  readonly property var rows: Model.rows(state, pending)

  function setEnabled(id, on) {
    var next = Object.assign({}, pending)
    next[id] = on
    pending = next
    settleTimer.restart()
    Quickshell.execDetached(Model.switchCommand(id, on))
  }

  function openApp() {
    Quickshell.execDetached(["uwsm-app", "--", "junc-gui"])
  }

  function refreshShortcut() {
    if (!bindsProcess.running) bindsProcess.running = true
  }

  onStateChanged: pending = Model.settle(state, pending)

  Process {
    id: watchProcess
    command: ["junc", "--json", "watch"]
    running: true
    stdout: SplitParser {
      onRead: function(line) {
        root.available = true
        root.state = Model.apply(root.state, line)
      }
    }
    onExited: function() {
      root.state = Model.apply(root.state, '{"event":"daemon","running":false}')
      restartTimer.restart()
    }
  }

  // A watch that ends is started again; slower while junc is missing.
  Timer {
    id: restartTimer
    interval: root.available ? 2000 : 30000
    onTriggered: watchProcess.running = true
  }

  // A switch juncd never confirmed (it failed or juncd is down) goes back.
  Timer {
    id: settleTimer
    interval: 5000
    onTriggered: root.pending = ({})
  }

  Process {
    id: bindsProcess
    command: ["hyprctl", "binds", "-j"]
    stdout: StdioCollector {
      waitForEnd: true
      onStreamFinished: {
        try {
          root.shortcut = Model.shortcut(JSON.parse(text.length > 1048576 ? "[]" : text))
        } catch (e) {
          root.shortcut = ""
        }
      }
    }
  }

  Component.onCompleted: refreshShortcut()
}
