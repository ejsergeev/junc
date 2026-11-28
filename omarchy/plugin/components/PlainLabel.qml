// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick

// Text that is never read as markup: names come from the user's config and
// app names from PipeWire, so they are shown exactly as they are.
Text {
  textFormat: Text.PlainText
  elide: Text.ElideRight
  renderType: Text.NativeRendering
}
