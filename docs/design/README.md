# Design reference renders

PNG renders of every board of the design canvas "JUNC for Omarchy"
(design canvas artifact `FLgCesoEBfmAa1N1uDwmJf`, version 1795592639-9476),
numbered in canvas order. They are the visual spec for the Slint GUI
(`docs/architecture.md` §7.5 maps each board to a scenario). The canvas
stays the source of truth; these files are for sessions that cannot open it.

How they were made (regenerate when the canvas changes, in its own commit):

1. Download the artifact's `project/*.dc.html` and `artifact-type/dc-runtime.js`
   into an empty directory; copy `dc-runtime.js` to `project/support.js`.
2. Serve `project/` with a static server on `127.0.0.1`.
3. For each board in `project/canvas.json` order, at its `w`×`h`:
   `chromium --headless=new --disable-gpu --hide-scrollbars --window-size=W,H
   --force-device-scale-factor=1 --virtual-time-budget=6000 --screenshot=NN-Name.png URL`.

Meters are animated in the design, so meter widths differ between renders.
`32-StateArabic.png` is a post-v1 reference (RTL is deferred).
