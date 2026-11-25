# Component gallery renders (B13)

`scripts/gui/gallery-check.sh` output for `junc-gui --scenario gallery`,
in a private headless session (Xvfb, software renderer), compared by eye
with section 2 of the Spec board (`../51-Spec.png`):

- `gallery-tokyo-night.png`, `gallery-flexoki-light.png`: every component
  in its states, in both built-in themes.
- `gallery-focus.png` and `gallery-focus.txt`: after five Tabs the first
  switch has the focus and its accent ring; AT-SPI reports
  `toggle button name='Zoom Mic' desc='on'`.
- `gallery-atspi.txt`: the whole AT-SPI tree (labels, descriptions, slider
  values, list items, radio buttons).

Regenerate with the script after a component changes, in its own commit.
Screenshot goldens checked in CI come with B14.
