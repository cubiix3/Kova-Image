# Kova Image interface

The image is the primary surface. Chrome provides orientation and a few precise
actions without resembling an editor or a media library.

## Design system

`ui/theme.slint` owns the palette and shared metrics. It retains Kova File's
cyan identity and Segoe UI typography while using darker, more neutral surfaces
appropriate for viewing images.

| Role | Value |
| --- | --- |
| Image canvas | `#101214` |
| Header, control bar, popovers | `#1b1e22` |
| Control groups | `#24282d` |
| Hover / pressed | `#30363d` / `#3a434c` |
| Primary / secondary text | `#f0f2f5` / `#b2bac4` |
| Supporting text | `#8d98a5` |
| Selected surface / Kova accent | `#253e4b` / `#86d5f4` |
| Subtle divider | `#2b3036` |

The 56 px header balances the existing Kova mark, filename, format and dimensions.
The 72 px bottom surface contains 36 px controls in 40 px groups: navigation,
zoom, fit mode, transforms/playback, and fullscreen/more. Below 820 px the Open
label and spacing become compact; controls retain their hit areas. The minimum
window remains 640 × 420. Popovers scroll within short windows.

The shared `Tool`, `ToggleRow`, `Group`, `Rule` and `Divider` components in
`ui/controls.slint` provide consistent enabled, disabled, hover, pressed, active
and keyboard-focus states. Original utility icons in `assets/icons/` use one
24-unit grid and stroke weight; `ui/icons.slint` exposes them to the UI.
No icon font, network asset, blur, acrylic or shadow dependency is required.

Normal labels are 13 px. File details and version text use 11–12 px; 10 px
uppercase section labels are supplemental, never the sole label of a control.
Accent marks selection and keyboard focus. Red is reserved for destructive
actions and errors. Small images retain their natural size in Fit mode.

## Interaction

- Navigation shows folder position and disables directions at folder boundaries.
- Fit and 100% indicate the selected view mode; the zoom value remains explicit.
- Icon actions expose descriptive accessibility labels and on-screen hints.
- Tab traverses controls; Space/Enter activate a focused button. Clicking the
  image or dismissing a popup restores focus to the viewer. Space then controls
  animation playback.
- Escape dismisses the current popup before leaving fullscreen. Clicking outside
  More dismisses it. Settings and information share the same panel structure.
- Fullscreen uses a compact floating control surface. After two seconds of
  inactivity, header and controls fade over 120 ms. Hovered controls, open
  popovers and keyboard-focused controls remain available.
- Empty, loading and error states have distinct, concise text. Decode failures
  retain folder navigation. Successful action notices expire after five seconds.
- Image information uses label/value rows, with wrapping paths. View transforms
  continue to leave original files unchanged.

Focus behavior uses Slint's
[FocusScope events](https://docs.slint.dev/latest/docs/slint/reference/keyboard-input/focusscope/)
and explicit ownership so destroying a popup cannot leave a stale focus count.

## Performance constraints

The renderer, decoder pipeline, worker count, cache budgets and format support
are unchanged. Only the two small chrome surfaces animate opacity, and only
during show/hide transitions. No animation is attached to the image or canvas.
Metadata rows are updated on load/information requests, never per animation frame.
No new Cargo dependencies were added.

During animated images, the renderer may cache the header and bottom bar as two
small surfaces, so unchanged icons and text do not require individual draw calls
on every frame. This hint is disabled for still images. It trades bounded chrome
texture memory for less CPU work; the image itself is never cached in this layer.
See Slint's [rendering-cache guidance](https://docs.slint.dev/latest/docs/slint/reference/common/#cache-rendering-hint).

The viewport now has a 16 px inset in windowed mode. Cursor-anchored zoom accounts
for that offset; fullscreen uses the full viewport. Geometry is still calculated
by the existing Rust view model.

## Possible video presentation

Video playback is **not implemented**. This is a visual extension point, not a
format-support claim or a commitment to expand the current viewer's scope.

If video is approved later, the existing control surface can gain a slim timeline
row above it, with elapsed/total time and the same focus and active colors. The
existing playback position would host Play/Pause; a compact mute/volume control
would appear only for media with audio. Header, fullscreen behavior, loading and
error presentation would remain shared. Images must never show a seekbar, volume
controls or disabled video placeholders. Seek updates should be input-driven;
text time updates should not redraw on every video frame.

## Remaining visual validation

- Physical multi-monitor 125–200% DPI transitions, touchpads and screen readers.
- Windows high-contrast/reduced-motion preferences are not yet integrated into
  the custom theme; the design does not claim full accessibility conformance.
- Native file picker and Explorer/Open With surfaces retain Windows styling.
- Any video UI requires an independently reviewed playback architecture first.

Local renderer captures and interaction checks are described in
[PERFORMANCE.md](PERFORMANCE.md); generated captures stay in `artifacts/`.

![Kova Image empty state](images/empty.png)

Actual before/after measurements, including memory costs, are recorded in
[UI_MEASUREMENTS.md](UI_MEASUREMENTS.md).
